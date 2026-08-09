use anyhow::Result;
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
    pub async fn sync_directory(&self, dir_path: &Path) -> Result<SyncReport> {
        let start_time = Instant::now();
        let mut report = SyncReport::new();

        tracing::info!("Starting directory sync for: {}", dir_path.display());

        // 1. Get current database state
        let db_state = match self.file_operations.get_directory_state(dir_path).await {
            Ok(state) => state,
            Err(e) => {
                let error_msg = format!("Failed to get database state: {e:?}");
                report.errors.push(error_msg.clone());
                return Err(e);
            }
        };

        tracing::info!("Found {} files in database", db_state.len());

        // 2. Scan filesystem
        let fs_files = match self.scan_filesystem_recursive(dir_path).await {
            Ok(files) => files,
            Err(e) => {
                let error_msg = format!("Failed to scan filesystem: {e:?}");
                report.errors.push(error_msg);
                return Err(e);
            }
        };

        report.files_scanned = fs_files.0.len();
        tracing::info!("Found {} files in filesystem", fs_files.0.len());

        report.folders_scanned = fs_files.1.len();
        tracing::info!("Found {} folders in filesystem", fs_files.1.len());

        // 3a. Calculate file sync operations
        let mut operations: Vec<SyncOperation> =
            Self::calculate_file_sync_operations(&db_state, fs_files.0);
        tracing::info!("Calculated {} file operations to perform", operations.len());

        // 3b. Calculate all sync operations
        operations.extend(Self::calculate_folder_sync_operations(
            &db_state, fs_files.1,
        ));
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
                SyncOperation::InsertFile(file_info) => {
                    upsert_file_batch.push(file_info);
                    if upsert_file_batch.len() >= self.config.batch_size {
                        self.execute_upsert_file_batch(&mut upsert_file_batch, &mut report)
                            .await;
                    }
                }
                SyncOperation::InsertFolder(folder_info) => {
                    upsert_folder_batch.push(folder_info);
                    if upsert_folder_batch.len() >= self.config.batch_size {
                        self.execute_upsert_folder_batch(&mut upsert_folder_batch, &mut report)
                            .await;
                    }
                }
                SyncOperation::UpdateFile(file_info) => {
                    upsert_file_batch.push(file_info); // Upsert handles both insert and update
                    if upsert_file_batch.len() >= self.config.batch_size {
                        self.execute_upsert_file_batch(&mut upsert_file_batch, &mut report)
                            .await;
                    }
                }
                SyncOperation::UpdateFolder(folder_info) => {
                    upsert_folder_batch.push(folder_info); // Upsert handles both insert and update
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
        if !upsert_file_batch.is_empty() {
            self.execute_upsert_file_batch(&mut upsert_file_batch, &mut report)
                .await;
        }

        if !upsert_folder_batch.is_empty() {
            self.execute_upsert_folder_batch(&mut upsert_folder_batch, &mut report)
                .await;
        }

        if !delete_file_batch.is_empty() {
            self.execute_delete_file_batch(&mut delete_file_batch, &mut report)
                .await;
        }

        if !delete_folder_batch.is_empty() {
            self.execute_delete_folder_batch(&mut delete_file_batch, &mut report)
                .await;
        }
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
        let mut entries = match fs::read_dir(dir_path).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::error!("Failed to read directory {}: {}", dir_path.display(), e);
                return Err(e)?; // Continue with other directories
            }
        };

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

                //TODO: Also need to add all of the folders, recurse into them and add all folders
                //into database.
                match Folder::create_folder_info(&path).await {
                    Ok(folder_info) => folders.push(folder_info),
                    Err(e) => {
                        return Err(e);
                    }
                }

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
                match File::create_file_info_from_path(&path).await {
                    Ok(file_info) => files.push(file_info),
                    Err(e) => {
                        tracing::error!("Failed to process file {}: {:?}", path.display(), e);
                        // Continue with other files
                    }
                }
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
                    if db_metadata.content_hash != fs_file.content_hash
                        || db_metadata.identity_hash != fs_file.identity_hash
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
                Some(db_metadata) => {
                    // File exists in database, check if it needs updating
                    if db_metadata.content_hash != fs_folder.content_hash
                        || db_metadata.identity_hash != fs_folder.identity_hash
                    {
                        operations.push(SyncOperation::UpdateFolder(fs_folder));
                    }
                    // If hashes match, no operation needed
                }
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
        if batch.is_empty() {
            return;
        }

        match self.file_operations.batch_upsert_files(batch.clone()).await {
            Ok(file_report) => {
                report.files_inserted += file_report.file_inserted; // Note: this includes both inserts and updates
                report.files_updated += file_report.file_updated; // Note: this includes both inserts and updates
                tracing::info!(
                    "Successfully processed batch of {} files",
                    file_report.file_inserted + file_report.file_updated
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
