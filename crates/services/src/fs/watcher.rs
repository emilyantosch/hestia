use anyhow::{Context, Result, ensure};
use events::{FileEvent, FolderEvent};
use hash::{ContentDigest, FilesystemObjectId};
use itertools::Itertools;
use model::services::{CanonPath, IndexedPath};
use notify::event::{CreateKind, EventKind, RemoveKind};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache, new_debouncer,
};
use repositories::fs::operations::FileRepository;
use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum FSEvent {
    File(FileEvent),
    Folder(FolderEvent),
}

#[derive(Debug)]
pub enum FileWatcherMessage {
    WatchPath(CanonPath),
    UnwatchPath(CanonPath),
    GetWatchPaths(oneshot::Sender<HashSet<CanonPath>>),
}

pub struct FileWatcher {
    watcher: Option<Debouncer<RecommendedWatcher, RecommendedCache>>,
    pub message_receiver: mpsc::UnboundedReceiver<FileWatcherMessage>,
    watched_paths: HashSet<CanonPath>,
}

impl fmt::Debug for FileWatcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileWatcher")
            .field("watcher_initialized", &self.watcher.is_some())
            .field("watched_paths", &self.watched_paths)
            .finish_non_exhaustive()
    }
}

impl FileWatcher {
    #[must_use]
    pub fn new(message_receiver: mpsc::UnboundedReceiver<FileWatcherMessage>) -> FileWatcher {
        Self {
            watcher: None,
            message_receiver,
            watched_paths: HashSet::new(),
        }
    }

    /// Start the debouncer; observed changes are sent to `events`.
    pub fn init_watcher(&mut self, events: Sender<FSEvent>) -> Result<()> {
        let (results_sender, mut results) = mpsc::channel(100);
        let runtime = tokio::runtime::Handle::current();

        let debouncer = new_debouncer(
            Duration::from_secs(2),
            None,
            move |result: DebounceEventResult| {
                let results_sender = results_sender.clone();
                runtime.spawn(async move {
                    if let Err(error) = results_sender.send(result).await {
                        tracing::warn!("Error sending event result: {error:?}");
                    }
                });
            },
        );

        tokio::spawn(async move {
            while let Some(result) = results.recv().await {
                let debounced_events = match result {
                    Ok(debounced_events) => debounced_events,
                    Err(errors) => {
                        tracing::warn!("Filesystem watcher reported errors: {errors:?}");
                        continue;
                    }
                };
                for event in debounced_events {
                    match observe(event).await {
                        Ok(Some(event)) => {
                            if let Err(error) = events.send(event).await {
                                tracing::warn!("Error sending processed event: {error:#?}");
                            }
                        }
                        Ok(None) => (),
                        Err(error) => tracing::warn!("Failed to process event: {error:#}"),
                    }
                }
            }
        });

        self.watcher = Some(debouncer.context("failed to initialize filesystem watcher")?);
        tracing::info!("File watcher started");
        Ok(())
    }

    pub async fn run(mut self, events: Sender<FSEvent>) -> Result<()> {
        self.init_watcher(events)?;
        while let Some(message) = self.message_receiver.recv().await {
            let watcher = self
                .watcher
                .as_mut()
                .context("watcher is not initialized")?;
            match message {
                FileWatcherMessage::WatchPath(path) => {
                    if !self.watched_paths.contains(&path) {
                        watcher.watch(&path, RecursiveMode::Recursive)?;
                        self.watched_paths.insert(path);
                    }
                }
                FileWatcherMessage::UnwatchPath(path) => {
                    if self.watched_paths.remove(&path) {
                        watcher.unwatch(&path)?;
                    }
                }
                FileWatcherMessage::GetWatchPaths(sender) => {
                    drop(sender.send(self.watched_paths.clone()));
                }
            }
        }
        Ok(())
    }
}

// ponytail: temporary direct-write adapter; remove when scanner owns reconciliation
/// Apply a watcher event to the database.
pub async fn store_event(event: FSEvent, repository: &FileRepository) -> Result<()> {
    match event {
        FSEvent::File(event) => match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                let file = repository.upsert_file_from_event(&event).await?;
                tracing::info!("Stored file: {} (ID: {})", file.path, file.id);
            }
            EventKind::Remove(_) => {
                for path in &event.paths {
                    let deleted = repository.delete_file_by_path(path).await?;
                    tracing::info!(deleted, "Removed file from database: {}", path.display());
                }
            }
            _ => (),
        },
        FSEvent::Folder(event) => match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                let folder = repository.upsert_folder_from_event(&event).await?;
                tracing::info!("Stored folder: {} (ID: {})", folder.path, folder.id);
            }
            EventKind::Remove(_) => {
                for path in &event.paths {
                    let deleted = repository.delete_folder_by_path(path).await?;
                    tracing::info!(deleted, "Removed folder from database: {}", path.display());
                }
            }
            _ => (),
        },
    }
    Ok(())
}

/// Classify a debounced event and observe the entry it points at. Access events are ignored.
async fn observe(mut event: DebouncedEvent) -> Result<Option<FSEvent>> {
    event.paths = event
        .paths
        .iter()
        .map(|path| IndexedPath::try_from(path.as_path()).map(PathBuf::from))
        .try_collect()?;
    let kind = event.kind;
    let paths = event.paths.clone();
    let path = paths
        .last()
        .context("watcher event does not contain a path")?;

    let is_folder = path.is_dir()
        || matches!(
            kind,
            EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder)
        );
    if is_folder {
        let filesystem_object_id = if kind == EventKind::Remove(RemoveKind::Folder) {
            None
        } else {
            Some(FilesystemObjectId::observe(path).await?)
        };
        return Ok(Some(FSEvent::Folder(FolderEvent {
            event,
            kind,
            paths,
            filesystem_object_id,
        })));
    }
    if matches!(kind, EventKind::Access(_)) {
        return Ok(None);
    }

    let (content_digest, filesystem_object_id) = if matches!(kind, EventKind::Remove(_)) {
        (None, None)
    } else {
        let before = FilesystemObjectId::observe(path).await?;
        let content_digest = ContentDigest::observe(path).await?;
        let filesystem_object_id = FilesystemObjectId::observe(path).await?;
        ensure!(
            before == filesystem_object_id,
            "file was replaced while the watcher observed it"
        );
        (Some(content_digest), Some(filesystem_object_id))
    };
    Ok(Some(FSEvent::File(FileEvent {
        event,
        kind,
        paths,
        content_digest,
        filesystem_object_id,
    })))
}
