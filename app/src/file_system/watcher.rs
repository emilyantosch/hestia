use notify::{Config, Error, Event, RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{
    new_debouncer_opt, DebounceEventResult, DebouncedEvent, Debouncer, FileIdMap,
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
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let rt = tokio::runtime::Handle::current();

        let debouncer = new_debouncer_opt(
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
            FileIdMap::new(),
            Config::default(),
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

    pub async fn watch(&mut self, path: &PathBuf) -> notify::Result<()> {
        if !path.exists() {
            panic!();
        }
        println!("Watching path: {:?}", path);

        if let Some(watcher) = self.watcher.as_mut() {
            watcher.watch(path, RecursiveMode::Recursive)?;
            println!("Watcher ready!");

            if let Some(mut rx) = self.receiver.take() {
                println!("RX taken out of Option");
                tokio::spawn(async move {
                    println!("Spawned thread!");
                    while let Some(res) = rx.recv().await {
                        println!("Received events!");
                        match res {
                            Ok(events) => {
                                println!("event: {:?}", events);
                            }
                            Err(e) => {
                                println!("errors: {:?}", e);
                            }
                        }
                    }
                });
            }
        }
        Ok(())
    }
}
