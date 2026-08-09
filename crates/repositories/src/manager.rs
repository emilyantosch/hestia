use anyhow::{Context, Result, ensure};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use std::sync::Arc;
use std::time::Duration;

use crate::config::DatabaseSettings;

#[derive(Debug)]
pub struct DatabaseManager {
    connection: Arc<DatabaseConnection>,
    settings: DatabaseSettings,
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
