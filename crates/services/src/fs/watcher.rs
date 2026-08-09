use anyhow::{Context, Result, bail};
use events::{FileEvent, FolderEvent};
use hash::hash::{FileHash, FolderHash};
use model::services::CanonPath;
use notify::event::{CreateKind, EventKind, RemoveKind};
use notify::{Error, RecommendedWatcher};
use notify_debouncer_full::{
    DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache, new_debouncer,
};
use repositories::fs::operations::FileRepository as FileOperations;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{self, Sender, UnboundedSender};
use tokio::sync::{Mutex, oneshot};

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

type RawEventReceiver = Option<
    Arc<Mutex<tokio::sync::mpsc::Receiver<std::result::Result<Vec<DebouncedEvent>, Vec<Error>>>>>,
>;

#[derive(Debug)]
pub struct FileWatcherHandler {
    pub sender: mpsc::UnboundedSender<FileWatcherMessage>,
}

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

impl FileWatcher {
    pub async fn init_watcher(
        &mut self,
        event_handler: Box<dyn FileWatcherEventHandler>,
    ) -> Result<()> {
        let (r_tx, mut r_rx) = tokio::sync::mpsc::channel(100);
        let rt = tokio::runtime::Handle::current();
        let (p_tx, mut p_rx) = tokio::sync::mpsc::channel::<FSEvent>(100);

        let debouncer = new_debouncer(
            Duration::from_secs(2),
            None,
            move |result: DebounceEventResult| {
                let r_tx_clone = r_tx.clone();
                rt.spawn(async move {
                    if let Err(e) = r_tx_clone.send(result).await {
                        tracing::info!("Error sending event result: {:?}", e);
                    };
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

        match debouncer {
            Ok(watcher) => {
                tracing::info!("Init of FileWatcher completed successfully!");
                self.watcher = Some(watcher);
            }
            Err(e) => tracing::error!("{:?}", e),
        };
        Ok(())
    }

    pub fn new(message_receiver: mpsc::UnboundedReceiver<FileWatcherMessage>) -> FileWatcher {
        Self {
            watcher: None,
            message_receiver,
            watched_paths: HashSet::new(),
        }
    }

    pub async fn run(mut self, event_handler: Box<dyn FileWatcherEventHandler>) -> Result<()> {
        self.init_watcher(event_handler).await?;
        while let Some(res) = self.message_receiver.recv().await {
            match res {
                FileWatcherMessage::WatchPath(path) => {
                    self.watched_paths.insert(path);
                }
                FileWatcherMessage::UnwatchPath(path) => {
                    self.watched_paths.remove(&path);
                }
                FileWatcherMessage::GetWatchPaths(sender) => {
                    let paths = self.watched_paths.clone();
                    drop(sender.send(paths));
                }
            }
            self.watch();
        }
        Ok(())
    }

    fn watch(&self) {
        todo!()
    }

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
                        Err(e) => {
                            tracing::error!("Failed to upsert file: {:?}", e);
                            return Err(e)?;
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
                            Err(e) => {
                                tracing::error!("Failed to delete file from database: {:?}", e);
                                return Err(e)?;
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
                                "Successfully stored file: {} (ID: {})",
                                folder_model.path,
                                folder_model.id
                            );
                        }
                        Err(e) => {
                            tracing::error!("Failed to upsert file: {:?}", e);
                            return Err(e)?;
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
                            Err(e) => {
                                tracing::error!("Failed to delete file from database: {:?}", e);
                                return Err(e)?;
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
        | (false, EventKind::Create(CreateKind::Folder))
        | (false, EventKind::Remove(RemoveKind::Folder)) => {
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
    let paths = event.paths.to_owned();
    let mut hash: Option<FileHash> = None;
    tracing::info!("The following paths are involved in the file event: {paths:#?}");
    tracing::info!("The event kind is {kind:#?}");
    if kind != EventKind::Remove(RemoveKind::File) {
        hash = Some(
            FileHash::hash(
                paths
                    .last()
                    .context("file event does not contain a path to hash")?,
            )
            .await?,
        );
    }

    let file_event = FileEvent {
        event,
        kind,
        paths,
        hash,
    };
    tracing::info!("Constructed FileEvent from Raw Stream");

    if let Err(e) = processed_event_tx.send(file_event.into()).await {
        tracing::info!("Error sending processed event into channel: {e:#?}");
    } else {
        tracing::info!("Sending processed event successful")
    }
    Ok(())
}

async fn to_folder_event_and_send(
    event: DebouncedEvent,
    processed_event_tx: &Sender<FSEvent>,
) -> Result<()> {
    let kind = event.kind;
    let paths = event.paths.to_owned();
    let mut hash = None;
    tracing::info!("The following paths are involved in the file event: {paths:#?}");
    tracing::info!("The event kind is {kind:#?}");
    if kind != EventKind::Remove(RemoveKind::Folder) {
        hash = Some(
            FolderHash::hash(
                paths
                    .last()
                    .context("folder event does not contain a path to hash")?,
            )
            .await?,
        );
    }

    let folder_event = FolderEvent {
        event,
        kind,
        paths,
        hash,
    };
    tracing::info!("Constructed FileEvent from Raw Stream");

    if let Err(e) = processed_event_tx.send(folder_event.into()).await {
        tracing::info!("Error sending processed event into channel: {e:#?}");
    } else {
        tracing::info!("Sending processed event successful")
    }
    Ok(())
}
