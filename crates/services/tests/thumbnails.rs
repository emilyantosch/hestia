use anyhow::{Result, bail};
use migration::{Migrator, MigratorTrait};
use model::services::file::FileSystemFile;
use model::services::thumbnail::ThumbnailSize;
use repositories::{
    config::DatabaseSettings, fs::operations::FileRepository, manager::DatabaseManager,
    thumbnail::operations::ThumbnailOperations,
};
use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use services::thumbnails::generator::ThumbnailGenerator;
use services::thumbnails::thumbnails::{ThumbnailProcessor, ThumbnailProcessorHandler};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn processor_generates_missing_thumbnails_and_stops_with_its_handler() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("notes.txt");
    tokio::fs::write(&path, b"notes").await?;
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
    files
        .batch_upsert_files(vec![
            FileSystemFile::create_file_info_from_path(&path).await?,
        ])
        .await?;
    let thumbnails = Arc::new(ThumbnailOperations::new(Arc::clone(&database)));

    let (handler, receiver) = ThumbnailProcessorHandler::new();
    let processor = tokio::spawn(
        ThumbnailProcessor::new(
            receiver,
            Arc::clone(&thumbnails),
            Arc::new(ThumbnailGenerator::new()),
        )
        .run(),
    );
    handler.queue_missing_files()?;

    let file_id = files.get_file_by_path(&path).await?.map(|file| file.id);
    let Some(file_id) = file_id else {
        bail!("the file was not stored");
    };
    let generated = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let generated = thumbnails.get_thumbnails_for_file(file_id).await?;
            if generated.len() == ThumbnailSize::all().len() {
                return anyhow::Ok(generated);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??;
    assert!(
        generated
            .iter()
            .all(|thumbnail| thumbnail.mime_type() == "image/png")
    );

    drop(handler);
    tokio::time::timeout(Duration::from_secs(5), processor).await???;
    Ok(())
}
