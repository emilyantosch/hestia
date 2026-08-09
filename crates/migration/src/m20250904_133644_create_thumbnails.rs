use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Thumbnails {
    Table,
    Id,
    FileId,
    Size,
    Data,
    MimeType,
    FileSize,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Files {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create Thumbnails table
        manager
            .create_table(
                Table::create()
                    .table(Thumbnails::Table)
                    .if_not_exists()
                    .col(pk_auto(Thumbnails::Id))
                    .col(integer(Thumbnails::FileId))
                    .col(
                        ColumnDef::new(Thumbnails::Size)
                            .string_len(16)
                            .not_null()
                            .check(Expr::col(Thumbnails::Size).is_in(["small", "medium", "large"])),
                    )
                    .col(binary_len(Thumbnails::Data, 10485760)) // 10MB max thumbnail size
                    .col(string_len(Thumbnails::MimeType, 100))
                    .col(integer(Thumbnails::FileSize))
                    .col(date_time(Thumbnails::CreatedAt))
                    .col(date_time(Thumbnails::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_thumbnails_files")
                            .from(Thumbnails::Table, Thumbnails::FileId)
                            .to(Files::Table, Files::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create unique index to prevent duplicate thumbnails for same file/size
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_thumbnails_file_size_unique")
                    .table(Thumbnails::Table)
                    .col(Thumbnails::FileId)
                    .col(Thumbnails::Size)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create index for efficient batch queries by size
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_thumbnails_size_file")
                    .table(Thumbnails::Table)
                    .col(Thumbnails::Size)
                    .col(Thumbnails::FileId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop indexes first
        manager
            .drop_index(Index::drop().name("idx_thumbnails_size_file").to_owned())
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_thumbnails_file_size_unique")
                    .to_owned(),
            )
            .await?;

        // Drop Thumbnails table (foreign keys will be dropped automatically)
        manager
            .drop_table(Table::drop().table(Thumbnails::Table).to_owned())
            .await?;

        Ok(())
    }
}
