use anyhow::{Context, Result, ensure};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tokio::fs;

use model::services::file::FileSystemFile as File;
use model::services::folder::FileSystemFolder as Folder;
use repositories::fs::operations::FileRepository as FileOperations;

/// Report of synchronization operations
#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub files_scanned: usize,
    pub files_inserted: usize,
    pub files_updated: usize,
    pub files_deleted: usize,
    pub folders_scanned: usize,
    pub folders_inserted: usize,
    pub folders_updated: usize,
    pub folders_deleted: usize,
    pub errors: Vec<String>,
    pub duration: std::time::Duration,
}

impl SyncReport {
    /// Number of inserted, updated, or deleted files and folders.
    #[must_use]
    pub fn changed(&self) -> usize {
        self.files_inserted
            + self.files_updated
            + self.files_deleted
            + self.folders_inserted
            + self.folders_updated
            + self.folders_deleted
    }
}

/// Database changes needed to make the database match the filesystem.
#[derive(Debug, Default)]
struct SyncPlan<T> {
    upserts: Vec<T>,
    deletes: Vec<PathBuf>,
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
        Self::new_with_config(file_operations, ScanConfig::default())
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
    pub async fn sync_directory(
        &self,
        dir_path: &model::services::CanonPath,
    ) -> Result<SyncReport> {
        let dir_path = dir_path.as_ref();
        let start_time = Instant::now();
        let mut report = SyncReport::default();

        tracing::info!("Starting directory sync for: {}", dir_path.display());

        let db_files = self
            .file_operations
            .get_database_state(dir_path)
            .await
            .context("failed to get database state")?;
        let db_folders = self
            .file_operations
            .get_database_folder_state(dir_path)
            .await?;
        tracing::info!("Found {} files in database", db_files.len());

        let (files, folders) = self
            .scan_filesystem_recursive(dir_path)
            .await
            .context("Failed to scan filesystem state")?;
        report.files_scanned = files.len();
        report.folders_scanned = folders.len();
        tracing::info!(
            "Found {} files and {} folders in filesystem",
            files.len(),
            folders.len()
        );

        let file_plan = Self::plan(&db_files, files, |db_file, file: &File| {
            db_file.content_digest != file.content_digest
                || db_file.filesystem_object_id != file.filesystem_object_id
        });
        let folder_plan = Self::plan(&db_folders, folders, |db_folder, folder: &Folder| {
            db_folder.name != folder.name
                || u64::try_from(db_folder.device_id).ok()
                    != Some(folder.filesystem_object_id.device)
                || u64::try_from(db_folder.inode).ok() != Some(folder.filesystem_object_id.inode)
        });

        let batch_size = self.config.batch_size.max(1);
        let repository = &self.file_operations;
        for batch in file_plan.upserts.chunks(batch_size) {
            match repository.batch_upsert_files(batch.to_vec()).await {
                Ok(result) => {
                    report.files_inserted += result.file_inserted;
                    report.files_updated += result.file_updated;
                }
                Err(error) => report.errors.push(error.to_string()),
            }
        }
        // Folders are upserted in scan order, so parents are stored before their children.
        for batch in folder_plan.upserts.chunks(batch_size) {
            match repository.batch_upsert_folders(batch.to_vec()).await {
                Ok(result) => {
                    report.folders_inserted += result.folder_inserted;
                    report.folders_updated += result.folder_updated;
                }
                Err(error) => report.errors.push(error.to_string()),
            }
        }
        for batch in file_plan.deletes.chunks(batch_size) {
            match repository.batch_delete_files(batch.to_vec()).await {
                Ok(count) => report.files_deleted += count,
                Err(error) => report.errors.push(error.to_string()),
            }
        }
        for batch in folder_plan.deletes.chunks(batch_size) {
            match repository.batch_delete_folders(batch.to_vec()).await {
                Ok(count) => report.folders_deleted += count,
                Err(error) => report.errors.push(error.to_string()),
            }
        }
        report.duration = start_time.elapsed();

        ensure!(
            report.errors.is_empty(),
            "Directory sync failed: {}",
            report.errors.join("; ")
        );
        tracing::info!(
            duration = ?report.duration,
            files_inserted = report.files_inserted,
            files_updated = report.files_updated,
            files_deleted = report.files_deleted,
            folders_inserted = report.folders_inserted,
            folders_updated = report.folders_updated,
            folders_deleted = report.folders_deleted,
            "Directory sync completed"
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

        while let Some(entry) = entries
            .next_entry()
            .await
            .context("Could not read directory entry")?
        {
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
                    Box::pin(self.scan_directory_impl(&path, files, folders)).await?;
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

    /// Upsert new or changed scanned entries and delete stored entries that were not scanned.
    fn plan<T: HasPath, Stored>(
        db_state: &HashMap<PathBuf, Stored>,
        scanned: Vec<T>,
        changed: impl Fn(&Stored, &T) -> bool,
    ) -> SyncPlan<T> {
        let scanned_paths: HashSet<PathBuf> = scanned
            .iter()
            .map(|entry| entry.path().to_owned())
            .collect();
        let upserts = scanned
            .into_iter()
            .filter(|entry| {
                db_state
                    .get(entry.path())
                    .is_none_or(|stored| changed(stored, entry))
            })
            .collect();
        let deletes = db_state
            .keys()
            .filter(|path| !scanned_paths.contains(*path))
            .cloned()
            .collect();
        SyncPlan { upserts, deletes }
    }
}

trait HasPath {
    fn path(&self) -> &Path;
}

impl HasPath for File {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl HasPath for Folder {
    fn path(&self) -> &Path {
        &self.path
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
