//! Filesystem IO operations for library management
//!
//! This module handles all filesystem interactions including:
//! - Reading and writing configuration files
//! - Directory creation and validation
//! - Database file management
//! - Last library path tracking

use anyhow::{Context, Result, bail};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Read the content of a file as a String
pub fn read_file_to_string(path: &Path) -> Result<String> {
    let mut content = String::new();
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("failed to open {} for reading", path.display()))?;
    file.read_to_string(&mut content)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(content)
}

/// Write string content to a file, creating it if it doesn't exist
pub fn write_string_to_file(path: &Path, content: &str) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("failed to open {} for writing", path.display()))?;
    file.write_all(content.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync {}", path.display()))?;
    Ok(())
}

/// Create a directory and all its parent directories if they don't exist
pub fn ensure_directory_exists(path: &Path) -> Result<()> {
    if !path
        .try_exists()
        .with_context(|| format!("failed to inspect directory {}", path.display()))?
    {
        fs::create_dir_all(path)
            .with_context(|| format!("failed to create directory {}", path.display()))?;
    }
    Ok(())
}

/// Create or validate that the data directory exists and return its path
pub fn create_or_validate_data_directory() -> Result<PathBuf> {
    let datahome = dirs::data_dir().context("operating system did not provide a data directory")?;

    ensure_directory_exists(&datahome)?;

    Ok(datahome)
}

/// Ensure a file exists at the given path, creating it with default content if needed
/// Returns the path to the file
pub fn ensure_file_exists(path: &Path, default_content: &str) -> Result<PathBuf> {
    if fs::exists(path).with_context(|| format!("failed to inspect file {}", path.display()))? {
        return Ok(path.to_path_buf());
    }

    write_string_to_file(path, default_content)?;
    Ok(path.to_path_buf())
}

/// Create or open a database file at the given path
pub fn ensure_database_file(share_path: &Path) -> Result<PathBuf> {
    let db_path = share_path.join("db.sqlite");
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&db_path)
        .with_context(|| format!("failed to create database file {}", db_path.display()))?;
    Ok(db_path)
}

/// Delete a directory and all its contents
pub fn delete_directory(path: &Path) -> Result<()> {
    if !fs::exists(path)
        .with_context(|| format!("failed to inspect directory {}", path.display()))?
    {
        bail!("cannot delete missing directory {}", path.display());
    }

    fs::remove_dir_all(path)
        .with_context(|| format!("failed to delete directory {}", path.display()))
}

/// List all entries in a directory and return their paths as strings
pub fn list_directory_entries(path: &Path) -> Result<Vec<String>> {
    fs::read_dir(path)
        .with_context(|| format!("failed to list directory {}", path.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.path().to_string_lossy().to_string())
                .with_context(|| format!("failed to read an entry in {}", path.display()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_dir() -> Result<TempDir> {
        Ok(TempDir::new()?)
    }

    #[test]
    fn test_write_and_read_file() -> Result<()> {
        let temp_dir = setup_test_dir()?;
        let file_path = temp_dir.path().join("test.txt");
        let content = "Hello, World!";

        write_string_to_file(&file_path, content)?;
        let read_content = read_file_to_string(&file_path)?;

        assert_eq!(content, read_content);
        Ok(())
    }

    #[test]
    fn test_ensure_directory_exists_creates_new() -> Result<()> {
        let temp_dir = setup_test_dir()?;
        let new_dir = temp_dir.path().join("new_directory");

        assert!(!new_dir.exists());
        ensure_directory_exists(&new_dir)?;
        assert!(new_dir.exists());
        assert!(new_dir.is_dir());

        Ok(())
    }

    #[test]
    fn test_ensure_directory_exists_with_existing() -> Result<()> {
        let temp_dir = setup_test_dir()?;
        let existing_dir = temp_dir.path();

        // Should not error on existing directory
        ensure_directory_exists(existing_dir)?;
        assert!(existing_dir.exists());

        Ok(())
    }

    #[test]
    fn test_ensure_file_exists_creates_new() -> Result<()> {
        let temp_dir = setup_test_dir()?;
        let file_path = temp_dir.path().join("new_file.txt");
        let default_content = "default content";

        assert!(!file_path.exists());
        let result_path = ensure_file_exists(&file_path, default_content)?;

        assert!(file_path.exists());
        assert_eq!(result_path, file_path);

        let content = read_file_to_string(&file_path)?;
        assert_eq!(content, default_content);

        Ok(())
    }

    #[test]
    fn test_ensure_file_exists_with_existing() -> Result<()> {
        let temp_dir = setup_test_dir()?;
        let file_path = temp_dir.path().join("existing_file.txt");
        let original_content = "original";

        write_string_to_file(&file_path, original_content)?;

        let result_path = ensure_file_exists(&file_path, "default")?;

        assert_eq!(result_path, file_path);
        let content = read_file_to_string(&file_path)?;
        assert_eq!(content, original_content);

        Ok(())
    }

    #[test]
    fn test_ensure_database_file() -> Result<()> {
        let temp_dir = setup_test_dir()?;

        let db_path = ensure_database_file(temp_dir.path())?;

        assert!(db_path.exists());
        assert_eq!(db_path, temp_dir.path().join("db.sqlite"));

        Ok(())
    }

    #[test]
    fn test_delete_directory() -> Result<()> {
        let temp_dir = setup_test_dir()?;
        let dir_to_delete = temp_dir.path().join("to_delete");

        ensure_directory_exists(&dir_to_delete)?;
        assert!(dir_to_delete.exists());

        delete_directory(&dir_to_delete)?;
        assert!(!dir_to_delete.exists());

        Ok(())
    }

    #[test]
    fn test_delete_nonexistent_directory_reports_path() -> Result<()> {
        let temp_dir = setup_test_dir()?;
        let nonexistent = temp_dir.path().join("does_not_exist");

        let Err(error) = delete_directory(&nonexistent) else {
            bail!("missing directory should fail");
        };

        assert!(
            error
                .to_string()
                .contains(&nonexistent.display().to_string())
        );
        Ok(())
    }

    #[test]
    fn test_list_directory_entries() -> Result<()> {
        let temp_dir = setup_test_dir()?;

        // Create some test files and directories
        let file1 = temp_dir.path().join("file1.txt");
        let file2 = temp_dir.path().join("file2.txt");
        let subdir = temp_dir.path().join("subdir");

        write_string_to_file(&file1, "content1")?;
        write_string_to_file(&file2, "content2")?;
        ensure_directory_exists(&subdir)?;

        let entries = list_directory_entries(temp_dir.path())?;

        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|e| e.contains("file1.txt")));
        assert!(entries.iter().any(|e| e.contains("file2.txt")));
        assert!(entries.iter().any(|e| e.contains("subdir")));

        Ok(())
    }

    #[test]
    fn test_write_overwrites_existing_file() -> Result<()> {
        let temp_dir = setup_test_dir()?;
        let file_path = temp_dir.path().join("overwrite.txt");

        write_string_to_file(&file_path, "original")?;
        write_string_to_file(&file_path, "new content")?;

        let content = read_file_to_string(&file_path)?;
        assert_eq!(content, "new content");

        Ok(())
    }
}
