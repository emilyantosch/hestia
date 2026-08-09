use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};

#[derive(Debug, Clone, Default)]
pub struct DatabaseSettings {
    pub con_string: String,
    pub timeout: u32,
    pub journal_mode: SqliteJournalMode,
    pub synchronous: SqliteSynchronous,
}

impl DatabaseSettings {
    #[must_use]
    pub fn new(
        con_string: String,
        timeout: u32,
        journal_mode: SqliteJournalMode,
        synchronous: SqliteSynchronous,
    ) -> Self {
        Self {
            con_string,
            timeout,
            journal_mode,
            synchronous,
        }
    }
}
