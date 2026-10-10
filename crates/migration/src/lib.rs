pub use sea_orm_migration::prelude::*;

// Individual migrations in dependency order
mod m20250831_100001_create_file_system_identifier;
mod m20250831_100002_create_file_types;
mod m20250831_100003_create_tags;
mod m20250831_100004_create_files;
mod m20250831_100005_create_folders;
mod m20250831_100006_create_file_has_tags;
mod m20250831_100007_create_tag_has_tags;
mod m20250831_181914_icon_color;
mod m20250904_133644_create_thumbnails;
mod m20260901_100001_tag_color;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // Base tables with no dependencies
            Box::new(m20250831_100001_create_file_system_identifier::Migration),
            Box::new(m20250831_100002_create_file_types::Migration),
            Box::new(m20250831_100003_create_tags::Migration),
            // Tables with dependencies on base tables
            Box::new(m20250831_100004_create_files::Migration),
            Box::new(m20250831_100005_create_folders::Migration),
            // Junction tables with dependencies on entity tables
            Box::new(m20250831_100006_create_file_has_tags::Migration),
            Box::new(m20250831_100007_create_tag_has_tags::Migration),
            // Additional feature tables
            Box::new(m20250831_181914_icon_color::Migration),
            Box::new(m20250904_133644_create_thumbnails::Migration),
            Box::new(m20260901_100001_tag_color::Migration),
        ]
    }
}
