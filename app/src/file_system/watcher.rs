use notify::{recommended_watcher, Event, RecursiveMode, Result, Watcher};
use notify_debouncer_full::{new_debouncer, notify::*, DebounceEventResult};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task;

pub struct FileWatcher {
    sender: Arc<mpsc::Sender<Event>>,
    receiver: Arc<mpsc::Receiver<Event>>,
}

impl FileWatcher {
    pub async fn new(dir_path: PathBuf) -> Result<Self, std::io::Error> {
        let (tx, rx) = mpsc::channel(10); // Buffered channel
        let tx_arc = Arc::new(tx);
        let rx_arc = Arc::new(rx);
        let watcher_arc = Arc::clone(&tx_arc);

        tokio::spawn(async move {
            let mut watcher: RecommendedWatcher =
                match RecommendedWatcher::new(watcher_arc.as_ref(), RecursiveMode::Recursive) {
                    Ok(watcher) => watcher,
                    Err(e) => {
                        eprintln!("Error creating watcher: {:?}", e);
                        return;
                    }
                };

            if let Err(e) = watcher.watch(&dir_path, RecursiveMode::Recursive) {
                eprintln!("Error watching directory: {:?}", e);
                return;
            }

            while let Ok(event) = watcher.next() {
                if let Err(e) = tx_arc.send(event).await {
                    eprintln!("Error sending event: {:?}", e);
                    break;
                }
                println!("Hellow :3");
            }
        });

        Ok(Self {
            sender: tx_arc,
            receiver: rx_arc,
        })
    }

    pub fn get_receiver(&self) -> Arc<mpsc::Sender<Event>> {
        Arc::clone(&self.receiver)
    }
}
