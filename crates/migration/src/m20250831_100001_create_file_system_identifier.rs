use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum FileSystemIdentifier {
    Table,
    Id,
    Inode,
    DeviceNum,
    VolumeSerialNum,
    IndexNum,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create FileSystemIdentifier table
        manager
            .create_table(
                Table::create()
                    .table(FileSystemIdentifier::Table)
                    .if_not_exists()
                    .col(pk_auto(FileSystemIdentifier::Id))
                    .col(integer_null(FileSystemIdentifier::Inode))
                    .col(integer_null(FileSystemIdentifier::DeviceNum))
                    .col(integer_null(FileSystemIdentifier::IndexNum))
                    .col(integer_null(FileSystemIdentifier::VolumeSerialNum))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop FileSystemIdentifier table
        manager
            .drop_table(Table::drop().table(FileSystemIdentifier::Table).to_owned())
            .await?;

        Ok(())
    }
}
