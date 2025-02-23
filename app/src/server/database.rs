use dioxus::prelude::*;
use sea_orm::{Database, DatabaseConnection, DbErr};

async fn _init_db() -> Result<DatabaseConnection, DbErr> {
    let db: DatabaseConnection = Database::connect("sqlite://main.sqlite3?mode=rwc")
        .await
        .expect("Failed to open database!");
    Ok(db)
}

#[server]
async fn input_test() -> Result<(), ServerFnError> {
    let db = _init_db();
    Ok(())
}
