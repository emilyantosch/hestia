use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use entity::prelude::FileTypes;
use entity::{file_has_tags, file_system_identifier, file_types};
use entity::{files, prelude::Files};
use entity::{folders, prelude::Folders};
use events::{FileEvent, FolderEvent};
use hash::file_id::FileId;
use hash::{ContentDigest, FilesystemObjectId};
use model::commands::filter::{Filter, FolderFilter, TagFilter};
use model::commands::watched_folders::WatchedFolderTree;
use model::services::file::{FileSystemFile as File, PersistedFile};
use model::services::folder::FileSystemFolder as Folder;
use notify::EventKind;
use notify::event::{ModifyKind, RenameMode};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter, QuerySelect, TransactionTrait,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::manager::DatabaseManager;
use crate::thumbnail::operations::ThumbnailOperations;

/// Database file metadata for comparison
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub id: i32,
    pub path: PathBuf,
    pub content_digest: ContentDigest,
    pub filesystem_object_id: FilesystemObjectId,
    pub updated_at: DateTime<Utc>,
}

/// File information for bulk operations
#[derive(Clone, Copy, Debug)]
pub struct UpsertFileBatchReport {
    pub file_inserted: usize,
    pub file_updated: usize,
}

/// Folder information for bulk operations
#[derive(Clone, Copy, Debug)]
pub struct UpsertFolderBatchReport {
    pub folder_inserted: usize,
    pub folder_updated: usize,
}

/// Database operations for file management with caching and bulk operations
#[derive(Debug)]
pub struct FileRepository {
    database_manager: Arc<DatabaseManager>,
    file_type_cache: Arc<RwLock<HashMap<String, i32>>>,
    thumbnail_repository: ThumbnailOperations,
}

impl FileRepository {
    #[must_use]
    pub fn new(database_manager: Arc<DatabaseManager>) -> Self {
        let thumbnail_repository = ThumbnailOperations::new(Arc::clone(&database_manager));

        Self {
            database_manager,
            file_type_cache: Arc::new(RwLock::new(HashMap::new())),
            thumbnail_repository,
        }
    }

    /// Get a reference to the thumbnail repository
    #[must_use]
    pub fn thumbnail_repository(&self) -> &ThumbnailOperations {
        &self.thumbnail_repository
    }

    //TODO: Finish this function to return either None for when the folder is one of the root
    //library folders or Some(path), when the folder is at least one level lower than one of the
    //library root folders
    pub async fn find_parent_folder_id<C: ConnectionTrait>(
        &self,
        folder_path: &PathBuf,
        transaction: &C,
    ) -> Result<Option<i32>> {
        if self
            .find_root_folder_paths(transaction)
            .await?
            .contains(folder_path)
        {
            return Ok(None);
        }

        let parent_folder_path = folder_path
            .parent()
            .with_context(|| format!("folder {} has no parent", folder_path.display()))?;

        let parent_folder_model = Folders::find()
            .filter(folders::Column::Path.eq(parent_folder_path.to_string_lossy().to_string()))
            .one(transaction)
            .await?;

        let parent_folder_id = parent_folder_model
            .with_context(|| {
                format!(
                    "parent folder {} is not registered in the database",
                    parent_folder_path.display()
                )
            })?
            .id;

        Ok(Some(parent_folder_id))
    }

    pub async fn upsert_root_folders(&self, library_paths: Vec<PathBuf>) -> Result<()> {
        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        tracing::info!("All library_paths are {library_paths:#?}");

        for path in library_paths {
            self.upsert_root_folder(&transaction, path)
                .await
                .inspect_err(|error| {
                    tracing::error!("The upsert of a root folder failed due to {error:#?}");
                })?;
        }
        transaction.commit().await?;
        tracing::info!("Transaction committed");
        Ok(())
    }

    #[tracing::instrument(skip(transaction), fields(path = %path.display()))]
    async fn upsert_root_folder<C>(&self, transaction: &C, path: PathBuf) -> Result<()>
    where
        C: ConnectionTrait,
    {
        let folder_info = Folder::create_folder_info(&path).await?;
        let path = Self::utf8_path(&path)?.to_string();
        let (device_id, inode) = Self::database_object_id(folder_info.filesystem_object_id)?;
        let existing = Folders::find()
            .filter(folders::Column::Path.eq(&path))
            .one(transaction)
            .await?;

        if let Some(existing) = existing {
            let mut active = existing.into_active_model();
            active.name = Set(folder_info.name);
            active.device_id = Set(device_id);
            active.inode = Set(inode);
            active.parent_folder_id = Set(None);
            active.update(transaction).await?;
        } else {
            folders::ActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                name: Set(folder_info.name),
                path: Set(path),
                parent_folder_id: Set(None),
                device_id: Set(device_id),
                inode: Set(inode),
                created_at: Set(chrono::Local::now().naive_local()),
                updated_at: Set(chrono::Local::now().naive_local()),
            }
            .insert(transaction)
            .await?;
        }
        Ok(())
    }

    pub async fn find_folder_by_id(&self, folder_id: i32) -> Result<Option<folders::Model>> {
        let connection = self.database_manager.get_connection();
        let folder = Folders::find()
            .filter(folders::Column::Id.eq(folder_id))
            .one(&*connection)
            .await?;

        Ok(folder)
    }

    pub async fn find_root_folder_ids<C>(&self, transaction: &C) -> Result<Vec<i32>>
    where
        C: ConnectionTrait,
    {
        let root_folders = self.find_root_folders(Some(transaction)).await?;
        let root_folder_ids = root_folders.into_iter().map(|v| v.id).collect();
        Ok(root_folder_ids)
    }

    pub async fn find_root_folders<C>(&self, transaction: Option<&C>) -> Result<Vec<folders::Model>>
    where
        C: ConnectionTrait,
    {
        let root_folders = if let Some(transaction) = transaction {
            Self::find_root_folders_with(transaction).await?
        } else {
            let connection = self.database_manager.get_connection();
            Self::find_root_folders_with(connection.as_ref()).await?
        };
        Ok(root_folders)
    }

    pub async fn find_root_folder_paths<C>(&self, transaction: &C) -> Result<Vec<PathBuf>>
    where
        C: ConnectionTrait,
    {
        let root_folders = self.find_root_folders(Some(transaction)).await?;

        let root_folder_paths = root_folders
            .into_iter()
            .map(|v| PathBuf::from(v.path))
            .collect();
        Ok(root_folder_paths)
    }

    pub async fn find_subfolders_of_folder(&self, folder_id: i32) -> Result<Vec<folders::Model>> {
        let connection = self.database_manager.get_connection();
        let subfolders = Folders::find()
            .filter(folders::Column::ParentFolderId.eq(folder_id))
            .all(&*connection)
            .await?;

        Ok(subfolders)
    }

    pub async fn upsert_folder_from_event(&self, event: &FolderEvent) -> Result<folders::Model> {
        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        let folder_path = event
            .paths
            .last()
            .context("cannot upsert a folder event without a path")?;
        let folder_name = folder_path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("path {} has no valid folder name", folder_path.display()))?
            .to_string();
        let path = Self::utf8_path(folder_path)?.to_string();
        let filesystem_object_id = event
            .filesystem_object_id
            .context("cannot upsert a folder event without a filesystem object ID")?;
        let (device_id, inode) = Self::database_object_id(filesystem_object_id)?;
        let parent_folder_id = self
            .find_parent_folder_id(folder_path, &transaction)
            .await?;
        let existing = Folders::find()
            .filter(folders::Column::Path.eq(&path))
            .one(&transaction)
            .await?;
        let existing = if existing.is_none()
            && event.paths.len() == 2
            && matches!(
                event.kind,
                EventKind::Modify(ModifyKind::Name(RenameMode::Both))
            ) {
            let old_path = event
                .paths
                .first()
                .context("rename event does not contain its old path")?;
            Folders::find()
                .filter(folders::Column::Path.eq(Self::utf8_path(old_path)?))
                .one(&transaction)
                .await?
        } else {
            existing
        };

        let folder_model = if let Some(existing) = existing {
            let mut active = existing.into_active_model();
            active.name = Set(folder_name);
            active.path = Set(path);
            active.parent_folder_id = Set(parent_folder_id);
            active.device_id = Set(device_id);
            active.inode = Set(inode);
            active.updated_at = Set(chrono::Local::now().naive_local());
            active.update(&transaction).await?
        } else {
            folders::ActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                name: Set(folder_name),
                path: Set(path),
                parent_folder_id: Set(parent_folder_id),
                device_id: Set(device_id),
                inode: Set(inode),
                created_at: Set(Utc::now().naive_utc()),
                updated_at: Set(Utc::now().naive_utc()),
            }
            .insert(&transaction)
            .await?
        };

        transaction.commit().await?;
        Ok(folder_model)
    }
    /// Insert or update a file in the database based on `FileEvent`
    pub async fn upsert_file_from_event(&self, event: &FileEvent) -> Result<files::Model> {
        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        let file_path = event
            .paths
            .last()
            .context("cannot upsert a file event without a path")?;
        let file_name = file_path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("path {} has no valid file name", file_path.display()))?
            .to_string();
        let path = Self::utf8_path(file_path)?.to_string();
        let file_type_id = self
            .get_or_create_file_type(file_path, &transaction)
            .await?;
        let content_digest = event
            .content_digest
            .context("cannot upsert a file event without a content digest")?;
        let filesystem_object_id = event
            .filesystem_object_id
            .context("cannot upsert a file event without a filesystem object ID")?;
        let (device_id, inode) = Self::database_object_id(filesystem_object_id)?;

        let existing = Files::find()
            .filter(files::Column::Path.eq(&path))
            .one(&transaction)
            .await?;
        let existing = if existing.is_none()
            && event.paths.len() == 2
            && matches!(
                &event.kind,
                EventKind::Modify(ModifyKind::Name(RenameMode::Both))
            ) {
            let old_path = event
                .paths
                .first()
                .context("rename event does not contain its old path")?;
            Files::find()
                .filter(files::Column::Path.eq(Self::utf8_path(old_path)?))
                .one(&transaction)
                .await?
        } else {
            existing
        };

        let file_model = if let Some(existing) = existing {
            let mut active_model = existing.into_active_model();
            active_model.name = Set(file_name);
            active_model.path = Set(path);
            active_model.content_digest = Set(content_digest.as_bytes().to_vec());
            active_model.device_id = Set(device_id);
            active_model.inode = Set(inode);
            active_model.file_type_id = Set(file_type_id);
            active_model.updated_at = Set(chrono::Local::now().naive_local());
            active_model.update(&transaction).await?
        } else {
            files::ActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                name: Set(file_name),
                path: Set(path),
                content_digest: Set(content_digest.as_bytes().to_vec()),
                device_id: Set(device_id),
                inode: Set(inode),
                file_type_id: Set(file_type_id),
                created_at: Set(Utc::now().naive_utc()),
                updated_at: Set(Utc::now().naive_utc()),
            }
            .insert(&transaction)
            .await?
        };

        transaction.commit().await?;
        Ok(file_model)
    }

    /// Delete a file record from the database
    pub async fn delete_file_by_path(&self, file_path: &Path) -> Result<bool> {
        tracing::info!("FileOperations: Deleting path {file_path:#?} from database");
        let path = Self::utf8_path(file_path)?;
        let connection = self.database_manager.get_connection();

        let result = Files::delete_many()
            .filter(files::Column::Path.eq(path))
            .exec(&*connection)
            .await?;

        Ok(result.rows_affected > 0)
    }

    /// Delete a file record from the database
    pub async fn delete_folder_by_path(&self, folder_path: &Path) -> Result<bool> {
        let path_str = folder_path.to_string_lossy().to_string();
        let connection = self.database_manager.get_connection();

        let result = Folders::delete_many()
            .filter(folders::Column::Path.eq(&path_str))
            .exec(&*connection)
            .await?;

        Ok(result.rows_affected > 0)
    }

    /// Get or create a file type based on file extension
    async fn get_or_create_file_type<C>(&self, file_path: &Path, connection: &C) -> Result<i32>
    where
        C: ConnectionTrait,
    {
        let file_type_name = Self::detect_file_type(file_path);

        // Check if file type already exists
        if let Some(existing_type) = FileTypes::find()
            .filter(file_types::Column::Name.eq(&file_type_name))
            .one(connection)
            .await?
        {
            return Ok(existing_type.id);
        }

        // Create new file type
        let new_file_type = file_types::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            name: Set(file_type_name),
        };

        let created_type = new_file_type.insert(connection).await?;

        Ok(created_type.id)
    }

    /// Detect file type based on file extension
    fn detect_file_type(file_path: &Path) -> String {
        match file_path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => {
                let ext_lower = ext.to_lowercase();
                match ext_lower.as_str() {
                    // Document types
                    "md" | "markdown" => "markdown",
                    "txt" => "text",
                    "pdf" => "pdf",
                    "doc" | "docx" => "document",
                    "xls" | "xlsx" => "spreadsheet",
                    "ppt" | "pptx" => "presentation",

                    // Image types
                    "jpg" | "jpeg" => "image_jpeg",
                    "png" => "image_png",
                    "gif" => "image_gif",
                    "svg" => "image_svg",
                    "webp" => "image_webp",
                    "bmp" => "image_bmp",

                    // Video types
                    "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" => "video",

                    // Audio types
                    "mp3" | "wav" | "flac" | "ogg" | "aac" => "audio",

                    // Code types
                    "rs" => "rust",
                    "js" | "ts" => "javascript",
                    "py" => "python",
                    "java" => "java",
                    "cpp" | "cc" | "cxx" => "cpp",
                    "c" => "c",
                    "h" | "hpp" => "header",
                    "html" | "htm" => "html",
                    "css" => "css",
                    "json" => "json",
                    "xml" => "xml",
                    "yaml" | "yml" => "yaml",
                    "toml" => "toml",

                    // Archive types
                    "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" => "archive",

                    // Default
                    _ => {
                        return format!("ext_{ext_lower}");
                    }
                }
                .to_string()
            }
            None => {
                // Check if it's a directory
                if file_path.is_dir() {
                    "directory".to_string()
                } else {
                    "unknown".to_string()
                }
            }
        }
    }

    fn utf8_path(path: &Path) -> Result<&str> {
        path.to_str()
            .with_context(|| format!("path {} is not valid UTF-8", path.display()))
    }

    fn database_object_id(object_id: FilesystemObjectId) -> Result<(i64, i64)> {
        Ok((
            object_id
                .device
                .try_into()
                .context("filesystem device ID exceeds the SQLite INTEGER range")?,
            object_id
                .inode
                .try_into()
                .context("filesystem inode exceeds the SQLite INTEGER range")?,
        ))
    }

    /// Get file by path
    pub async fn get_file_by_path(&self, file_path: &Path) -> Result<Option<files::Model>> {
        let path = Self::utf8_path(file_path)?;
        let connection = self.database_manager.get_connection();

        let files = Files::find()
            .filter(files::Column::Path.eq(path))
            .one(&*connection)
            .await?;
        Ok(files)
    }

    /// Get all files in a directory
    pub async fn get_files_in_directory(&self, dir_path: &Path) -> Result<Vec<files::Model>> {
        let pattern = format!("{}%", Self::utf8_path(dir_path)?);
        let connection = self.database_manager.get_connection();

        let files = Files::find()
            .filter(files::Column::Path.like(&pattern))
            .all(&*connection)
            .await?;
        Ok(files)
    }

    // === BULK OPERATIONS FOR SCANNER ===

    /// Get directory state as a map for efficient comparison
    pub async fn get_directory_state(
        &self,
        dir_path: &Path,
    ) -> Result<HashMap<PathBuf, FileMetadata>> {
        let files = self.get_files_in_directory(dir_path).await?;

        let mut state = HashMap::new();
        for file in files {
            let updated_at = file.updated_at.and_utc();
            let file = PersistedFile::try_from(file)?;
            let metadata = FileMetadata {
                id: file.id,
                path: file.path.clone(),
                content_digest: file.content_digest,
                filesystem_object_id: file.filesystem_object_id,
                updated_at,
            };
            state.insert(file.path, metadata);
        }

        Ok(state)
    }

    pub async fn get_watched_folder_map(&self) -> Result<HashMap<String, WatchedFolderTree>> {
        let mut map = HashMap::new();

        let connection = self.database_manager.get_connection();
        let folders = Folders::find().all(&*connection).await?;

        let (with_parent, without_parent): (Vec<folders::Model>, Vec<folders::Model>) = folders
            .into_iter()
            .partition(|model| model.parent_folder_id.is_some());

        let root_children = without_parent
            .into_iter()
            .map(|model| model.id.to_string())
            .collect();
        let root = WatchedFolderTree::with(
            "".to_string(),
            "".to_string(),
            Some(root_children),
            None,
            None,
        );
        map.insert("0".to_string(), root);
        tracing::info!("First map value inserted {map:#?}");

        for folder in with_parent {
            let children = Folders::find()
                .select_only()
                .column(folders::Column::Id)
                .filter(folders::Column::ParentFolderId.eq(folder.id))
                .all(&*connection)
                .await?;
            let children_array: Option<Vec<String>> = if children.is_empty() {
                None
            } else {
                Some(children.into_iter().map(|v| v.id.to_string()).collect())
            };
            let wf = WatchedFolderTree::with(folder.name, folder.path, children_array, None, None);
            map.insert(folder.id.to_string(), wf);
        }
        tracing::info!("Complete map {map:#?}");
        Ok(map)
    }

    /// Batch insert/update files with transaction
    pub async fn batch_upsert_files(&self, files: Vec<File>) -> Result<UpsertFileBatchReport> {
        if files.is_empty() {
            return Ok(UpsertFileBatchReport {
                file_inserted: 0,
                file_updated: 0,
            });
        }

        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        let mut file_inserted = 0;
        let mut file_updated = 0;

        for file_info in files {
            let file_type_id = self
                .get_or_create_file_type_cached(&file_info.file_type_name, &transaction)
                .await?;
            let path = Self::utf8_path(&file_info.path)?.to_string();
            let (device_id, inode) = Self::database_object_id(file_info.filesystem_object_id)?;
            let existing_file = Files::find()
                .filter(files::Column::Path.eq(&path))
                .one(&transaction)
                .await?;

            if let Some(existing) = existing_file {
                let mut active_model = existing.into_active_model();
                active_model.name = Set(file_info.name);
                active_model.content_digest = Set(file_info.content_digest.as_bytes().to_vec());
                active_model.device_id = Set(device_id);
                active_model.inode = Set(inode);
                active_model.file_type_id = Set(file_type_id);
                active_model.updated_at = Set(Utc::now().naive_utc());
                active_model.update(&transaction).await?;
                file_updated += 1;
            } else {
                files::ActiveModel {
                    id: sea_orm::ActiveValue::NotSet,
                    name: Set(file_info.name),
                    path: Set(path),
                    content_digest: Set(file_info.content_digest.as_bytes().to_vec()),
                    device_id: Set(device_id),
                    inode: Set(inode),
                    file_type_id: Set(file_type_id),
                    created_at: Set(Utc::now().naive_utc()),
                    updated_at: Set(Utc::now().naive_utc()),
                }
                .insert(&transaction)
                .await?;
                file_inserted += 1;
            }
        }

        transaction.commit().await?;
        Ok(UpsertFileBatchReport {
            file_inserted,
            file_updated,
        })
    }

    pub async fn batch_upsert_folders(
        &self,
        folders: Vec<Folder>,
    ) -> Result<UpsertFolderBatchReport> {
        if folders.is_empty() {
            return Ok(UpsertFolderBatchReport {
                folder_inserted: 0,
                folder_updated: 0,
            });
        }

        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        let mut folder_inserted = 0;
        let mut folder_updated = 0;

        for folder in folders {
            let parent_folder_id = self
                .find_parent_folder_id(&folder.path, &transaction)
                .await?;
            let path = Self::utf8_path(&folder.path)?.to_string();
            let (device_id, inode) = Self::database_object_id(folder.filesystem_object_id)?;
            let existing = Folders::find()
                .filter(folders::Column::Path.eq(&path))
                .one(&transaction)
                .await?;

            if let Some(existing) = existing {
                let mut active = existing.into_active_model();
                active.name = Set(folder.name);
                active.parent_folder_id = Set(parent_folder_id);
                active.device_id = Set(device_id);
                active.inode = Set(inode);
                active.updated_at = Set(Utc::now().naive_utc());
                active.update(&transaction).await?;
                folder_updated += 1;
            } else {
                folders::ActiveModel {
                    id: sea_orm::ActiveValue::NotSet,
                    name: Set(folder.name),
                    path: Set(path),
                    parent_folder_id: Set(parent_folder_id),
                    device_id: Set(device_id),
                    inode: Set(inode),
                    created_at: Set(Utc::now().naive_utc()),
                    updated_at: Set(Utc::now().naive_utc()),
                }
                .insert(&transaction)
                .await?;
                folder_inserted += 1;
            }
        }

        transaction.commit().await?;
        Ok(UpsertFolderBatchReport {
            folder_inserted,
            folder_updated,
        })
    }

    /// Batch delete files by paths
    pub async fn batch_delete_files(&self, paths: Vec<PathBuf>) -> Result<usize> {
        if paths.is_empty() {
            return Ok(0);
        }

        let path_strings = paths
            .iter()
            .map(|path| Self::utf8_path(path).map(str::to_owned))
            .collect::<Result<Vec<_>>>()?;

        let connection = self.database_manager.get_connection();
        let result = Files::delete_many()
            .filter(files::Column::Path.is_in(path_strings))
            .exec(&*connection)
            .await?;

        Ok(result.rows_affected as usize)
    }

    /// Batch delete files by paths
    pub async fn batch_delete_folders(&self, paths: Vec<PathBuf>) -> Result<usize> {
        if paths.is_empty() {
            return Ok(0);
        }

        let path_strings: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let connection = self.database_manager.get_connection();
        let result = Folders::delete_many()
            .filter(folders::Column::Path.is_in(path_strings))
            .exec(&*connection)
            .await?;

        Ok(result.rows_affected as usize)
    }

    /// Clear file type cache (useful for testing or cache invalidation)
    pub fn clear_file_type_cache(&self) {
        self.file_type_cache
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Get or create file type with caching
    async fn get_or_create_file_type_cached<C>(
        &self,
        file_type_name: &str,
        connection: &C,
    ) -> Result<i32>
    where
        C: ConnectionTrait,
    {
        // Check cache first
        {
            let cache = self
                .file_type_cache
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(&type_id) = cache.get(file_type_name) {
                return Ok(type_id);
            }
        }

        // Not in cache, get or create from database
        let type_id = self
            .get_or_create_file_type_by_name(file_type_name, connection)
            .await?;

        // Update cache
        {
            let mut cache = self
                .file_type_cache
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.insert(file_type_name.to_string(), type_id);
        }

        Ok(type_id)
    }

    /// Get or create file type by name (without path inference)
    async fn get_or_create_file_type_by_name<C>(
        &self,
        file_type_name: &str,
        connection: &C,
    ) -> Result<i32>
    where
        C: ConnectionTrait,
    {
        // Check if file type already exists
        if let Some(existing_type) = FileTypes::find()
            .filter(file_types::Column::Name.eq(file_type_name))
            .one(connection)
            .await?
        {
            return Ok(existing_type.id);
        }

        // Create new file type
        let new_file_type = file_types::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            name: Set(file_type_name.to_string()),
        };

        let created_type = new_file_type.insert(connection).await?;

        Ok(created_type.id)
    }

    /// Get or create file system identifier based on file metadata
    pub async fn get_or_create_file_system_identifier<C: ConnectionTrait>(
        &self,
        file_path: &Path,
        transaction: &C,
    ) -> Result<i32> {
        let file_id = FileId::extract(file_path).await?;
        match file_id {
            FileId::Inode {
                device_id,
                inode_num,
            } => {
                let inode = i64::try_from(inode_num)
                    .context("filesystem inode exceeds the SQLite INTEGER range")?;
                let device = i64::try_from(device_id)
                    .context("filesystem device ID exceeds the SQLite INTEGER range")?;
                let existing_fsi = file_system_identifier::Entity::find()
                    .filter(file_system_identifier::Column::Inode.eq(inode))
                    .filter(file_system_identifier::Column::DeviceNum.eq(device))
                    .one(transaction)
                    .await?;

                if let Some(fsi) = existing_fsi {
                    return Ok(fsi.id);
                }

                let new_fsi = file_system_identifier::ActiveModel {
                    id: sea_orm::ActiveValue::NotSet,
                    inode: Set(Some(inode)),
                    device_num: Set(Some(device)),
                    index_num: sea_orm::ActiveValue::NotSet,
                    volume_serial_num: sea_orm::ActiveValue::NotSet,
                };
                let created_fsi = new_fsi.insert(transaction).await?;
                Ok(created_fsi.id)
            }
            FileId::Index {
                volume_serial_num,
                file_index,
            } => {
                let file_index = i64::try_from(file_index)
                    .context("filesystem index exceeds the SQLite INTEGER range")?;
                let volume_serial_num = i64::from(volume_serial_num);
                let existing_fsi = file_system_identifier::Entity::find()
                    .filter(file_system_identifier::Column::VolumeSerialNum.eq(volume_serial_num))
                    .filter(file_system_identifier::Column::IndexNum.eq(file_index))
                    .one(transaction)
                    .await?;
                if let Some(fsi) = existing_fsi {
                    return Ok(fsi.id);
                }

                let new_fsi = file_system_identifier::ActiveModel {
                    id: sea_orm::ActiveValue::NotSet,
                    inode: sea_orm::ActiveValue::NotSet,
                    device_num: sea_orm::ActiveValue::NotSet,
                    index_num: Set(Some(file_index)),
                    volume_serial_num: Set(Some(volume_serial_num)),
                };
                let created_fsi = new_fsi.insert(transaction).await?;
                Ok(created_fsi.id)
            }
        }
    }

    /// Preload common file types into cache
    pub async fn preload_file_type_cache(&self) -> Result<()> {
        let connection = self.database_manager.get_connection();
        let all_types = FileTypes::find().all(&*connection).await?;

        let mut cache = self
            .file_type_cache
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for file_type in all_types {
            cache.insert(file_type.name, file_type.id);
        }

        Ok(())
    }

    /*
     * SELECT name
     * FROM files f
     * LEFT JOIN file_has_tags fht ON fht.file_id = f.id
     * WHERE fht.id = tag.id AND fht.id = tag.id AND (f.path like folderPath% OR f.path like folderPath%)
     */
    pub async fn get_files_for_filter(&self, filter: Filter) -> Result<Vec<files::Model>> {
        let connection = self.database_manager.get_connection();
        let mut folder_condition: Condition = Condition::any();
        let mut tag_condition: Condition = Condition::all();
        match (filter.tag_filter.as_ref(), filter.folder_filter.as_ref()) {
            (Some(tag_filter), Some(folder_filter)) => {
                folder_condition = Self::get_folder_filter_condition(folder_filter);
                tag_condition = Self::get_tag_filter_condition(tag_filter);
            }
            (Some(tag_filter), None) => {
                tag_condition = Self::get_tag_filter_condition(tag_filter);
            }
            (None, Some(folder_filter)) => {
                folder_condition = Self::get_folder_filter_condition(folder_filter);
            }
            (None, None) => (),
        }
        let files = Files::find()
            .left_join(file_has_tags::Entity)
            .filter(folder_condition)
            .filter(tag_condition)
            .all(&*connection)
            .await?;
        Ok(files)
    }

    fn get_folder_filter_condition(filter: &FolderFilter) -> Condition {
        let mut condition = Condition::any();
        for folder in &filter.folders {
            condition =
                condition.add(files::Column::Path.like(format!("{}%", folder.to_string_lossy())));
        }
        condition
    }

    fn get_tag_filter_condition(filter: &TagFilter) -> Condition {
        let mut condition = Condition::all();
        for tag in &filter.tags {
            condition = condition.add(file_has_tags::Column::TagId.eq(tag.id));
        }
        condition
    }

    async fn find_root_folders_with<C>(transaction: &C) -> Result<Vec<folders::Model>>
    where
        C: ConnectionTrait,
    {
        let root_folders = Folders::find()
            .filter(folders::Column::ParentFolderId.is_null())
            .all(transaction)
            .await?;
        Ok(root_folders)
    }
}
