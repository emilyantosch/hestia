use anyhow::{Context, Result, ensure};
use blake3::Hasher;
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::SystemTime;
use tokio::fs;
use tokio::io::AsyncReadExt;

const READ_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ContentDigest([u8; blake3::OUT_LEN]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MetadataSnapshot {
    device: u64,
    inode: u64,
    length: u64,
    modified: SystemTime,
}

impl ContentDigest {
    pub async fn observe(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let initial = snapshot(
            &fs::symlink_metadata(path)
                .await
                .with_context(|| format!("failed to inspect {}", path.display()))?,
        )?;
        let mut file = fs::File::open(path)
            .await
            .with_context(|| format!("failed to open {}", path.display()))?;
        let opened = snapshot(&file.metadata().await?)?;
        ensure!(initial == opened, "file changed while it was being opened");

        let mut hasher = Hasher::new();
        let mut buffer = vec![0; READ_BUFFER_SIZE];
        let mut bytes_read = 0_u64;
        loop {
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            bytes_read = bytes_read
                .checked_add(count as u64)
                .context("file length exceeded the supported range")?;
            ensure!(
                bytes_read <= initial.length,
                "file length changed while it was being read"
            );
            hasher.update(
                buffer
                    .get(..count)
                    .context("reader returned an invalid byte count")?,
            );
        }

        let finished = snapshot(&file.metadata().await?)?;
        let current = snapshot(
            &fs::symlink_metadata(path)
                .await
                .with_context(|| format!("failed to inspect {} after reading", path.display()))?,
        )?;
        ensure!(initial == finished, "file changed while it was being read");
        ensure!(
            initial == current,
            "file path was replaced while it was being read"
        );
        ensure!(
            bytes_read == initial.length,
            "file length changed while it was being read"
        );

        Ok(Self(hasher.finalize().into()))
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; blake3::OUT_LEN] {
        &self.0
    }
}

fn snapshot(metadata: &Metadata) -> Result<MetadataSnapshot> {
    ensure!(
        metadata.file_type().is_file(),
        "entry is not a regular file"
    );
    Ok(MetadataSnapshot {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        modified: metadata
            .modified()
            .context("file modification time is unavailable")?,
    })
}

#[cfg(test)]
mod tests {
    use super::{ContentDigest, READ_BUFFER_SIZE};
    use anyhow::Result;

    #[tokio::test]
    async fn observe_reads_to_eof_across_multiple_chunks() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("file");
        let content = vec![42; READ_BUFFER_SIZE * 2 + 17];
        tokio::fs::write(&path, &content).await?;

        let observed = ContentDigest::observe(path).await?;

        assert_eq!(observed.as_bytes(), blake3::hash(&content).as_bytes());
        Ok(())
    }
}
