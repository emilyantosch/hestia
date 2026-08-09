use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use entity::prelude::FileTypes;
use entity::{file_has_tags, file_system_identifier, file_types};
use entity::{files, prelude::Files};
use entity::{folders, prelude::Folders};
use events::{FileEvent, FolderEvent};
use hash::file_id::FileId;
use model::commands::filter::{Filter, FolderFilter, TagFilter};
use model::commands::watched_folders::WatchedFolderTree;
use model::services::file::FileSystemFile as File;
use model::services::folder::FileSystemFolder as Folder;
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
    pub content_hash: String,
    pub identity_hash: String,
    pub file_system_id: i32,
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
        let possible_root_folder = Folders::find()
            .filter(folders::Column::Path.eq(path.to_string_lossy().to_string()))
            .one(transaction)
            .await?;
        let folder_info = Folder::create_folder_info(&path).await?;
        tracing::info!("Got folder info {folder_info:#?} for root folder {path:#?}");
        let file_system_id = self
            .get_or_create_file_system_identifier(&path, transaction)
            .await?;
        tracing::info!("Got file system id {file_system_id:#?} for root folder {path:#?}");
        if let Some(rf) = possible_root_folder {
            let mut active_rf = rf.into_active_model();
            active_rf.name = Set(folder_info.name);
            active_rf.file_system_id = Set(file_system_id);
            active_rf.parent_folder_id = Set(None);
            active_rf.content_hash = Set(folder_info.content_hash);
            active_rf.identity_hash = Set(folder_info.identity_hash);
            active_rf.structure_hash = Set(folder_info.structure_hash);

            tracing::info!("Updating existing root folder {path:#?}");
            active_rf.update(transaction).await?;
        } else {
            let new_folder = folders::ActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                name: Set(folder_info.name),
                path: Set(path.to_string_lossy().to_string()),
                parent_folder_id: Set(None),
                content_hash: Set(folder_info.content_hash),
                identity_hash: Set(folder_info.identity_hash),
                structure_hash: Set(folder_info.structure_hash),
                file_system_id: Set(file_system_id),
                created_at: Set(chrono::Local::now().naive_local()),
                updated_at: Set(chrono::Local::now().naive_local()),
            };

            tracing::info!("Inserting existing root folder {path:#?}");
            new_folder.insert(transaction).await?;
            tracing::info!("Insert of {path:#?} complete!");
        }
        Ok(())
    }

    pub async fn find_folder_by_id(&self, fsi_id: i32) -> Result<Option<folders::Model>> {
        let connection = self.database_manager.get_connection();
        let folder = Folders::find()
            .filter(folders::Column::Id.eq(fsi_id))
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
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let path_str = folder_path.to_string_lossy().to_string();

        let hash = event
            .hash
            .as_ref()
            .context("cannot upsert a folder event without a hash")?;
        let content_hash_str = format!("{:?}", hash.content_hash);
        let identity_hash_str = format!("{:?}", hash.identity_hash);
        let structure_hash_str = format!("{:?}", hash.structure_hash);

        // Get file system identifier
        let file_system_id = self
            .get_or_create_file_system_identifier(folder_path, &transaction)
            .await?;

        let parent_folder_id = self
            .find_parent_folder_id(folder_path, &transaction)
            .await?;

        //  What we actually wanna do is check if the file exists by fsi and/or hash.
        let folder_with_fsi = Folders::find()
            .filter(folders::Column::FileSystemId.eq(file_system_id))
            .one(&transaction)
            .await?;

        let folder_model = if let Some(existing) = folder_with_fsi {
            // Update existing file
            let mut active_model = existing.into_active_model();
            active_model.name = Set(folder_name);
            active_model.path = Set(path_str);
            active_model.content_hash = Set(content_hash_str);
            active_model.identity_hash = Set(identity_hash_str);
            active_model.structure_hash = Set(structure_hash_str);
            active_model.parent_folder_id = Set(parent_folder_id);
            active_model.file_system_id = Set(file_system_id);
            active_model.updated_at = Set(chrono::Local::now().naive_local());

            active_model.update(&transaction).await?
        } else {
            // Insert new file
            let new_folder = folders::ActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                name: Set(folder_name),
                path: Set(path_str),
                parent_folder_id: Set(parent_folder_id),
                content_hash: Set(content_hash_str),
                identity_hash: Set(identity_hash_str),
                structure_hash: Set(structure_hash_str),
                file_system_id: Set(file_system_id),
                created_at: Set(Utc::now().naive_utc()),
                updated_at: Set(Utc::now().naive_utc()),
            };

            new_folder.insert(&transaction).await?
        };

        transaction.commit().await?;

        Ok(folder_model)
    }
    /// Insert or update a file in the database based on `FileEvent`
    pub async fn upsert_file_from_event(&self, event: &FileEvent) -> Result<files::Model> {
        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;

        // Extract file information from the event
        let file_path = event
            .paths
            .last()
            .context("cannot upsert a file event without a path")?;

        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let path_str = file_path.to_string_lossy().to_string();

        // Get or create file type
        let file_type_id = self
            .get_or_create_file_type(file_path, &transaction)
            .await?;

        let hash = event
            .hash
            .as_ref()
            .context("cannot upsert a file event without a hash")?;
        let content_hash_str = format!("{:?}", hash.content_hash);
        let identity_hash_str = format!("{:?}", hash.identity_hash);

        // Get file system identifier
        let file_system_id = self
            .get_or_create_file_system_identifier(file_path, &transaction)
            .await?;

        //  What we actually wanna do is check if the file exists by fsi and/or hash.
        let file_with_fsi = Files::find()
            .filter(files::Column::FileSystemId.eq(file_system_id))
            .one(&transaction)
            .await?;

        let file_model = if let Some(existing) = file_with_fsi {
            // Update existing file
            let mut active_model = existing.into_active_model();
            active_model.name = Set(file_name);
            active_model.path = Set(path_str);
            active_model.content_hash = Set(content_hash_str);
            active_model.identity_hash = Set(identity_hash_str);
            active_model.file_type_id = Set(file_type_id);
            active_model.file_system_id = Set(file_system_id);
            active_model.updated_at = Set(chrono::Local::now().naive_local());

            active_model.update(&transaction).await?
        } else {
            // Insert new file
            let new_file = files::ActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                name: Set(file_name),
                path: Set(path_str),
                content_hash: Set(content_hash_str),
                identity_hash: Set(identity_hash_str),
                file_type_id: Set(file_type_id),
                file_system_id: Set(file_system_id),
                created_at: Set(Utc::now().naive_utc()),
                updated_at: Set(Utc::now().naive_utc()),
            };

            new_file.insert(&transaction).await?
        };

        transaction.commit().await?;
        Ok(file_model)
    }

    /// Delete a file record from the database
    pub async fn delete_file_by_path(&self, file_path: &Path) -> Result<bool> {
        tracing::info!("FileOperations: Deleting path {file_path:#?} from database");
        let path_str = file_path.to_string_lossy().to_string();
        let connection = self.database_manager.get_connection();

        let result = Files::delete_many()
            .filter(files::Column::Path.eq(&path_str))
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

    /// Get file by path
    pub async fn get_file_by_path(&self, file_path: &Path) -> Result<Option<files::Model>> {
        let path_str = file_path.to_string_lossy().to_string();
        let connection = self.database_manager.get_connection();

        let files = Files::find()
            .filter(files::Column::Path.eq(&path_str))
            .one(&*connection)
            .await?;
        Ok(files)
    }

    /// Get all files in a directory
    pub async fn get_files_in_directory(&self, dir_path: &Path) -> Result<Vec<files::Model>> {
        let dir_str = dir_path.to_string_lossy().to_string();
        let pattern = format!("{dir_str}%");
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
            let metadata = FileMetadata {
                id: file.id,
                path: PathBuf::from(&file.path),
                content_hash: file.content_hash,
                identity_hash: file.identity_hash,
                file_system_id: file.file_system_id,
                updated_at: file.updated_at.and_utc(),
            };
            state.insert(PathBuf::from(file.path), metadata);
        }

        Ok(state)
    }

    /// Get file hashes as a map for quick comparison
    pub async fn get_file_hashes_map(
        &self,
        dir_path: &Path,
    ) -> Result<HashMap<PathBuf, (String, String)>> {
        let files = self.get_files_in_directory(dir_path).await?;

        let mut hashes = HashMap::new();
        for file in files {
            hashes.insert(
                PathBuf::from(file.path),
                (file.content_hash, file.identity_hash),
            );
        }

        Ok(hashes)
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
            // Get or create file type (with caching)
            let file_type_id = self
                .get_or_create_file_type_cached(&file_info.file_type_name, &transaction)
                .await?;

            // Get file system identifier
            let file_system_id = if let Some(fsi_id) = file_info.file_system_id {
                fsi_id
            } else {
                self.get_or_create_file_system_identifier(&file_info.path, &transaction)
                    .await?
            };

            // Check if file exists
            let existing_file = Files::find()
                .filter(files::Column::Path.eq(file_info.path.to_string_lossy().to_string()))
                .one(&transaction)
                .await?;

            if let Some(existing) = existing_file {
                // Update existing file
                let mut active_model = existing.into_active_model();
                active_model.name = Set(file_info.name);
                active_model.content_hash = Set(file_info.content_hash);
                active_model.identity_hash = Set(file_info.identity_hash);
                active_model.file_type_id = Set(file_type_id);
                active_model.file_system_id = Set(file_system_id);
                active_model.updated_at = Set(Utc::now().naive_utc());

                active_model.update(&transaction).await?;
                file_inserted += 1;
            } else {
                // Insert new file
                let new_file = files::ActiveModel {
                    id: sea_orm::ActiveValue::NotSet,
                    name: Set(file_info.name),
                    path: Set(file_info.path.to_string_lossy().to_string()),
                    content_hash: Set(file_info.content_hash),
                    identity_hash: Set(file_info.identity_hash),
                    file_type_id: Set(file_type_id),
                    file_system_id: Set(file_system_id),
                    created_at: Set(Utc::now().naive_utc()),
                    updated_at: Set(Utc::now().naive_utc()),
                };

                new_file.insert(&transaction).await?;

                file_updated += 1;
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

        for folder_info in folders {
            // Find parent folder id
            let parent_folder_id = self
                .find_parent_folder_id(&folder_info.path, &transaction)
                .await?;

            // Get file system identifier
            let file_system_id = match folder_info.file_system_id {
                Some(fsi_id) => fsi_id,
                None => {
                    self.get_or_create_file_system_identifier(&folder_info.path, &transaction)
                        .await?
                }
            };

            // Check if file exists
            let possible_folder = Folders::find()
                .filter(folders::Column::Path.eq(folder_info.path.to_string_lossy().to_string()))
                .one(&transaction)
                .await?;

            if let Some(existing) = possible_folder {
                // Update existing file
                let mut existing_folder = existing.into_active_model();
                existing_folder.name = Set(folder_info.name);
                existing_folder.content_hash = Set(folder_info.content_hash);
                existing_folder.identity_hash = Set(folder_info.identity_hash);
                existing_folder.structure_hash = Set(folder_info.structure_hash);
                existing_folder.parent_folder_id = Set(parent_folder_id);
                existing_folder.file_system_id = Set(file_system_id);
                existing_folder.updated_at = Set(Utc::now().naive_utc());

                existing_folder.update(&transaction).await?;
                folder_updated += 1;
            } else {
                // Insert new file
                let new_folder = folders::ActiveModel {
                    id: sea_orm::ActiveValue::NotSet,
                    name: Set(folder_info.name),
                    path: Set(folder_info.path.to_string_lossy().to_string()),
                    content_hash: Set(folder_info.content_hash),
                    identity_hash: Set(folder_info.identity_hash),
                    structure_hash: Set(folder_info.structure_hash),
                    parent_folder_id: Set(parent_folder_id),
                    file_system_id: Set(file_system_id),
                    created_at: Set(Utc::now().naive_utc()),
                    updated_at: Set(Utc::now().naive_utc()),
                };

                new_folder.insert(&transaction).await?;
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

        let path_strings: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

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
                let existing_fsi = file_system_identifier::Entity::find()
                    .filter(file_system_identifier::Column::Inode.eq(inode_num))
                    .filter(file_system_identifier::Column::DeviceNum.eq(device_id))
                    .one(transaction)
                    .await?;

                if let Some(fsi) = existing_fsi {
                    return Ok(fsi.id);
                }

                // Create new file system identifier
                let new_fsi = file_system_identifier::ActiveModel {
                    id: sea_orm::ActiveValue::NotSet,
                    inode: Set(Some(inode_num.try_into().unwrap_or(0))),
                    device_num: Set(Some(device_id.try_into().unwrap_or(0))),
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
                let existing_fsi = file_system_identifier::Entity::find()
                    .filter(file_system_identifier::Column::VolumeSerialNum.eq(volume_serial_num))
                    .filter(file_system_identifier::Column::IndexNum.eq(file_index))
                    .one(transaction)
                    .await?;
                if let Some(fsi) = existing_fsi {
                    return Ok(fsi.id);
                }

                // Create new file system identifier
                let new_fsi = file_system_identifier::ActiveModel {
                    id: sea_orm::ActiveValue::NotSet,
                    inode: sea_orm::ActiveValue::NotSet,
                    device_num: sea_orm::ActiveValue::NotSet,
                    index_num: Set(Some(file_index.try_into().unwrap_or(0))),
                    volume_serial_num: Set(Some(volume_serial_num.into())),
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
