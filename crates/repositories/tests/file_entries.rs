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
    // The same entry spelling must work for observation, event upserts, and
    // deletion, even when the original path no longer exists.
    let root = directory.path().canonicalize()?;
    let target = root.join("target");
    tokio::fs::create_dir(&target).await?;
    std::os::unix::fs::symlink(&target, root.join("alias"))?;
    let entry = root.join("alias/entry.txt");
    let alternate = root.join("unused/../alias/./entry.txt");
    tokio::fs::write(&entry, b"entry bytes").await?;
    let observation = FileSystemFile::create_file_info_from_path(&alternate).await?;
    assert_eq!(observation.path, entry);
    assert_ne!(observation.path, entry.canonicalize()?);
    repository
        .batch_upsert_files(vec![observation.clone()])
        .await?;
    let stored = repository
        .get_file_by_path(&alternate)
        .await?
        .context("missing entry")?;
    assert_eq!(stored.path, entry.to_str().context("non-UTF-8 test path")?);

    let event = notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::File))
        .add_path(alternate.clone());
    let updated = repository
        .upsert_file_from_event(&events::FileEvent {
            kind: event.kind,
            paths: event.paths.clone(),
            event: notify_debouncer_full::DebouncedEvent::new(event, std::time::Instant::now()),
            content_digest: Some(observation.content_digest),
            filesystem_object_id: Some(observation.filesystem_object_id),
        })
        .await?;
    assert_eq!(updated.id, stored.id);
    let state = repository.get_database_state(&root).await?;
    assert_eq!(
        state
            .get(&entry)
            .context("missing comparison key")?
            .content_digest,
        observation.content_digest
    );
    tokio::fs::remove_file(&entry).await?;
    assert!(repository.delete_file_by_path(&alternate).await?);
    assert!(repository.get_file_by_path(&entry).await?.is_none());
    Ok(())
}
