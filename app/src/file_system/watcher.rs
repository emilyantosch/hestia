use notify::{Error, Event, RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, FileIdMap,
};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

pub struct FileWatcher {
    watcher: Option<Debouncer<RecommendedWatcher, FileIdMap>>,
    receiver: Option<Receiver<std::result::Result<Vec<DebouncedEvent>, Vec<Error>>>>,
}

impl FileWatcher {
    pub async fn init_watcher(&mut self) {
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let rt = tokio::runtime::Handle::current();

        let debouncer = new_debouncer(
            Duration::from_secs(2),
            None,
            move |result: DebounceEventResult| {
                let tx = tx.clone();

                println!("Calling by notify => {:?}", &result);
                rt.spawn(async move {
                    if let Err(e) = tx.send(result).await {
                        println!("Error sending event result: {:?}", e);
                    };
                });
            },
        );

        match debouncer {
            Ok(watcher) => {
                println!("Init of FileWatcher completed successfully!");
                self.watcher = Some(watcher);
                self.receiver = Some(rx);
            }
            Err(e) => println!("{:?}", e),
        };
    }
    pub async fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            watcher: None,
            receiver: None,
        })
    }
}
