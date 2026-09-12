use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use async_recursion::async_recursion;
use tokio::fs;

use model::services::file::FileSystemFile as File;
use model::services::folder::FileSystemFolder as Folder;
use repositories::fs::operations::{FileMetadata, FileRepository as FileOperations};

/// Types of synchronization operations
#[derive(Debug, Clone)]
pub enum SyncOperation {
    /// Insert a new file into the database
    InsertFile(File),
    InsertFolder(Folder),
    /// Update an existing file in the database
    UpdateFile(File),
    UpdateFolder(Folder),
    /// Delete a file from the database (file no longer exists)
    DeleteFile(PathBuf),
    DeleteFolder(PathBuf),
}

/// Report of synchronization operations
#[derive(Debug, Clone)]
pub struct SyncReport {
    pub files_scanned: usize,
    pub files_inserted: usize,
    pub files_updated: usize,
    pub files_deleted: usize,
    pub files_skipped: usize,
    pub folders_scanned: usize,
    pub folders_inserted: usize,
    pub folders_updated: usize,
    pub folders_deleted: usize,
    pub folders_skipped: usize,
    pub errors: Vec<String>,
    pub duration: std::time::Duration,
}

impl Default for SyncReport {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncReport {
    #[must_use]
    pub fn new() -> Self {
        Self {
            files_scanned: 0,
            files_inserted: 0,
            files_updated: 0,
            files_deleted: 0,
            files_skipped: 0,
            folders_scanned: 0,
            folders_inserted: 0,
            folders_updated: 0,
            folders_deleted: 0,
            folders_skipped: 0,
            errors: Vec::new(),
            duration: std::time::Duration::from_secs(0),
        }
    }

    #[must_use]
    pub fn total_operations(&self) -> usize {
        self.files_inserted + self.files_updated + self.files_deleted
    }
}

/// Directory scanner that synchronizes filesystem state with database
#[derive(Debug)]
pub struct DirectoryScanner {
    file_operations: Arc<FileOperations>,
    config: ScanConfig,
}

impl DirectoryScanner {
    /// Create a new directory scanner
    #[must_use]
    pub fn new(file_operations: Arc<FileOperations>) -> Self {
        Self {
            file_operations,
            config: ScanConfig::default(),
        }
    }

    /// Create a new directory scanner with custom configuration
    #[must_use]
    pub fn new_with_config(file_operations: Arc<FileOperations>, config: ScanConfig) -> Self {
        Self {
            file_operations,
            config,
        }
    }

    /// Synchronize a directory with the database
    #[expect(
        clippy::too_many_lines,
        reason = "the linear synchronization workflow is clearer in one place"
    )]
    pub async fn sync_directory(
        &self,
        dir_path: &model::services::CanonPath,
    ) -> Result<SyncReport> {
        let dir_path = dir_path.as_ref();
        let start_time = Instant::now();
        let mut report = SyncReport::new();

        tracing::info!("Starting directory sync for: {}", dir_path.display());

        let db_state = self
            .file_operations
            .get_database_state(dir_path)
            .await
            .inspect_err(|error| {
                report
                    .errors
                    .push(format!("failed to get database state: {error}"));
            })?;

        tracing::info!("Found {} files in database", db_state.len());

        let (files, folders) =
            self.scan_filesystem_recursive(dir_path)
                .await
                .inspect_err(|error| {
                    report
                        .errors
                        .push(format!("Failed to scan filesystem state: {error}"));
                })?;

        report.files_scanned = files.len();
        tracing::info!("Found {} files in filesystem", files.len());

        report.folders_scanned = folders.len();
        tracing::info!("Found {} folders in filesystem", folders.len());

        // 3a. Calculate file sync operations
        let mut operations: Vec<SyncOperation> =
            Self::calculate_file_sync_operations(&db_state, files);
        tracing::info!("Calculated {} file operations to perform", operations.len());

        // 3b. Calculate all sync operations
        operations.extend(Self::calculate_folder_sync_operations(&db_state, folders));
        tracing::info!(
            "Calculated {} file and folder operations to perform",
            operations.len()
        );

        // 4. Execute operations in batches
        let mut upsert_file_batch = Vec::new();
        let mut delete_file_batch = Vec::new();

        let mut upsert_folder_batch = Vec::new();
        let mut delete_folder_batch = Vec::new();

        //TODO: Split up into files and folders, may need to implement trait dependency injection
        //to make it more ergonomic in the future
        for operation in operations {
            match operation {
                SyncOperation::InsertFile(file_info) | SyncOperation::UpdateFile(file_info) => {
                    upsert_file_batch.push(file_info);
                    if upsert_file_batch.len() >= self.config.batch_size {
                        self.execute_upsert_file_batch(&mut upsert_file_batch, &mut report)
                            .await;
                    }
                }
                SyncOperation::InsertFolder(folder_info)
                | SyncOperation::UpdateFolder(folder_info) => {
                    upsert_folder_batch.push(folder_info);
                    if upsert_folder_batch.len() >= self.config.batch_size {
                        self.execute_upsert_folder_batch(&mut upsert_folder_batch, &mut report)
                            .await;
                    }
                }
                SyncOperation::DeleteFile(path) => {
                    delete_file_batch.push(path);
                    if delete_file_batch.len() >= self.config.batch_size {
                        self.execute_delete_file_batch(&mut delete_file_batch, &mut report)
                            .await;
                    }
                }
                SyncOperation::DeleteFolder(path) => {
                    delete_folder_batch.push(path);
                    if delete_folder_batch.len() >= self.config.batch_size {
                        self.execute_delete_folder_batch(&mut delete_folder_batch, &mut report)
                            .await;
                    }
                }
            }
        }

        // Execute remaining batches
        self.execute_upsert_file_batch(upsert_file_batch, &mut report)
            .await;
        self.execute_upsert_folder_batch(&mut upsert_folder_batch, &mut report)
            .await;
        self.execute_delete_file_batch(&mut delete_file_batch, &mut report)
            .await;
        self.execute_delete_folder_batch(&mut delete_folder_batch, &mut report)
            .await;
        report.duration = start_time.elapsed();

        //NOTE: This could be removed in the future if I do not find any worth in it
        println!("Directory sync completed in {:?}", report.duration);
        println!(
            r"Results: 
            {} files inserted,
            {} folders inserted,
            {} files updated, 
            {} folders updated, 
            {} files deleted, 
            {} folders deleted, 
            {} errors",
            report.files_inserted,
            report.folders_inserted,
            report.files_updated,
            report.folders_updated,
            report.files_deleted,
            report.folders_deleted,
            report.errors.len()
        );

        Ok(report)
    }

    /// Scan filesystem recursively and return file information
    async fn scan_filesystem_recursive(&self, dir_path: &Path) -> Result<(Vec<File>, Vec<Folder>)> {
        let mut files = Vec::new();
        let mut folders = Vec::new();
        self.scan_directory_impl(dir_path, &mut files, &mut folders)
            .await?;
        Ok((files, folders))
    }

    /// Recursive implementation of directory scanning
    #[async_recursion]
    async fn scan_directory_impl(
        &self,
        dir_path: &Path,
        files: &mut Vec<File>,
        folders: &mut Vec<Folder>,
    ) -> Result<()> {
        //TODO: Also need to add the root directory, which then could be one of the only ones, that
        //does not have a parent_folder_id
        let mut entries = fs::read_dir(dir_path)
            .await
            .context("Could not read directory")?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();

            if path.is_dir() {
                // Check if directory should be ignored
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str())
                    && self
                        .config
                        .ignore_directories
                        .contains(&dir_name.to_string())
                {
                    continue;
                }

                let folder_info = Folder::create_folder_info(&path)
                    .await
                    .context("Could not create folder info")?;
                folders.push(folder_info);

                // Recurse into subdirectory if configured
                if self.config.recursive {
                    self.scan_directory_impl(&path, files, folders).await?;
                }
            } else if path.is_file() {
                // Check if file should be ignored
                if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
                    let ext_with_dot = format!(".{extension}");
                    if self.config.ignore_extensions.contains(&ext_with_dot) {
                        continue;
                    }
                }

                // Check file size if limit is set
                if let Some(max_size) = self.config.max_file_size
                    && let Ok(metadata) = fs::metadata(&path).await
                    && metadata.len() > max_size
                {
                    continue;
                }

                // Process the file
                let file_info = File::create_file_info_from_path(&path)
                    .await
                    .context("Could not create file info")?;
                files.push(file_info);
            }
        }
        Ok(())
    }

    /// Calculate what operations need to be performed
    fn calculate_file_sync_operations(
        db_state: &HashMap<PathBuf, FileMetadata>,
        fs_files: Vec<File>,
    ) -> Vec<SyncOperation> {
        let mut operations = Vec::new();
        let mut processed_paths = std::collections::HashSet::new();

        // Check filesystem files against database
        for fs_file in fs_files {
            processed_paths.insert(fs_file.path.clone());

            match db_state.get(&fs_file.path) {
                Some(db_metadata) => {
                    // File exists in database, check if it needs updating
                    if db_metadata.content_digest != fs_file.content_digest
                        || db_metadata.filesystem_object_id != fs_file.filesystem_object_id
                    {
                        operations.push(SyncOperation::UpdateFile(fs_file));
                    }
                    // If hashes match, no operation needed
                }
                None => {
                    // File doesn't exist in database, insert it
                    operations.push(SyncOperation::InsertFile(fs_file));
                }
            }
        }

        // Check for files in database that no longer exist in filesystem
        for db_path in db_state.keys() {
            if !processed_paths.contains(db_path) {
                operations.push(SyncOperation::DeleteFile(db_path.to_owned()));
            }
        }
        operations
    }

    fn calculate_folder_sync_operations(
        db_state: &HashMap<PathBuf, FileMetadata>,
        fs_folders: Vec<Folder>,
    ) -> Vec<SyncOperation> {
        let mut operations = Vec::new();
        let mut processed_paths = std::collections::HashSet::new();

        // Check filesystem files against database
        for fs_folder in fs_folders {
            processed_paths.insert(fs_folder.path.clone());

            match db_state.get(&fs_folder.path) {
                Some(_) => operations.push(SyncOperation::UpdateFolder(fs_folder)),
                None => {
                    // File doesn't exist in database, insert it
                    operations.push(SyncOperation::InsertFolder(fs_folder));
                }
            }
        }

        // Check for files in database that no longer exist in filesystem
        for db_path in db_state.keys() {
            if !processed_paths.contains(db_path) {
                operations.push(SyncOperation::DeleteFile(db_path.to_owned()));
            }
        }
        operations
    }

    /// Execute a batch of insert/update operations
    async fn execute_upsert_file_batch(&self, batch: &mut Vec<File>, report: &mut SyncReport) {
        self.file_operations
            .batch_upsert_files(batch.clone())
            .await
            .inspect(|x| {
                report.files_inserted += x.file_inserted;
                report.files_updated += x.file_updated;
            })
            .inspect_err(|error| {
                report.errors.push(error.to_string());
            });
        batch.clear();
    }

    async fn execute_upsert_folder_batch(&self, batch: &mut Vec<Folder>, report: &mut SyncReport) {
        if batch.is_empty() {
            return;
        }

        match self
            .file_operations
            .batch_upsert_folders(batch.clone())
            .await
        {
            Ok(folder_report) => {
                report.folders_inserted += folder_report.folder_inserted;
                report.folders_updated += folder_report.folder_updated;
                tracing::info!(
                    "Successfully processed batch of {} files",
                    folder_report.folder_inserted + folder_report.folder_updated
                );
            }
            Err(e) => {
                let error_msg = format!("Failed to execute insert batch: {e:?}");
                report.errors.push(error_msg);
                tracing::error!("Batch insert failed: {:?}", e);
            }
        }

        batch.clear();
    }

    /// Execute a batch of delete operations
    async fn execute_delete_file_batch(&self, batch: &mut Vec<PathBuf>, report: &mut SyncReport) {
        if batch.is_empty() {
            return;
        }

        match self.file_operations.batch_delete_files(batch.clone()).await {
            Ok(count) => {
                report.files_deleted += count;
                tracing::info!("Successfully deleted {} files from database", count);
            }
            Err(e) => {
                let error_msg = format!("Failed to execute delete batch: {e:?}");
                report.errors.push(error_msg);
                tracing::error!("Batch delete failed: {:?}", e);
            }
        }
        batch.clear();
    }

    /// Execute a batch of delete operations
    async fn execute_delete_folder_batch(&self, batch: &mut Vec<PathBuf>, report: &mut SyncReport) {
        if batch.is_empty() {
            return;
        }

        match self
            .file_operations
            .batch_delete_folders(batch.clone())
            .await
        {
            Ok(count) => {
                report.folders_deleted += count;
                tracing::info!("Successfully deleted {} files from database", count);
            }
            Err(e) => {
                let error_msg = format!("Failed to execute delete batch: {e:?}");
                report.errors.push(error_msg);
                tracing::error!("Batch delete failed: {:?}", e);
            }
        }
        batch.clear();
    }
}

/// Configuration for directory scanning
#[derive(Debug, Clone)]
pub struct ScanConfig {
    /// Maximum number of files to process in a single batch
    pub batch_size: usize,
    /// Whether to scan subdirectories recursively
    pub recursive: bool,
    /// File extensions to ignore (e.g., `.tmp` and `.log`)
    pub ignore_extensions: Vec<String>,
    /// Directory names to ignore (e.g., `.git` and `node_modules`)
    pub ignore_directories: Vec<String>,
    /// Maximum file size to process (in bytes)
    pub max_file_size: Option<u64>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            batch_size: 100,
            recursive: true,
            ignore_extensions: vec![
                ".tmp".to_string(),
                ".log".to_string(),
                ".bak".to_string(),
                ".swp".to_string(),
            ],
            ignore_directories: vec![
                ".git".to_string(),
                ".svn".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
                ".DS_Store".to_string(),
            ],
            max_file_size: Some(100 * 1024 * 1024), // 100 MB
        }
    }
}
