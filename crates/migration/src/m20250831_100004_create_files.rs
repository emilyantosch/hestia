use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Files {
    Table,
    Id,
    Name,
    Path,
    ContentDigest,
    DeviceId,
    Inode,
    FileTypeId,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum FileTypes {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Files::Table)
                    .if_not_exists()
                    .col(pk_auto(Files::Id))
                    .col(string(Files::Name))
                    .col(string(Files::Path))
                    .col(binary_len(Files::ContentDigest, 32))
                    .col(big_integer(Files::DeviceId))
                    .col(big_integer(Files::Inode))
                    .col(integer(Files::FileTypeId))
                    .col(date_time(Files::CreatedAt))
                    .col(date_time(Files::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_files_file_types")
                            .from(Files::Table, Files::FileTypeId)
                            .to(FileTypes::Table, FileTypes::Id)
                            .on_delete(ForeignKeyAction::Restrict)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        for index in [
            Index::create()
                .name("idx_files_path")
                .table(Files::Table)
                .col(Files::Path)
                .unique()
                .to_owned(),
            Index::create()
                .name("idx_files_content_digest")
                .table(Files::Table)
                .col(Files::ContentDigest)
                .to_owned(),
            Index::create()
                .name("idx_files_filesystem_object")
                .table(Files::Table)
                .col(Files::DeviceId)
                .col(Files::Inode)
                .to_owned(),
        ] {
            manager.create_index(index).await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in [
            "idx_files_filesystem_object",
            "idx_files_content_digest",
            "idx_files_path",
        ] {
            manager
                .drop_index(Index::drop().name(name).to_owned())
                .await?;
        }
        manager
            .drop_table(Table::drop().table(Files::Table).to_owned())
            .await?;
        Ok(())
    }
}
