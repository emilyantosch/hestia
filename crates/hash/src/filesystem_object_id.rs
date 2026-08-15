use anyhow::{Context, Result};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use tokio::fs;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FilesystemObjectId {
    pub device: u64,
    pub inode: u64,
}

impl FilesystemObjectId {
    pub async fn observe(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata = fs::symlink_metadata(path)
            .await
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}
