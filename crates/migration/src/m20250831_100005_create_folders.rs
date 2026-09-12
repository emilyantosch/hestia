use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Folders {
    Table,
    Id,
    DeviceId,
    Inode,
    ParentFolderId,
    Name,
    Path,
    CreatedAt,
    UpdatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Folders::Table)
                    .if_not_exists()
                    .col(pk_auto(Folders::Id))
                    .col(big_integer(Folders::DeviceId))
                    .col(big_integer(Folders::Inode))
                    .col(integer_null(Folders::ParentFolderId))
                    .col(string(Folders::Name))
                    .col(string(Folders::Path))
                    .col(date_time(Folders::CreatedAt))
                    .col(date_time(Folders::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_folders_parent_folder")
                            .from(Folders::Table, Folders::ParentFolderId)
                            .to(Folders::Table, Folders::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        for index in [
            Index::create()
                .name("idx_folders_path")
                .table(Folders::Table)
                .col(Folders::Path)
                .unique()
                .to_owned(),
            Index::create()
                .name("idx_folders_filesystem_object")
                .table(Folders::Table)
                .col(Folders::DeviceId)
                .col(Folders::Inode)
                .to_owned(),
            Index::create()
                .name("idx_folders_parent")
                .table(Folders::Table)
                .col(Folders::ParentFolderId)
                .to_owned(),
        ] {
            manager.create_index(index).await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in [
            "idx_folders_parent",
            "idx_folders_filesystem_object",
            "idx_folders_path",
        ] {
            manager
                .drop_index(Index::drop().name(name).to_owned())
                .await?;
        }

        manager
            .drop_table(Table::drop().table(Folders::Table).to_owned())
            .await?;

        Ok(())
    }
}
