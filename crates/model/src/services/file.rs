use anyhow::{Context, Result, ensure};
use entity::files;
use hash::{ContentDigest, FilesystemObjectId};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileSystemFile {
    pub id: Option<i32>,
    pub path: PathBuf,
    pub name: String,
    pub content_digest: ContentDigest,
    pub filesystem_object_id: FilesystemObjectId,
    pub file_type_name: String,
}

impl TryFrom<files::Model> for FileSystemFile {
    type Error = anyhow::Error;

    fn try_from(value: files::Model) -> Result<Self> {
        let path = PathBuf::from(&value.path);
        let file_type_name = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(Self {
            id: Some(value.id),
            path,
            name: value.name,
            content_digest: digest_from_database(value.content_digest)?,
            filesystem_object_id: object_id_from_database(value.device_id, value.inode)?,
            file_type_name,
        })
    }
}

impl FileSystemFile {
    pub async fn create_file_info_from_path(path: &Path) -> Result<Self> {
        path.to_str()
            .with_context(|| format!("path {} is not valid UTF-8", path.display()))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("path {} has no valid file name", path.display()))?
            .to_string();
        let before = FilesystemObjectId::observe(path).await?;
        let content_digest = ContentDigest::observe(path).await?;
        let filesystem_object_id = FilesystemObjectId::observe(path).await?;
        ensure!(
            before == filesystem_object_id,
            "file was replaced while it was being observed"
        );

        Ok(Self {
            id: None,
            path: path.to_path_buf(),
            name,
            content_digest,
            filesystem_object_id,
            file_type_name: infer::get_from_path(path)?.map_or_else(
                || "unknown".to_string(),
                |kind| kind.mime_type().to_string(),
            ),
        })
    }
}

#[derive(Debug, Clone)]
pub struct PersistedFile {
    pub id: i32,
    pub path: PathBuf,
    pub name: String,
    pub content_digest: ContentDigest,
    pub filesystem_object_id: FilesystemObjectId,
}

impl TryFrom<files::Model> for PersistedFile {
    type Error = anyhow::Error;

    fn try_from(value: files::Model) -> Result<Self> {
        Ok(Self {
            id: value.id,
            path: PathBuf::from(value.path),
            name: value.name,
            content_digest: digest_from_database(value.content_digest)?,
            filesystem_object_id: object_id_from_database(value.device_id, value.inode)?,
        })
    }
}

fn digest_from_database(bytes: Vec<u8>) -> Result<ContentDigest> {
    let length = bytes.len();
    let bytes = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("database content digest must be 32 bytes, got {length}"))?;
    Ok(ContentDigest::from_bytes(bytes))
}

fn object_id_from_database(device_id: i64, inode: i64) -> Result<FilesystemObjectId> {
    Ok(FilesystemObjectId {
        device: device_id
            .try_into()
            .context("database device ID cannot be negative")?,
        inode: inode
            .try_into()
            .context("database inode cannot be negative")?,
    })
}

#[cfg(test)]
mod tests {
    use super::FileSystemFile;
    use entity::files;

    fn database_file(content_digest: Vec<u8>, device_id: i64) -> files::Model {
        let now = chrono::Utc::now().naive_utc();
        files::Model {
            id: 1,
            name: "file".to_string(),
            path: "/file".to_string(),
            content_digest,
            device_id,
            inode: 2,
            file_type_id: 1,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn database_observation_rejects_invalid_digest_and_object_id() {
        assert!(FileSystemFile::try_from(database_file(vec![0; 31], 1)).is_err());
        assert!(FileSystemFile::try_from(database_file(vec![0; 32], -1)).is_err());
    }
}
