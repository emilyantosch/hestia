use anyhow::{Context, Result};
use migration::{Migrator, MigratorTrait};
use model::services::file::FileSystemFile;
use repositories::config::DatabaseSettings;
use repositories::fs::operations::FileRepository;
use repositories::manager::DatabaseManager;
use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use std::sync::Arc;

#[tokio::test]
async fn equal_content_and_hard_links_remain_separate_file_entries() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let original = directory.path().join("original.txt");
    let copy = directory.path().join("copy.txt");
    let hard_link = directory.path().join("hard-link.txt");
    tokio::fs::write(&original, b"same bytes").await?;
    tokio::fs::copy(&original, &copy).await?;
    tokio::fs::hard_link(&original, &hard_link).await?;

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
    let repository = FileRepository::new(database);

    repository
        .batch_upsert_files(vec![
            FileSystemFile::create_file_info_from_path(&original).await?,
            FileSystemFile::create_file_info_from_path(&copy).await?,
            FileSystemFile::create_file_info_from_path(&hard_link).await?,
        ])
        .await?;

    let original = repository
        .get_file_by_path(&original)
        .await?
        .context("original file was not persisted")?;
    let copy = repository
        .get_file_by_path(&copy)
        .await?
        .context("copied file was not persisted")?;
    let hard_link = repository
        .get_file_by_path(&hard_link)
        .await?
        .context("hard link was not persisted")?;

    assert_ne!(original.id, copy.id);
    assert_ne!(original.id, hard_link.id);
    assert_eq!(original.content_digest.len(), 32);
    assert_eq!(original.content_digest, copy.content_digest);
    assert_eq!(original.content_digest, hard_link.content_digest);
    assert_ne!(
        (original.device_id, original.inode),
        (copy.device_id, copy.inode)
    );
    assert_eq!(
        (original.device_id, original.inode),
        (hard_link.device_id, hard_link.inode)
    );
    Ok(())
}
