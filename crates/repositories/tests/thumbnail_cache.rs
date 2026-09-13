use anyhow::{Context, Result};
use migration::{Migrator, MigratorTrait};
use model::services::file::FileSystemFile;
use model::services::thumbnail::{Thumbnail, ThumbnailSize};
use repositories::config::DatabaseSettings;
use repositories::fs::operations::FileRepository;
use repositories::manager::DatabaseManager;
use repositories::thumbnail::operations::ThumbnailOperations;
use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use std::sync::Arc;

async fn repositories() -> Result<(tempfile::TempDir, FileRepository, ThumbnailOperations)> {
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
    let files = FileRepository::new(Arc::clone(&database));
    let writer = ThumbnailOperations::new(database);
    let directory = tempfile::tempdir()?;
    for name in ["image.png", "other.png"] {
        let path = directory.path().join(name);
        tokio::fs::write(&path, name.as_bytes()).await?;
        files
            .batch_upsert_files(vec![
                FileSystemFile::create_file_info_from_path(&path).await?,
            ])
            .await?;
    }
    Ok((directory, files, writer))
}

#[tokio::test]
async fn thumbnail_lookups_observe_writes_and_deletes_across_repositories() -> Result<()> {
    let (directory, files, writer) = repositories().await?;
    let reader = files.thumbnail_repository();
    let path = directory.path().join("image.png");
    let file_id = files
        .get_file_by_path(&path)
        .await?
        .context("missing image")?
        .id;
    let small = Thumbnail::with_image_data(ThumbnailSize::Small, vec![1, 2, 3]);
    let medium = Thumbnail::with_image_data(ThumbnailSize::Medium, vec![4, 5]);
    let updated = Thumbnail::with_image_data(ThumbnailSize::Small, vec![9]);

    assert!(
        reader
            .get_by_file_and_size(file_id, ThumbnailSize::Small)
            .await?
            .is_none()
    );
    let stored = writer.create_thumbnail(file_id, small.clone()).await?;
    assert_eq!(
        reader
            .get_by_file_and_size(file_id, ThumbnailSize::Small)
            .await?,
        Some(small.clone())
    );
    assert_eq!(
        reader.get_thumbnail_by_id(stored.id).await?,
        Some(small.clone())
    );
    assert_eq!(
        reader.get_thumbnails_for_file(file_id).await?,
        vec![small.clone()]
    );
    writer.create_thumbnail(file_id, medium.clone()).await?;
    assert_eq!(
        reader
            .get_thumbnail_for_file_and_size(file_id, ThumbnailSize::Medium)
            .await?,
        medium
    );
    assert_eq!(reader.get_thumbnails_for_file(file_id).await?.len(), 2);
    writer.upsert_thumbnail(file_id, updated.clone()).await?;
    assert_eq!(reader.get_thumbnail_by_id(stored.id).await?, Some(updated));

    assert_eq!(writer.delete_thumbnails_for_file(file_id).await?, 2);
    assert!(reader.get_thumbnails_for_file(file_id).await?.is_empty());
    assert!(reader.get_thumbnail_by_id(stored.id).await?.is_none());
    writer.upsert_thumbnail(file_id, small.clone()).await?;
    assert_eq!(
        reader
            .get_by_file_and_size(file_id, ThumbnailSize::Small)
            .await?,
        Some(small)
    );
    assert!(files.delete_file_by_path(&path).await?);
    assert!(reader.get_thumbnails_for_file(file_id).await?.is_empty());
    assert_eq!(writer.delete_orphaned_thumbnails().await?, 0);
    Ok(())
}

#[tokio::test]
async fn thumbnail_batch_lookups_observe_only_committed_changes() -> Result<()> {
    let (directory, files, writer) = repositories().await?;
    let reader = files.thumbnail_repository();
    let path = directory.path().join("image.png");
    let other_path = directory.path().join("other.png");
    let file_id = files
        .get_file_by_path(&path)
        .await?
        .context("missing image")?
        .id;
    let other_id = files
        .get_file_by_path(&other_path)
        .await?
        .context("missing other")?
        .id;
    let small = Thumbnail::with_image_data(ThumbnailSize::Small, vec![1, 2, 3]);
    let medium = Thumbnail::with_image_data(ThumbnailSize::Medium, vec![4, 5]);
    let updated = Thumbnail::with_image_data(ThumbnailSize::Small, vec![9]);
    writer.create_thumbnail(file_id, updated.clone()).await?;
    assert_eq!(
        reader
            .get_by_file_and_size(file_id, ThumbnailSize::Small)
            .await?,
        Some(updated.clone())
    );
    assert_eq!(
        writer
            .batch_upsert_thumbnails(vec![(file_id, small.clone()), (other_id, updated.clone())])
            .await?,
        (1, 1)
    );
    assert_eq!(
        reader
            .get_thumbnail_for_file_and_size(file_id, ThumbnailSize::Small)
            .await?,
        small
    );
    // One warm file, one cold file, a duplicate, and a nonexistent file.
    let batch = reader
        .get_all_thumbnails_for_files_and_size(
            vec![file_id, other_id, file_id, -1],
            ThumbnailSize::Small,
        )
        .await?;
    assert_eq!(batch.len(), 2);
    assert!(batch.contains(&small));
    assert!(batch.contains(&updated));
    assert_eq!(
        reader
            .get_thumbnails_for_filter(vec![other_id], ThumbnailSize::Small)
            .await?,
        vec![updated.clone()]
    );
    assert!(
        reader
            .get_thumbnails_for_filter(vec![], ThumbnailSize::Small)
            .await?
            .is_empty()
    );

    // An update followed by a foreign-key failure must not escape the transaction.
    assert!(
        writer
            .batch_upsert_thumbnails(vec![(file_id, updated), (-1, small.clone())])
            .await
            .is_err()
    );
    assert_eq!(
        reader
            .get_thumbnail_for_file_and_size(file_id, ThumbnailSize::Small)
            .await?,
        small
    );
    // A successful insert followed by a duplicate must roll back as well.
    assert!(
        writer
            .batch_create_thumbnails(vec![(other_id, medium.clone()), (file_id, small)])
            .await
            .is_err()
    );
    assert!(
        reader
            .get_by_file_and_size(other_id, ThumbnailSize::Medium)
            .await?
            .is_none()
    );
    assert_eq!(
        writer
            .batch_create_thumbnails(vec![(other_id, medium.clone())])
            .await?,
        1
    );
    assert_eq!(
        reader
            .get_by_file_and_size(other_id, ThumbnailSize::Medium)
            .await?,
        Some(medium)
    );
    assert_eq!(files.batch_delete_files(vec![other_path]).await?, 1);
    assert!(reader.get_thumbnails_for_file(other_id).await?.is_empty());
    Ok(())
}
