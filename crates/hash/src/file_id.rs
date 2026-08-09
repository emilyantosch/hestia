use anyhow::{Context, Result};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub enum FileId {
    Inode {
        device_id: u64,
        inode_num: u64,
    },
    Index {
        volume_serial_num: u32,
        file_index: u64,
    },
}
impl FileId {
    #[cfg(target_family = "unix")]
    pub async fn extract(path: impl AsRef<Path>) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;

        let metadata = tokio::fs::metadata(path.as_ref())
            .await
            .context("Failed to extract file metadata for file_id extraction on UNIX!")?;

        Ok(FileId::Inode {
            device_id: metadata.dev(),
            inode_num: metadata.ino(),
        })
    }
}

impl PartialEq for FileId {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                FileId::Inode {
                    device_id: dev1,
                    inode_num: num1,
                },
                FileId::Inode {
                    device_id: dev2,
                    inode_num: num2,
                },
            ) => dev1 == dev2 && num1 == num2,
            (
                FileId::Index {
                    volume_serial_num: vol1,
                    file_index: idx1,
                },
                FileId::Index {
                    volume_serial_num: vol2,
                    file_index: idx2,
                },
            ) => vol1 == vol2 && idx1 == idx2,
            _ => false,
        }
    }
}
