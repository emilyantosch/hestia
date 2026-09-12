use anyhow::{Context, Result, bail, ensure};
use events::{FileEvent, FolderEvent};
use hash::{ContentDigest, FilesystemObjectId};
use model::services::CanonPath;
use notify::event::{CreateKind, EventKind, RemoveKind};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache, new_debouncer,
};
use repositories::fs::operations::FileRepository as FileOperations;
use std::collections::HashSet;
use std::fmt;
#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;
#[cfg(test)]
use tokio::sync::Mutex;
use tokio::sync::mpsc::{self, Sender, UnboundedSender};
use tokio::sync::oneshot;

#[derive(Debug)]
pub struct FSEvent {
    pub file_event: Option<FileEvent>,
    pub folder_event: Option<FolderEvent>,
}

impl From<FileEvent> for FSEvent {
    fn from(file_event: FileEvent) -> Self {
        FSEvent {
            file_event: Some(file_event),
            folder_event: None,
        }
    }
}

impl From<FolderEvent> for FSEvent {
    fn from(folder_event: FolderEvent) -> Self {
        FSEvent {
            file_event: None,
            folder_event: Some(folder_event),
        }
    }
}

#[async_trait::async_trait]
pub trait FileWatcherEventHandler: Send + Sync {
    async fn handle_event(&self, event: FSEvent) -> Result<()>;
}

#[derive(Debug)]
pub struct DatabaseFileWatcherEventHandler {
    pub db_operations: FileOperations,
    pub changes: Option<UnboundedSender<()>>,
}

#[async_trait::async_trait]
impl FileWatcherEventHandler for DatabaseFileWatcherEventHandler {
    async fn handle_event(&self, event: FSEvent) -> Result<()> {
        FileWatcher::to_database(event, &self.db_operations).await?;
        if let Some(changes) = &self.changes {
            let _send_result = changes.send(());
        }
        Ok(())
    }
}

#[cfg(test)]
#[derive(Debug)]
pub struct TestFileWatcherEventHandler {
    pub sender: Arc<Mutex<UnboundedSender<FSEvent>>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl FileWatcherEventHandler for TestFileWatcherEventHandler {
    async fn handle_event(&self, event: FSEvent) -> Result<()> {
        tracing::info!("FileWatcher is sending event to test pipeline");
        let sender = self.sender.lock().await;
        sender.send(event)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct FileWatcherHandler {
    pub sender: UnboundedSender<FileWatcherMessage>,
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
    pub fn init_watcher(&mut self, event_handler: Box<dyn FileWatcherEventHandler>) -> Result<()> {
        let (r_tx, mut r_rx) = mpsc::channel(100);
        let rt = tokio::runtime::Handle::current();
        let (p_tx, mut p_rx) = mpsc::channel::<FSEvent>(100);

        let debouncer = new_debouncer(
            Duration::from_secs(2),
            None,
            move |result: DebounceEventResult| {
                let r_tx_clone = r_tx.clone();
                rt.spawn(async move {
                    if let Err(e) = r_tx_clone.send(result).await {
                        tracing::info!("Error sending event result: {:?}", e);
                    }
                });
            },
        );

        tokio::spawn(async move {
            while let Some(res) = r_rx.recv().await {
                match res {
                    Ok(events) => {
                        for event in events {
                            if let Err(e) = to_file_or_folder_event_and_send(event, &p_tx).await {
                                tracing::error!("Failed to process event: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("errors: {:?}", e);
                    }
                }
            }
        });

        tokio::spawn(async move {
            while let Some(event) = p_rx.recv().await {
                if let Err(e) = event_handler.handle_event(event).await {
                    tracing::error!("Failed to store event to database: {:?}", e);
                }
            }
        });

        self.watcher = Some(debouncer.context("failed to initialize filesystem watcher")?);
        tracing::info!("Init of FileWatcher completed successfully!");
        Ok(())
    }

    #[must_use]
    pub fn new(message_receiver: mpsc::UnboundedReceiver<FileWatcherMessage>) -> FileWatcher {
        Self {
            watcher: None,
            message_receiver,
            watched_paths: HashSet::new(),
        }
    }

    pub async fn run(mut self, event_handler: Box<dyn FileWatcherEventHandler>) -> Result<()> {
        self.init_watcher(event_handler)?;
        while let Some(message) = self.message_receiver.recv().await {
            match message {
                FileWatcherMessage::WatchPath(path) => {
                    if !self.watched_paths.contains(&path) {
                        self.watcher
                            .as_mut()
                            .context("watcher is not initialized")?
                            .watch(&path, RecursiveMode::Recursive)?;
                        self.watched_paths.insert(path);
                    }
                }
                FileWatcherMessage::UnwatchPath(path) => {
                    if self.watched_paths.contains(&path) {
                        self.watcher
                            .as_mut()
                            .context("watcher is not initialized")?
                            .unwatch(&path)?;
                        self.watched_paths.remove(&path);
                    }
                }
                FileWatcherMessage::GetWatchPaths(sender) => {
                    drop(sender.send(self.watched_paths.clone()));
                }
            }
        }
        Ok(())
    }

    // ponytail: temporary direct-write adapter; remove when scanner owns reconciliation
    async fn to_database(event: FSEvent, db_operations: &FileOperations) -> Result<()> {
        if let Some(file_event) = event.file_event {
            match file_event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    // File was created or modified, insert/update in database
                    match db_operations.upsert_file_from_event(&file_event).await {
                        Ok(file_model) => {
                            tracing::info!(
                                "Successfully stored file: {} (ID: {})",
                                file_model.path,
                                file_model.id
                            );
                        }
                        Err(error) => {
                            tracing::error!("Failed to upsert file: {error:?}");
                            return Err(error);
                        }
                    }
                }
                EventKind::Remove(_) => {
                    // File was deleted, remove from database
                    for path in &file_event.paths {
                        tracing::info!("File with path {path:#?} is getting removed from db");
                        match db_operations.delete_file_by_path(path).await {
                            Ok(deleted) => {
                                if deleted {
                                    tracing::info!(
                                        "Successfully removed file from database: {}",
                                        path.display()
                                    );
                                } else {
                                    tracing::info!(
                                        "File not found in database (already removed?): {}",
                                        path.display()
                                    );
                                }
                            }
                            Err(error) => {
                                tracing::error!("Failed to delete file from database: {error:?}");
                                return Err(error);
                            }
                        }
                    }
                }
                _ => {
                    // Other event types (e.g., access) - we might not need to handle these
                    tracing::info!("Ignoring event type: {:?}", file_event.kind);
                }
            }
        } else if let Some(folder_event) = event.folder_event {
            match folder_event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    // File was created or modified, insert/update in database
                    match db_operations.upsert_folder_from_event(&folder_event).await {
                        Ok(folder_model) => {
                            tracing::info!(
                                "Successfully stored folder: {} (ID: {})",
                                folder_model.path,
                                folder_model.id
                            );
                        }
                        Err(error) => {
                            tracing::error!("Failed to upsert folder: {error:?}");
                            return Err(error);
                        }
                    }
                }
                EventKind::Remove(_) => {
                    // File was deleted, remove from database
                    for path in &folder_event.paths {
                        match db_operations.delete_folder_by_path(path).await {
                            Ok(deleted) => {
                                if deleted {
                                    tracing::info!(
                                        "Successfully removed folder from database: {}",
                                        path.display()
                                    );
                                } else {
                                    tracing::info!(
                                        "Folder not found in database (already removed?): {}",
                                        path.display()
                                    );
                                }
                            }
                            Err(error) => {
                                tracing::error!("Failed to delete folder from database: {error:?}");
                                return Err(error);
                            }
                        }
                    }
                }
                _ => {
                    // Other event types (e.g., access) - we might not need to handle these
                    tracing::info!("Ignoring event type: {:?}", folder_event.kind);
                }
            }
        } else {
            bail!("watcher event contains neither a file event nor a folder event");
        }
        Ok(())
    }
}

async fn to_file_or_folder_event_and_send(
    event: DebouncedEvent,
    processed_event_tx: &Sender<FSEvent>,
) -> Result<()> {
    let path = event
        .paths
        .last()
        .context("watcher event does not contain a path")?;

    tracing::info!("Deciding handling based on event type for {event:#?}");
    match (path.is_dir(), event.kind) {
        (true, _)
        | (false, EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder)) => {
            tracing::info!("{event:#?} is folder event");
            to_folder_event_and_send(event, processed_event_tx).await?;
        }
        (_, EventKind::Access(_)) => return Ok(()),
        (false, _) => {
            tracing::info!("{event:#?} is file event");
            to_file_event_and_send(event, processed_event_tx).await?;
        }
    }
    Ok(())
}

async fn to_file_event_and_send(
    event: DebouncedEvent,
    processed_event_tx: &Sender<FSEvent>,
) -> Result<()> {
    let kind = event.kind;
    let paths = event.paths.clone();
    tracing::info!("The following paths are involved in the file event: {paths:#?}");
    tracing::info!("The event kind is {kind:#?}");
    let (content_digest, filesystem_object_id) = if matches!(kind, EventKind::Remove(_)) {
        (None, None)
    } else {
        let path = paths
            .last()
            .context("file event does not contain a path to observe")?;
        let before = FilesystemObjectId::observe(path).await?;
        let content_digest = ContentDigest::observe(path).await?;
        let filesystem_object_id = FilesystemObjectId::observe(path).await?;
        ensure!(
            before == filesystem_object_id,
            "file was replaced while the watcher observed it"
        );
        (Some(content_digest), Some(filesystem_object_id))
    };

    let file_event = FileEvent {
        event,
        kind,
        paths,
        content_digest,
        filesystem_object_id,
    };
    tracing::info!("Constructed FileEvent from Raw Stream");

    if let Err(e) = processed_event_tx.send(file_event.into()).await {
        tracing::info!("Error sending processed event into channel: {e:#?}");
    } else {
        tracing::info!("Sending processed event successful");
    }
    Ok(())
}

async fn to_folder_event_and_send(
    event: DebouncedEvent,
    processed_event_tx: &Sender<FSEvent>,
) -> Result<()> {
    let kind = event.kind;
    let paths = event.paths.clone();
    tracing::info!("The following paths are involved in the folder event: {paths:#?}");
    tracing::info!("The event kind is {kind:#?}");
    let filesystem_object_id = if kind == EventKind::Remove(RemoveKind::Folder) {
        None
    } else {
        Some(
            FilesystemObjectId::observe(
                paths
                    .last()
                    .context("folder event does not contain a path to observe")?,
            )
            .await?,
        )
    };

    let folder_event = FolderEvent {
        event,
        kind,
        paths,
        filesystem_object_id,
    };
    tracing::info!("Constructed FileEvent from Raw Stream");

    if let Err(e) = processed_event_tx.send(folder_event.into()).await {
        tracing::info!("Error sending processed event into channel: {e:#?}");
    } else {
        tracing::info!("Sending processed event successful");
    }
    Ok(())
}
