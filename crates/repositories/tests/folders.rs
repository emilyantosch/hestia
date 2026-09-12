use anyhow::{Context, Result};
use hash::FilesystemObjectId;
use migration::{Migrator, MigratorTrait};
use model::services::folder::FileSystemFolder;
use repositories::config::DatabaseSettings;
use repositories::fs::operations::FileRepository;
use repositories::manager::DatabaseManager;
use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

#[tokio::test]
async fn roots_and_nested_folders_persist_by_location_and_object() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let canonical_root = directory.path().canonicalize()?;
    let root = canonical_root.as_path();
    let nested = root.join("nested");
    tokio::fs::create_dir(&nested).await?;
    let unreadable = nested.join("unreadable.txt");
    tokio::fs::write(&unreadable, b"folder persistence must not read this").await?;
    tokio::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).await?;

    let root_object_id = FilesystemObjectId::observe(root).await?;
    let nested_object_id = FilesystemObjectId::observe(&nested).await?;
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
    let repository = FileRepository::new(Arc::clone(&database));

    let alias = root.join("root-alias");
    std::os::unix::fs::symlink(root, &alias)?;
    repository
        .upsert_root_folders(vec![model::services::CanonPath::try_from(alias)?])
        .await?;
    let report = repository
        .batch_upsert_folders(vec![FileSystemFolder::create_folder_info(&nested).await?])
        .await?;
    assert_eq!(report.folder_inserted, 1);

    let roots = repository
        .find_root_folders(Some(database.get_connection().as_ref()))
        .await?;
    let root = roots.first().context("watched root was not persisted")?;
    assert_eq!(root.path, canonical_root.to_string_lossy());
    assert_eq!(root.parent_folder_id, None);
    assert_eq!(
        (root.device_id, root.inode),
        (
            i64::try_from(root_object_id.device)?,
            i64::try_from(root_object_id.inode)?
        )
    );

    let nested_folders = repository.find_subfolders_of_folder(root.id).await?;
    let nested = nested_folders
        .first()
        .context("nested folder was not persisted")?;
    assert_eq!(nested.path, canonical_root.join("nested").to_string_lossy());
    assert_eq!(nested.parent_folder_id, Some(root.id));
    assert_eq!(
        (nested.device_id, nested.inode),
        (
            i64::try_from(nested_object_id.device)?,
            i64::try_from(nested_object_id.inode)?
        )
    );
    Ok(())
}
