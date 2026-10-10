use anyhow::{Context, Result};
use hash::FilesystemObjectId;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileSystemFolder {
    pub id: Option<i32>,
    pub path: PathBuf,
    pub name: String,
    pub filesystem_object_id: FilesystemObjectId,
    pub parent_folder_id: Option<i32>,
}

impl FileSystemFolder {
    pub async fn create_folder_info(path: &Path) -> Result<Self> {
        let path = super::IndexedPath::try_from(path)?;
        let path = path.as_ref();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("path {} has no valid folder name", path.display()))?
            .to_string();
        let filesystem_object_id = FilesystemObjectId::observe(path).await?;

        Ok(Self {
            id: None,
            path: path.to_path_buf(),
            name,
            filesystem_object_id,
            parent_folder_id: None,
        })
    }
}
