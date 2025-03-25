use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::task;
use futures::StreamExt;
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::Arc;

// Define a type alias for the event
type FileSystemEvent = DebouncedEvent<String, Error>;

pub struct FileWatcher {
    sender: Arc<mpsc::Sender<FileSystemEvent>>,
}

impl FileWatcher {
    pub async fn new(dir_path: PathBuf) -> Result<Self, std::io::Error> {
        let (tx, rx) = mpsc::channel(10); // Buffered channel
        let tx_arc = Arc::new(tx);

        let watcher_arc = Arc::clone(&tx_arc);

        tokio::spawn(async move {
            let mut watcher: RecommendedWatcher = match RecommendedWatcher::new(watcher_arc.as_ref(), RecursiveMode::Recursive) {
                Ok(watcher) => watcher,
                Err(e) => {
                    eprintln!("Error creating watcher: {:?}", e);
                    return;
                }
            };

            if let Err(e) = watcher.watch(&dir_path) {
                eprintln!("Error watching directory: {:?}", e);
                return;
            }

            while let Ok(event) = watcher.next() {
                if let Err(e) = tx_arc.send(event).await {
                    eprintln!("Error sending event: {:?}", e);
                    break;
                }
            }
        });

        Ok(Self { sender: tx_arc })
    }

    pub fn get_receiver(&self) -> Arc<mpsc::Sender<FileSystemEvent>> {
        Arc::clone(&self.sender)
    }
}
