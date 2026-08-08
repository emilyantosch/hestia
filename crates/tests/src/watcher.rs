use anyhow::Error as AppError;
use services::fs::
use crate::file_system::FileWatcherMessage;
use crate::file_system::watcher::{FSEvent, FileWatcher};
use notify::EventKind::Create;

#[cfg(test)]
use crate::file_system::TestFileWatcherEventHandler;
use crate::utils::canon_path::CanonPath;

use notify::EventKind;
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use std::fs;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::sync::mpsc::error::TryRecvError;

use tokio::sync::Mutex;
#[cfg(test)]
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

// Helper function to create a FileWatcher instance for tests
#[cfg(test)]
async fn run_test_watcher() -> Result<
    (
        UnboundedSender<FileWatcherMessage>,
        UnboundedReceiver<FSEvent>,
    ),
    AppError,
> {
    // Initialize tracing for tests
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_test_writer() // This makes it work with cargo test
        .try_init()
        .ok(); // Ignore errors if already initialize
    let (fw_sender, fw_receiver) = tokio::sync::mpsc::unbounded_channel::<FileWatcherMessage>();
    let (fw_event_sender, fw_event_receiver) = tokio::sync::mpsc::unbounded_channel::<FSEvent>();
    let fw_event_handler = TestFileWatcherEventHandler {
        sender: Arc::new(Mutex::new(fw_event_sender)),
    };
    let watcher = FileWatcher::new(fw_receiver);
    tokio::spawn(async move {
        let _ = watcher.run(Box::new(fw_event_handler)).await;
    });
    Ok((fw_sender, fw_event_receiver))
}

#[tokio::test]
async fn on_file_create_emit_correct_event() -> Result<(), AppError> {
    let tmp_dir = tempdir().unwrap();
    let file_path = tmp_dir.path().join("test_file.txt");
    tracing::info!("Defined test file path at {file_path:#?}");

    let mut watcher = run_test_watcher().await?;
    tracing::info!("Watcher started");
    let _ = watcher
        .0
        .send(FileWatcherMessage::WatchPath(CanonPath::from(
            tmp_dir.path().to_path_buf(),
        )));

    tracing::info!("Watcher send command to worker thread");
    // Give the watcher a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create a file to trigger an event
    let mut file = std::fs::File::create(&file_path).unwrap();
    tracing::info!("File {file:#?} created");
    writeln!(file, "Hello, world!").unwrap();
    tracing::info!("File {file:#?} written to");
    drop(file); // Close the file

    // Wait for the event to be processed
    // The debouncer is set to 2 seconds, plus some buffer
    tokio::time::sleep(Duration::from_secs(3)).await;

    tracing::info!("Checking for change...");
    match tokio::time::timeout(Duration::from_secs(1), watcher.1.recv()).await {
        Ok(Some(event)) => {
            tracing::info!("Got event from worker! {event:#?}");
            assert!(matches!(
                event.file_event.as_ref().unwrap().kind,
                notify::EventKind::Create(CreateKind::File)
            ));
            assert_eq!(
                event.file_event.as_ref().unwrap().paths,
                vec![file_path.clone()]
            );
            // Further assertions on file_id if necessary,
            // e.g. check if it's not default or matches expected hash
            let expected_file_id = FileId::extract(&file_path).await.unwrap();
            assert_eq!(
                event
                    .file_event
                    .as_ref()
                    .unwrap()
                    .hash
                    .as_ref()
                    .unwrap()
                    .file_id,
                expected_file_id
            );
        }
        Ok(None) => panic!("Processed event channel closed prematurely"),
        Err(_) => panic!("Timeout waiting for processed event for create"),
    }
    Ok(())
}

#[tokio::test]
async fn on_file_modify_emit_correct_event() -> Result<(), AppError> {
    let tmp_dir = tempdir().unwrap();
    let file_path = tmp_dir.path().join("mod_test_file.txt");

    // 1. Create and write initial content
    let mut file = fs::File::create(&file_path).expect("Failed to create test file");
    writeln!(file, "Initial content").expect("Failed to write initial content");
    drop(file);

    // 2. Create and start watcher
    let (message_sender, mut event_receiver) = run_test_watcher().await?;
    let _ = message_sender.send(FileWatcherMessage::WatchPath(CanonPath::from(
        tmp_dir.path().to_path_buf(),
    )));

    // Give watcher time to pick up initial create event and drain it
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Drain initial create event
    if let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        println!("Drained initial create event");
    }

    // 3. Modify the file
    fs::write(&file_path, "New content").expect("Failed to write new content to file");

    // 4. Wait for the modify event to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 5. Receive and assert the modify event
    match tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        Ok(Some(event)) => {
            assert!(
                matches!(
                    event.file_event.as_ref().unwrap().kind,
                    EventKind::Modify(_)
                ),
                "Event kind was not Modify: {:?}",
                event.file_event.as_ref().unwrap().kind
            );
            assert_eq!(
                event.file_event.as_ref().unwrap().paths,
                vec![file_path.clone()]
            );
            let expected_file_id = FileId::extract(&file_path).await.unwrap();
            assert_eq!(
                event
                    .file_event
                    .as_ref()
                    .unwrap()
                    .hash
                    .as_ref()
                    .unwrap()
                    .file_id,
                expected_file_id
            );
        }
        Ok(None) => panic!("Processed event channel closed prematurely"),
        Err(_) => panic!("Timeout waiting for processed modify event"),
    }
    Ok(())
}

#[tokio::test]
async fn on_file_delete_emit_correct_event() -> Result<(), AppError> {
    let tmp_dir = tempdir().unwrap();
    let file_path = tmp_dir.path().join("del_test_file.txt");

    // 1. Create the file
    fs::File::create(&file_path).expect("Failed to create test file for deletion test");

    // 2. Create and start watcher
    let (message_sender, mut event_receiver) = run_test_watcher().await?;
    let _ = message_sender.send(FileWatcherMessage::WatchPath(CanonPath::from(
        tmp_dir.path().to_path_buf(),
    )));

    // Give watcher time to pick up initial create event and drain it
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Drain initial create event
    if let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        println!("Drained initial create event");
    }

    // 3. Delete the file
    fs::remove_file(&file_path).expect("Failed to delete test file");

    // 4. Wait for the delete event to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 5. Receive and assert the delete event
    match tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        Ok(Some(event)) => {
            assert!(
                matches!(
                    event.file_event.as_ref().unwrap().kind,
                    EventKind::Remove(RemoveKind::File)
                ),
                "Event kind was not Remove(File): {:?}",
                event.file_event.as_ref().unwrap().kind
            );
            assert_eq!(
                event.file_event.as_ref().unwrap().paths,
                vec![file_path.clone()]
            );
        }
        Ok(None) => panic!("Processed event channel closed prematurely"),
        Err(_) => panic!("Timeout waiting for processed delete event"),
    }
    Ok(())
}

#[tokio::test]
async fn on_rename_emit_correct_event() -> Result<(), AppError> {
    let tmp_dir = tempdir().unwrap();
    let old_file_path = tmp_dir.path().join("rename_old.txt");
    let new_file_path = tmp_dir.path().join("rename_new.txt");

    // 1. Create the file at the old path
    fs::File::create(&old_file_path)
        .expect("Failed to create test file for rename test (old_file_path)");

    // 2. Create and start watcher
    let (message_sender, mut event_receiver) = run_test_watcher().await?;
    let _ = message_sender.send(FileWatcherMessage::WatchPath(CanonPath::from(
        tmp_dir.path().to_path_buf(),
    )));

    // Give watcher time to pick up initial create event and drain it
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Drain initial create event
    if let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        println!("Drained initial create event");
    }

    // 3. Rename the file
    fs::rename(&old_file_path, &new_file_path).expect("Failed to rename test file");

    // 4. Wait for the rename event to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 5. Receive and assert the rename event
    match tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        Ok(Some(event)) => {
            assert!(
                matches!(
                    event.file_event.as_ref().unwrap().kind,
                    EventKind::Modify(ModifyKind::Name(RenameMode::Both))
                ),
                "Event kind was not Modify(Name(Both)): {:?}",
                event.file_event.as_ref().unwrap().kind
            );
            assert_eq!(
                event.file_event.as_ref().unwrap().paths,
                vec![old_file_path.clone(), new_file_path.clone()]
            );
            let expected_file_id = FileId::extract(&new_file_path).await.unwrap();
            assert_eq!(
                event
                    .file_event
                    .as_ref()
                    .unwrap()
                    .hash
                    .as_ref()
                    .unwrap()
                    .file_id,
                expected_file_id
            );
        }
        Ok(None) => panic!("Processed event channel closed prematurely"),
        Err(_) => panic!("Timeout waiting for processed rename event"),
    }
    Ok(())
}

#[tokio::test]
async fn on_folder_create_emit_correct_event() -> Result<(), AppError> {
    let tmp_dir = tempdir().unwrap();
    let folder_path = tmp_dir.path().join("new_test_folder");

    // 1. Create and start watcher
    let (message_sender, mut event_receiver) = run_test_watcher().await?;
    let _ = message_sender.send(FileWatcherMessage::WatchPath(CanonPath::from(
        tmp_dir.path().to_path_buf(),
    )));

    // Give watcher a moment to initialize
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Create the new folder
    fs::create_dir(&folder_path).expect("Failed to create test folder");

    // 3. Wait for the folder creation event to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 4. Receive and assert the folder creation event
    match tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        Ok(Some(event)) => {
            assert!(
                matches!(
                    event.folder_event.as_ref().unwrap().kind,
                    EventKind::Create(CreateKind::Folder)
                ),
                "Event kind was not Create(Folder): {:?}",
                event.folder_event.as_ref().unwrap().kind
            );
            assert_eq!(
                event.folder_event.as_ref().unwrap().paths,
                vec![folder_path.clone()]
            );
            let expected_file_id = FileId::extract(&folder_path).await.unwrap();
            assert_eq!(
                event
                    .folder_event
                    .as_ref()
                    .unwrap()
                    .hash
                    .as_ref()
                    .unwrap()
                    .file_id,
                expected_file_id
            );
        }
        Ok(None) => panic!("Processed event channel closed prematurely"),
        Err(_) => panic!("Timeout waiting for processed folder create event"),
    }
    Ok(())
}

#[tokio::test]
async fn on_folder_delete_emit_correct_event() -> Result<(), AppError> {
    let tmp_dir = tempdir().unwrap();
    let folder_path = tmp_dir.path().join("del_test_folder");

    // 1. Create the folder
    fs::create_dir(&folder_path).expect("Failed to create test folder for delete test");

    // 2. Create and start watcher
    let (message_sender, mut event_receiver) = run_test_watcher().await?;
    let _ = message_sender.send(FileWatcherMessage::WatchPath(CanonPath::from(
        tmp_dir.path().to_path_buf(),
    )));

    // Give watcher time to pick up initial create event and drain it
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Drain initial create event
    if let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        println!("Drained initial create event");
    }

    // 3. Delete the folder
    fs::remove_dir(&folder_path).expect("Failed to delete test folder");

    // 4. Wait for the folder delete event to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 5. Receive and assert the folder delete event
    match tokio::time::timeout(Duration::from_secs(2), event_receiver.recv()).await {
        Ok(Some(event)) => {
            assert!(
                matches!(
                    event.folder_event.as_ref().unwrap().kind,
                    EventKind::Remove(RemoveKind::Folder)
                ),
                "Event kind was not Remove(Folder): {:?}",
                event.folder_event.as_ref().unwrap().kind
            );
            assert_eq!(
                event.folder_event.as_ref().unwrap().paths,
                vec![folder_path.clone()]
            );
        }
        Ok(None) => panic!("Processed event channel closed prematurely"),
        Err(_) => panic!("Timeout waiting for processed folder delete event"),
    }
    Ok(())
}

#[tokio::test]
async fn on_folder_rename_emit_correct_event() -> Result<(), AppError> {
    let tmp_dir = tempdir().unwrap();
    let old_folder_path = tmp_dir.path().join("rename_old_folder");
    let new_folder_path = tmp_dir.path().join("rename_new_folder");

    // 1. Create the folder at the old path
    fs::create_dir(&old_folder_path)
        .expect("Failed to create test folder for rename test (old_folder_path)");

    // 2. Create and start watcher
    let (message_sender, mut event_receiver) = run_test_watcher().await?;
    let _ = message_sender.send(FileWatcherMessage::WatchPath(CanonPath::from(
        tmp_dir.path().to_path_buf(),
    )));

    // Give watcher time to pick up initial create event and drain it
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Drain initial create event
    if let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        println!("Drained initial create event");
    }

    // 3. Rename the folder
    fs::rename(&old_folder_path, &new_folder_path).expect("Failed to rename test folder");

    // 4. Wait for the folder rename event to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 5. Receive and assert the folder rename event
    match tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
        Ok(Some(event)) => {
            assert!(
                matches!(
                    event.folder_event.as_ref().unwrap().kind,
                    EventKind::Modify(ModifyKind::Name(RenameMode::Both))
                ),
                "Event kind was not Modify(Name(Both)) for folder rename: {:?}",
                event.folder_event.as_ref().unwrap().kind
            );
            assert_eq!(
                event.folder_event.as_ref().unwrap().paths,
                vec![old_folder_path.clone(), new_folder_path.clone()]
            );
            let expected_file_id = FileId::extract(&new_folder_path).await.unwrap();
            assert_eq!(
                event
                    .folder_event
                    .as_ref()
                    .unwrap()
                    .hash
                    .as_ref()
                    .unwrap()
                    .file_id,
                expected_file_id
            );
        }
        Ok(None) => panic!("Processed event channel closed prematurely"),
        Err(_) => panic!("Timeout waiting for processed folder rename event"),
    }
    Ok(())
}

#[tokio::test]
async fn on_watch_non_existent_path_return_error() -> Result<(), AppError> {
    let tmp_dir = tempdir().unwrap();
    let non_existent_path = tmp_dir.path().join("this_path_should_not_exist");

    // Create watcher
    let (message_sender, mut event_receiver) = run_test_watcher().await?;

    // Try to watch non-existent path - this should be sent but handled gracefully
    let _ = message_sender.send(FileWatcherMessage::WatchPath(CanonPath::from(
        non_existent_path.to_path_buf(),
    )));

    // Wait a moment for any potential processing
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Try to create a file in the non-existent directory (this should fail)
    let result = fs::File::create(non_existent_path.join("test_file.txt"));

    // Assert that file creation fails (confirming path doesn't exist)
    assert!(
        result.is_err(),
        "File creation should fail in non-existent directory"
    );

    // Wait and confirm no events are received (since path doesn't exist)
    match tokio::time::timeout(Duration::from_secs(2), event_receiver.recv()).await {
        Ok(Some(_)) => panic!("Should not receive events for non-existent path"),
        Ok(None) => panic!("Event channel closed unexpectedly"),
        Err(_) => {
            // Timeout is expected - no events should be generated for non-existent paths
            println!("Correctly received no events for non-existent path");
        }
    }

    Ok(())
}
