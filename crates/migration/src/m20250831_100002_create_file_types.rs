use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum FileTypes {
    Table,
    Id,
    Name,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create FileTypes table
        manager
            .create_table(
                Table::create()
                    .table(FileTypes::Table)
                    .if_not_exists()
                    .col(pk_auto(FileTypes::Id))
                    .col(string(FileTypes::Name))
                    .to_owned(),
            )
            .await?;

        // Create index for FileTypes
        manager
            .create_index(
                Index::create()
                    .table(FileTypes::Table)
                    .if_not_exists()
                    .col(FileTypes::Id)
                    .col(FileTypes::Name)
                    .name("idx_file_types_id_name")
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop index first
        manager
            .drop_index(Index::drop().name("idx_file_types_id_name").to_owned())
            .await?;

        // Drop FileTypes table
        manager
            .drop_table(Table::drop().table(FileTypes::Table).to_owned())
            .await?;

        Ok(())
    }
}
