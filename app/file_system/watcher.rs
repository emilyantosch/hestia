use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use fsnotify::{FsNotify, FilesystemEvent};

type EventSender = mpsc::UnboundedSender<FilesystemEvent>;

pub struct FileWatcher {
    sender: Arc<EventSender>,
    dir_path: PathBuf,
}

impl FileWatcher {
    pub async fn new(dir_path: PathBuf) -> Result<Self, std::io::Error> {
        let (sender, receiver) = mpsc::unbounded_channel();
        
        // Create an Arc for the sender to share across threads
        let sender_arc = Arc::new(sender);
        
        // Spawn a task to watch filesystem events
        tokio::spawn({
            let dir_path = dir_path.clone();
            let sender = sender_arc.clone();
            
            async move {
                let mut watcher = FsNotify::new()?;
                
                // Watch all files and subdirectories recursively
                watcher.watch(&dir_path, FilesystemEvent::all())?;
                
                while let Ok(event) = watcher.next() {
                    if let Some(sender) = sender_arc.as_ref().try_clone() {
                        if sender.send(event).is_ok() {
                            continue;
                        }
                    }
                    
                    break;
                }
                
                Ok(())
            }
        });
        
        Ok(Self {
            sender: sender_arc,
            dir_path,
        })
    }

    pub fn get_sender(&self) -> Arc<EventSender> {
        self.sender.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    
    #[tokio::test]
    async fn test_watcher() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir_path = PathBuf::from(temp_dir.path());
        
        let watcher = FileWatcher::new(dir_path.clone()).await.unwrap();
        
        // Create a file
        let test_file = dir_path.join("test.txt");
        std::fs::File::create(&test_file).unwrap();
        
        // Modify the file
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&test_file)
            .unwrap();
        file.write_all(b"test").unwrap();
        
        // Delete the file
        std::fs::remove_file(test_file).unwrap();
    }
}
