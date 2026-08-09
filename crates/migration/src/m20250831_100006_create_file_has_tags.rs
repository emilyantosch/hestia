use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum FileHasTags {
    Table,
    Id,
    FileId,
    TagId,
}

#[derive(DeriveIden)]
enum Files {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Tags {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create FileHasTags junction table
        manager
            .create_table(
                Table::create()
                    .table(FileHasTags::Table)
                    .if_not_exists()
                    .col(pk_auto(FileHasTags::Id))
                    .col(integer(FileHasTags::FileId))
                    .col(integer(FileHasTags::TagId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_file_has_tags_files")
                            .from(FileHasTags::Table, FileHasTags::FileId)
                            .to(Files::Table, Files::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_file_has_tags_tags")
                            .from(FileHasTags::Table, FileHasTags::TagId)
                            .to(Tags::Table, Tags::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index for FileHasTags
        manager
            .create_index(
                Index::create()
                    .table(FileHasTags::Table)
                    .if_not_exists()
                    .col(FileHasTags::Id)
                    .col(FileHasTags::TagId)
                    .col(FileHasTags::FileId)
                    .name("idx_file_has_tags_id_tag_file")
                    .to_owned(),
            )
            .await?;

        // Create unique constraint to prevent duplicate file-tag relationships
        manager
            .create_index(
                Index::create()
                    .table(FileHasTags::Table)
                    .if_not_exists()
                    .col(FileHasTags::FileId)
                    .col(FileHasTags::TagId)
                    .name("idx_file_has_tags_unique_file_tag")
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop indexes first
        manager
            .drop_index(
                Index::drop()
                    .name("idx_file_has_tags_unique_file_tag")
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_file_has_tags_id_tag_file")
                    .to_owned(),
            )
            .await?;

        // Drop FileHasTags table (foreign keys will be dropped automatically)
        manager
            .drop_table(Table::drop().table(FileHasTags::Table).to_owned())
            .await?;

        Ok(())
    }
}
