use anyhow::Result;
use migration::{Migrator, MigratorTrait};
use model::services::{CanonPath, file::FileSystemFile};
use repositories::{
    config::DatabaseSettings, fs::operations::FileRepository, manager::DatabaseManager,
};
use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use services::fs::scanner::{DirectoryScanner, ScanConfig};
use std::sync::Arc;

#[tokio::test]
async fn root_snapshots_preserve_files_and_scope_deletions() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("photos_%");
    let sibling = directory.path().join("photos_%-backup");
    let wildcard_sibling = directory.path().join("photos_other");
    for path in [&root, &sibling, &wildcard_sibling] {
        tokio::fs::create_dir_all(path.join("nested/deep")).await?;
        tokio::fs::write(path.join("nested/deep/image.txt"), b"original").await?;
    }
    let root = CanonPath::try_from(root)?;
    let sibling = CanonPath::try_from(sibling)?;
    let wildcard_sibling = CanonPath::try_from(wildcard_sibling)?;
    let database = database().await?;
    let repository = Arc::new(FileRepository::new(database));
    repository
        .upsert_root_folders(vec![
            root.clone(),
            sibling.clone(),
            wildcard_sibling.clone(),
        ])
        .await?;
    let scanner = DirectoryScanner::new_with_config(
        Arc::clone(&repository),
        ScanConfig {
            batch_size: 1,
            ..ScanConfig::default()
        },
    );
    scanner.sync_directory(&sibling).await?;
    scanner.sync_directory(&wildcard_sibling).await?;
    let first = scanner.sync_directory(&root).await?;
    assert_eq!(
        (
            first.files_inserted,
            first.files_updated,
            first.folders_inserted,
            first.folders_updated
        ),
        (1, 0, 2, 0)
    );
    let second = scanner.sync_directory(&root).await?;
    assert_eq!(
        (
            second.files_inserted,
            second.files_updated,
            second.files_deleted,
            second.folders_inserted,
            second.folders_updated,
            second.folders_deleted
        ),
        (0, 0, 0, 0, 0, 0)
    );
    assert_eq!(repository.get_database_state(root.as_ref()).await?.len(), 1);

    tokio::fs::write(root.as_ref().join("nested/deep/image.txt"), b"changed").await?;
    let changed = scanner.sync_directory(&root).await?;
    assert_eq!((changed.files_inserted, changed.files_updated), (0, 1));
    tokio::fs::remove_dir_all(root.as_ref().join("nested")).await?;
    let deleted = scanner.sync_directory(&root).await?;
    assert_eq!((deleted.files_deleted, deleted.folders_deleted), (1, 2));
    assert!(
        repository
            .get_database_state(root.as_ref())
            .await?
            .is_empty()
    );
    assert!(
        repository
            .get_database_folder_state(root.as_ref())
            .await?
            .is_empty()
    );
    for sibling in [&sibling, &wildcard_sibling] {
        assert_eq!(
            repository.get_database_state(sibling.as_ref()).await?.len(),
            1
        );
        assert_eq!(
            repository
                .get_database_folder_state(sibling.as_ref())
                .await?
                .len(),
            2
        );
    }
    Ok(())
}

#[tokio::test]
async fn failed_batches_and_scans_return_errors() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = CanonPath::try_from(directory.path().to_path_buf())?;
    let path = root.as_ref().join("image.txt");
    tokio::fs::write(&path, b"original").await?;
    let database = database().await?;
    let repository = Arc::new(FileRepository::new(Arc::clone(&database)));
    repository.upsert_root_folders(vec![root.clone()]).await?;
    let scanner = DirectoryScanner::new(Arc::clone(&repository));
    database.get_connection().execute(Statement::from_string(DatabaseBackend::Sqlite,
        "CREATE TRIGGER reject_insert BEFORE INSERT ON files BEGIN SELECT RAISE(ABORT, 'injected batch failure'); END;".to_string())).await?;
    assert!(scanner.sync_directory(&root).await.is_err());
    assert!(
        repository
            .get_database_state(root.as_ref())
            .await?
            .is_empty()
    );
    database
        .get_connection()
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "DROP TRIGGER reject_insert".to_string(),
        ))
        .await?;
    repository
        .batch_upsert_files(vec![
            FileSystemFile::create_file_info_from_path(&path).await?,
        ])
        .await?;
    tokio::fs::remove_file(&path).await?;
    tokio::fs::remove_dir(root.as_ref()).await?;
    assert!(scanner.sync_directory(&root).await.is_err());
    assert_eq!(repository.get_database_state(root.as_ref()).await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn folder_batches_count_updates_and_cascaded_deletions() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root_path = directory.path().join("literal[*?]_%");
    tokio::fs::create_dir_all(root_path.join("nested/deep")).await?;
    let root = CanonPath::try_from(root_path)?;
    let repository = Arc::new(FileRepository::new(database().await?));
    repository.upsert_root_folders(vec![root.clone()]).await?;
    let scanner = DirectoryScanner::new(Arc::clone(&repository));
    scanner.sync_directory(&root).await?;
    let mut folder = model::services::folder::FileSystemFolder::create_folder_info(
        &root.as_ref().join("nested"),
    )
    .await?;
    folder.name = "stale name".to_string();
    let batch = repository.batch_upsert_folders(vec![folder]).await?;
    assert_eq!((batch.folder_inserted, batch.folder_updated), (0, 1));
    let report = scanner.sync_directory(&root).await?;
    assert_eq!((report.folders_inserted, report.folders_updated), (0, 1));
    assert_eq!(
        repository
            .batch_delete_folders(vec![root.as_ref().join("nested")])
            .await?,
        2
    );
    assert!(
        repository
            .get_database_folder_state(root.as_ref())
            .await?
            .is_empty()
    );
    Ok(())
}

async fn database() -> Result<Arc<DatabaseManager>> {
    let database = Arc::new(
        DatabaseManager::new(DatabaseSettings::new(
            "sqlite::memory:".to_string(),
            30_000,
            SqliteJournalMode::Memory,
            SqliteSynchronous::Normal,
        ))
        .await?,
    );
    Migrator::up(database.get_connection().as_ref(), None).await?;
    Ok(database)
}
