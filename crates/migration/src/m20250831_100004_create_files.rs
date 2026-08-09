use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Files {
    Table,
    Id,
    FileSystemId,
    Name,
    Path,
    ContentHash,
    IdentityHash,
    FileTypeId,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum FileTypes {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum FileSystemIdentifier {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create Files table
        manager
            .create_table(
                Table::create()
                    .table(Files::Table)
                    .if_not_exists()
                    .col(pk_auto(Files::Id))
                    .col(integer(Files::FileSystemId))
                    .col(string(Files::Name))
                    .col(string(Files::Path))
                    .col(string(Files::ContentHash))
                    .col(string(Files::IdentityHash))
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
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_files_file_system_identifier")
                            .from(Files::Table, Files::FileSystemId)
                            .to(FileSystemIdentifier::Table, FileSystemIdentifier::Id)
                            .on_delete(ForeignKeyAction::Restrict)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index for Files
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_files_id_path")
                    .table(Files::Table)
                    .col(Files::Id)
                    .col(Files::Path)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop index first
        manager
            .drop_index(Index::drop().name("idx_files_id_path").to_owned())
            .await?;

        // Drop Files table (foreign keys will be dropped automatically)
        manager
            .drop_table(Table::drop().table(Files::Table).to_owned())
            .await?;

        Ok(())
    }
}
