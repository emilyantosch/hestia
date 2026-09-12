use std::path::PathBuf;

use hash::{ContentDigest, FilesystemObjectId};
use notify::EventKind;
use notify_debouncer_full::DebouncedEvent;

#[derive(Debug)]
pub struct FileEvent {
    pub event: DebouncedEvent,
    pub paths: Vec<PathBuf>,
    pub kind: EventKind,
    pub content_digest: Option<ContentDigest>,
    pub filesystem_object_id: Option<FilesystemObjectId>,
}

#[derive(Debug)]
pub struct FolderEvent {
    pub event: DebouncedEvent,
    pub paths: Vec<PathBuf>,
    pub kind: EventKind,
    pub filesystem_object_id: Option<FilesystemObjectId>,
}
