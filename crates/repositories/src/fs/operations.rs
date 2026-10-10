use anyhow::{Context, Result};
use chrono::Utc;
use entity::file_types;
use entity::prelude::FileTypes;
use entity::{files, prelude::Files};
use entity::{folders, prelude::Folders};
use events::{FileEvent, FolderEvent};
use hash::{ContentDigest, FilesystemObjectId};
use itertools::Itertools;
use model::services::CanonPath;
use model::services::IndexedPath;
use model::services::file::{FileSystemFile as File, PersistedFile};
use model::services::folder::FileSystemFolder as Folder;
use moka::sync::Cache;
use notify::EventKind;
use notify::event::{ModifyKind, RenameMode};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{BinOper, Expr, IntoColumnRef, SimpleExpr};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, TransactionTrait,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::manager::DatabaseManager;
use crate::thumbnail::operations::ThumbnailOperations;

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

/// Columns written on every file insert or update.
struct FileRow {
    name: String,
    path: String,
    content_digest: ContentDigest,
    filesystem_object_id: FilesystemObjectId,
    file_type_id: i32,
}

/// Columns written on every folder insert or update.
struct FolderRow {
    name: String,
    path: String,
    parent_folder_id: Option<i32>,
    filesystem_object_id: FilesystemObjectId,
}

/// Database operations for file management with caching and bulk operations
#[derive(Debug)]
pub struct FileRepository {
    database_manager: Arc<DatabaseManager>,
    file_type_cache: Cache<String, i32>,
    thumbnail_repository: ThumbnailOperations,
}

impl FileRepository {
    #[must_use]
    pub fn new(database_manager: Arc<DatabaseManager>) -> Self {
        let thumbnail_repository = ThumbnailOperations::new(Arc::clone(&database_manager));

        Self {
            database_manager,
            file_type_cache: Cache::new(1_024),
            thumbnail_repository,
        }
    }

    /// Get a reference to the thumbnail repository
    #[must_use]
    pub fn thumbnail_repository(&self) -> &ThumbnailOperations {
        &self.thumbnail_repository
    }

    /// Returns `None` for watched roots and the parent's ID for everything below them.
    async fn find_parent_folder_id<C: ConnectionTrait>(
        folder_path: &Path,
        connection: &C,
    ) -> Result<Option<i32>> {
        let path = Self::database_path(folder_path)?;
        let is_root = Folders::find()
            .filter(folders::Column::Path.eq(&path))
            .filter(folders::Column::ParentFolderId.is_null())
            .count(connection)
            .await?
            > 0;
        if is_root {
            return Ok(None);
        }

        let parent_folder_path = folder_path
            .parent()
            .with_context(|| format!("folder {} has no parent", folder_path.display()))?;
        let parent = Self::find_folder(connection, &Self::database_path(parent_folder_path)?)
            .await?
            .with_context(|| {
                format!(
                    "parent folder {} is not registered in the database",
                    parent_folder_path.display()
                )
            })?;
        Ok(Some(parent.id))
    }

    pub async fn upsert_root_folders(&self, library_paths: Vec<CanonPath>) -> Result<()> {
        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        tracing::info!("All library_paths are {library_paths:#?}");

        for path in library_paths {
            Self::upsert_root_folder(&transaction, &path)
                .await
                .inspect_err(|error| {
                    tracing::error!("The upsert of a root folder failed due to {error:#?}");
                })?;
        }
        transaction.commit().await?;
        tracing::info!("Transaction committed");
        Ok(())
    }

    #[tracing::instrument(skip(transaction), fields(path = %path))]
    async fn upsert_root_folder<C>(transaction: &C, path: &CanonPath) -> Result<()>
    where
        C: ConnectionTrait,
    {
        let folder = Folder::create_folder_info(path.as_ref()).await?;
        let path = Self::database_path(path.as_ref())?;
        let existing = Self::find_folder(transaction, &path).await?;
        let row = FolderRow {
            name: folder.name,
            path,
            parent_folder_id: None,
            filesystem_object_id: folder.filesystem_object_id,
        };
        Self::save_folder(transaction, existing, row).await?;
        Ok(())
    }

    pub async fn find_root_folders(&self) -> Result<Vec<folders::Model>> {
        let connection = self.database_manager.get_connection();
        Folders::find()
            .filter(folders::Column::ParentFolderId.is_null())
            .all(connection.as_ref())
            .await
            .context("Could not find root folders")
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
        let folder_path = IndexedPath::try_from(
            event
                .paths
                .last()
                .context("cannot upsert a folder event without a path")?
                .as_path(),
        )?;
        let path = folder_path.as_str().to_owned();
        let filesystem_object_id = event
            .filesystem_object_id
            .context("cannot upsert a folder event without a filesystem object ID")?;
        let parent_folder_id =
            Self::find_parent_folder_id(folder_path.as_ref(), &transaction).await?;

        let mut existing = Self::find_folder(&transaction, &path).await?;
        if existing.is_none()
            && let Some(old_path) = Self::renamed_from(&event.paths, event.kind)
        {
            existing = Self::find_folder(&transaction, &Self::database_path(old_path)?).await?;
        }

        let row = FolderRow {
            name: Self::entry_name(folder_path.as_ref())?,
            path,
            parent_folder_id,
            filesystem_object_id,
        };
        let folder_model = Self::save_folder(&transaction, existing, row).await?;
        transaction.commit().await?;
        Ok(folder_model)
    }

    /// Insert or update a file in the database based on `FileEvent`
    pub async fn upsert_file_from_event(&self, event: &FileEvent) -> Result<files::Model> {
        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        let file_path = IndexedPath::try_from(
            event
                .paths
                .last()
                .context("cannot upsert a file event without a path")?
                .as_path(),
        )?;
        let path = file_path.as_str().to_owned();
        let file_type_name = Self::detect_file_type(file_path.as_ref());
        let file_type_id = self.file_type_id(&file_type_name, &transaction).await?;
        let content_digest = event
            .content_digest
            .context("cannot upsert a file event without a content digest")?;
        let filesystem_object_id = event
            .filesystem_object_id
            .context("cannot upsert a file event without a filesystem object ID")?;

        let mut existing = Self::find_file(&transaction, &path).await?;
        if existing.is_none()
            && let Some(old_path) = Self::renamed_from(&event.paths, event.kind)
        {
            existing = Self::find_file(&transaction, &Self::database_path(old_path)?).await?;
        }

        let row = FileRow {
            name: Self::entry_name(file_path.as_ref())?,
            path,
            content_digest,
            filesystem_object_id,
            file_type_id,
        };
        let file_model = Self::save_file(&transaction, existing, row).await?;
        transaction.commit().await?;
        self.file_type_cache.insert(file_type_name, file_type_id);
        Ok(file_model)
    }

    /// Delete a file record from the database
    pub async fn delete_file_by_path(&self, file_path: &Path) -> Result<bool> {
        tracing::info!("FileOperations: Deleting path {file_path:#?} from database");
        let path = Self::database_path(file_path)?;
        let connection = self.database_manager.get_connection();

        let result = Files::delete_many()
            .filter(files::Column::Path.eq(path))
            .exec(&*connection)
            .await?;

        self.database_manager.invalidate_thumbnail_cache();
        Ok(result.rows_affected > 0)
    }

    /// Delete a folder record from the database
    pub async fn delete_folder_by_path(&self, folder_path: &Path) -> Result<bool> {
        let path = Self::database_path(folder_path)?;
        let connection = self.database_manager.get_connection();

        let result = Folders::delete_many()
            .filter(folders::Column::Path.eq(&path))
            .exec(&*connection)
            .await?;
        self.database_manager.invalidate_thumbnail_cache();

        Ok(result.rows_affected > 0)
    }

    /// Detect file type based on file extension
    fn detect_file_type(file_path: &Path) -> String {
        let Some(extension) = file_path.extension().and_then(|ext| ext.to_str()) else {
            return if file_path.is_dir() {
                "directory"
            } else {
                "unknown"
            }
            .to_string();
        };
        let extension = extension.to_lowercase();
        let file_type = match extension.as_str() {
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

            _ => return format!("ext_{extension}"),
        };
        file_type.to_string()
    }

    fn database_path(path: &Path) -> Result<String> {
        Ok(IndexedPath::try_from(path)?.as_str().to_owned())
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

    fn entry_name(path: &Path) -> Result<String> {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .with_context(|| format!("path {} has no valid name", path.display()))
    }

    /// The old location of an entry when the event is a complete rename.
    fn renamed_from(paths: &[PathBuf], kind: EventKind) -> Option<&Path> {
        match (kind, paths) {
            (EventKind::Modify(ModifyKind::Name(RenameMode::Both)), [old_path, _]) => {
                Some(old_path)
            }
            _ => None,
        }
    }

    async fn find_file<C: ConnectionTrait>(
        connection: &C,
        path: &str,
    ) -> Result<Option<files::Model>> {
        Ok(Files::find()
            .filter(files::Column::Path.eq(path))
            .one(connection)
            .await?)
    }

    async fn find_folder<C: ConnectionTrait>(
        connection: &C,
        path: &str,
    ) -> Result<Option<folders::Model>> {
        Ok(Folders::find()
            .filter(folders::Column::Path.eq(path))
            .one(connection)
            .await?)
    }

    /// Update `existing` with `row`, or insert `row` when there is no existing record.
    async fn save_file<C: ConnectionTrait>(
        connection: &C,
        existing: Option<files::Model>,
        row: FileRow,
    ) -> Result<files::Model> {
        let (device_id, inode) = Self::database_object_id(row.filesystem_object_id)?;
        let now = Utc::now().naive_utc();
        let is_new = existing.is_none();
        let mut active = existing.map_or_else(
            || files::ActiveModel {
                created_at: Set(now),
                ..Default::default()
            },
            IntoActiveModel::into_active_model,
        );
        active.name = Set(row.name);
        active.path = Set(row.path);
        active.content_digest = Set(row.content_digest.as_bytes().to_vec());
        active.device_id = Set(device_id);
        active.inode = Set(inode);
        active.file_type_id = Set(row.file_type_id);
        active.updated_at = Set(now);
        Ok(if is_new {
            active.insert(connection).await?
        } else {
            active.update(connection).await?
        })
    }

    /// Update `existing` with `row`, or insert `row` when there is no existing record.
    async fn save_folder<C: ConnectionTrait>(
        connection: &C,
        existing: Option<folders::Model>,
        row: FolderRow,
    ) -> Result<folders::Model> {
        let (device_id, inode) = Self::database_object_id(row.filesystem_object_id)?;
        let now = Utc::now().naive_utc();
        let is_new = existing.is_none();
        let mut active = existing.map_or_else(
            || folders::ActiveModel {
                created_at: Set(now),
                ..Default::default()
            },
            IntoActiveModel::into_active_model,
        );
        active.name = Set(row.name);
        active.path = Set(row.path);
        active.parent_folder_id = Set(row.parent_folder_id);
        active.device_id = Set(device_id);
        active.inode = Set(inode);
        active.updated_at = Set(now);
        Ok(if is_new {
            active.insert(connection).await?
        } else {
            active.update(connection).await?
        })
    }

    /// Get file by path
    pub async fn get_file_by_path(&self, file_path: &Path) -> Result<Option<files::Model>> {
        let connection = self.database_manager.get_connection();
        Self::find_file(connection.as_ref(), &Self::database_path(file_path)?).await
    }

    // === BULK OPERATIONS FOR SCANNER ===

    /// Get directory state as a map for efficient comparison
    pub async fn get_database_state(
        &self,
        dir_path: &Path,
    ) -> Result<HashMap<PathBuf, PersistedFile>> {
        let connection = self.database_manager.get_connection();
        Files::find()
            .filter(Self::descendant_scope(files::Column::Path, dir_path)?)
            .all(&*connection)
            .await
            .context("Failed to get files from db")?
            .into_iter()
            .map(|file| {
                let file = PersistedFile::try_from(file)?;
                Ok((file.path.clone(), file))
            })
            .try_collect()
    }

    fn descendant_scope(column: impl IntoColumnRef, root: &Path) -> Result<SimpleExpr> {
        let root = Self::database_path(root)?;
        let prefix = format!("{}/", root.trim_end_matches('/'));
        // SQLite GLOB is case-sensitive; escape its metacharacters in literal paths.
        let escaped = prefix
            .replace('[', "[[]")
            .replace('*', "[*]")
            .replace('?', "[?]");
        Ok(Expr::col(column).binary(BinOper::Custom("GLOB"), format!("{escaped}*")))
    }

    pub async fn get_database_folder_state(
        &self,
        dir_path: &Path,
    ) -> Result<HashMap<PathBuf, folders::Model>> {
        let connection = self.database_manager.get_connection();
        let folder_state = Folders::find()
            .filter(Self::descendant_scope(folders::Column::Path, dir_path)?)
            .all(connection.as_ref())
            .await?
            .into_iter()
            .map(|folder| (PathBuf::from(&folder.path), folder))
            .collect();
        Ok(folder_state)
    }

    /// Batch insert/update files with transaction
    pub async fn batch_upsert_files(&self, files: Vec<File>) -> Result<UpsertFileBatchReport> {
        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        let mut file_inserted = 0;
        let mut file_updated = 0;
        let mut pending_types = HashMap::new();

        for file in files {
            let file_type_id = if let Some(&id) = pending_types.get(&file.file_type_name) {
                id
            } else {
                let id = self
                    .file_type_id(&file.file_type_name, &transaction)
                    .await?;
                pending_types.insert(file.file_type_name, id);
                id
            };
            let path = Self::database_path(&file.path)?;
            let existing = Self::find_file(&transaction, &path).await?;
            if existing.is_some() {
                file_updated += 1;
            } else {
                file_inserted += 1;
            }
            let row = FileRow {
                name: file.name,
                path,
                content_digest: file.content_digest,
                filesystem_object_id: file.filesystem_object_id,
                file_type_id,
            };
            Self::save_file(&transaction, existing, row).await?;
        }

        transaction.commit().await?;
        for (name, id) in pending_types {
            self.file_type_cache.insert(name, id);
        }
        Ok(UpsertFileBatchReport {
            file_inserted,
            file_updated,
        })
    }

    pub async fn batch_upsert_folders(
        &self,
        folders: Vec<Folder>,
    ) -> Result<UpsertFolderBatchReport> {
        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        let mut folder_inserted = 0;
        let mut folder_updated = 0;

        for folder in folders {
            let parent_folder_id = Self::find_parent_folder_id(&folder.path, &transaction).await?;
            let path = Self::database_path(&folder.path)?;
            let existing = Self::find_folder(&transaction, &path).await?;
            if existing.is_some() {
                folder_updated += 1;
            } else {
                folder_inserted += 1;
            }
            let row = FolderRow {
                name: folder.name,
                path,
                parent_folder_id,
                filesystem_object_id: folder.filesystem_object_id,
            };
            Self::save_folder(&transaction, existing, row).await?;
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

        let path_strings: Vec<_> = paths
            .iter()
            .map(|path| Self::database_path(path))
            .try_collect()?;

        let connection = self.database_manager.get_connection();
        let result = Files::delete_many()
            .filter(files::Column::Path.is_in(path_strings))
            .exec(&*connection)
            .await?;

        self.database_manager.invalidate_thumbnail_cache();
        Ok(result.rows_affected as usize)
    }

    /// Batch delete folders, and everything below them, by paths
    pub async fn batch_delete_folders(&self, paths: Vec<PathBuf>) -> Result<usize> {
        if paths.is_empty() {
            return Ok(0);
        }

        let mut scope = Condition::any();
        for path in paths {
            scope = scope
                .add(folders::Column::Path.eq(Self::database_path(&path)?))
                .add(Self::descendant_scope(folders::Column::Path, &path)?);
        }
        let connection = self.database_manager.get_connection();
        let transaction = connection.begin().await?;
        // Count before deletion: SQLite's rows_affected excludes cascaded descendants.
        let count = Folders::find()
            .filter(scope.clone())
            .count(&transaction)
            .await?;
        Folders::delete_many()
            .filter(scope)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;

        self.database_manager.invalidate_thumbnail_cache();
        Ok(usize::try_from(count)?)
    }

    /// Clear file type cache (useful for testing or cache invalidation)
    pub fn clear_file_type_cache(&self) {
        self.file_type_cache.invalidate_all();
    }

    /// Get or create a file type by name. The caller publishes the ID to the cache only after
    /// its transaction commits.
    async fn file_type_id<C>(&self, file_type_name: &str, connection: &C) -> Result<i32>
    where
        C: ConnectionTrait,
    {
        if let Some(type_id) = self.file_type_cache.get(file_type_name) {
            return Ok(type_id);
        }
        if let Some(existing_type) = FileTypes::find()
            .filter(file_types::Column::Name.eq(file_type_name))
            .one(connection)
            .await?
        {
            return Ok(existing_type.id);
        }
        let created_type = file_types::ActiveModel {
            name: Set(file_type_name.to_string()),
            ..Default::default()
        }
        .insert(connection)
        .await?;
        Ok(created_type.id)
    }

    /// Preload common file types into cache
    pub async fn preload_file_type_cache(&self) -> Result<()> {
        let connection = self.database_manager.get_connection();
        let all_types = FileTypes::find().all(&*connection).await?;

        for file_type in all_types {
            self.file_type_cache.insert(file_type.name, file_type.id);
        }

        Ok(())
    }
}
