use anyhow::{Context, Result, ensure};
use moka::sync::Cache;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::config::DatabaseSettings;

#[derive(Debug)]
pub struct DatabaseManager {
    connection: Arc<DatabaseConnection>,
    settings: DatabaseSettings,
    pub(crate) thumbnail_cache: Cache<(u64, i32), Arc<Vec<entity::thumbnails::Model>>>,
    pub(crate) thumbnail_cache_generation: AtomicU64,
}

impl DatabaseManager {
    pub async fn new(settings: DatabaseSettings) -> Result<Self> {
        ensure!(
            !settings.con_string.trim().is_empty(),
            "database connection string is empty"
        );

        let connection = Self::create_connection(&settings).await?;
        Ok(Self {
            connection: Arc::new(connection),
            settings,
            thumbnail_cache: Cache::builder()
                .max_capacity(64 * 1024 * 1024)
                .weigher(
                    |_: &(u64, i32), models: &Arc<Vec<entity::thumbnails::Model>>| {
                        let bytes = models.iter().fold(
                            size_of::<Vec<entity::thumbnails::Model>>(),
                            |total, model| {
                                total
                                    .saturating_add(size_of::<entity::thumbnails::Model>())
                                    .saturating_add(model.data.len())
                                    .saturating_add(model.mime_type.len())
                                    .saturating_add(model.size.len())
                            },
                        );
                        u32::try_from(bytes).unwrap_or(u32::MAX)
                    },
                )
                .build(),
            thumbnail_cache_generation: AtomicU64::new(0),
        })
    }

    pub async fn new_sqlite_default() -> Result<Self> {
        Self::new(DatabaseSettings::new(
            "sqlite://main.sqlite?mode=rwc".to_string(),
            30_000,
            sea_orm::sqlx::sqlite::SqliteJournalMode::Wal,
            sea_orm::sqlx::sqlite::SqliteSynchronous::Normal,
        ))
        .await
    }

    #[must_use]
    pub fn get_connection(&self) -> Arc<DatabaseConnection> {
        Arc::clone(&self.connection)
    }

    #[must_use]
    pub fn get_settings(&self) -> &DatabaseSettings {
        &self.settings
    }

    pub(crate) fn invalidate_thumbnail_cache(&self) {
        // A read started before a write must not repopulate the current generation.
        self.thumbnail_cache_generation
            .fetch_add(1, Ordering::SeqCst);
        // ponytail: invalidate all files on writes; use per-file generations if hit rate suffers.
        self.thumbnail_cache.invalidate_all();
    }

    pub async fn test_connection(&self) -> Result<()> {
        let statement = sea_orm::query::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT 1".to_string(),
        );
        self.connection.execute(statement).await?;
        Ok(())
    }

    async fn create_connection(settings: &DatabaseSettings) -> Result<DatabaseConnection> {
        let mut options = ConnectOptions::new(&settings.con_string);
        options
            .connect_timeout(Duration::from_millis(u64::from(settings.timeout)))
            .sqlx_logging(false);

        Database::connect(options)
            .await
            .context("failed to connect to database")
    }
}
