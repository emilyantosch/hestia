use crate::services::folder::PersistedFolder;
use serde::{Deserialize, Serialize};

/// Folder information for frontend display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderInfo {
    pub id: i32,
    pub name: String,
    pub path: String,
    pub parent_folder_id: Option<i32>,
    pub device_id: u64,
    pub inode: u64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<PersistedFolder> for FolderInfo {
    fn from(folder: PersistedFolder) -> Self {
        Self {
            id: folder.id,
            name: folder.name,
            path: folder.path.to_string_lossy().to_string(),
            parent_folder_id: folder.parent_folder_id,
            device_id: folder.filesystem_object_id.device,
            inode: folder.filesystem_object_id.inode,
            created_at: folder.created_at.to_string(),
            updated_at: folder.updated_at.to_string(),
        }
    }
}
