use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Files {
    Table,
    _ID,
    Name,
    Path,
    FileSize,
    FileTypeID,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum FileTypes {
    Table,
    _ID,
    Name,
}

#[derive(DeriveIden)]
enum Tags {
    Table,
    _ID,
    Name,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum FileHasTags {
    Table,
    _ID,
    FileID,
    TagID,
}

#[derive(DeriveIden)]
enum TagHasTags {
    Table,
    _ID,
    SuperTagId,
    SubTagId,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // File Migration
        manager
            .create_table(
                Table::create()
                    .table(Files::Table)
                    .if_not_exists()
                    .col(pk_auto(Files::_ID))
                    .col(string(Files::Name))
                    .col(string(Files::Path))
                    .col(integer(Files::FileSize))
                    .col(integer(Files::FileTypeID))
                    .col(date_time(Files::CreatedAt))
                    .col(date_time(Files::UpdatedAt))
                    .to_owned(),
            )
            .await
            .expect("Failed to execute Migration for files");

        manager
            .create_index(
                Index::create()
                    .table(Files::Table)
                    .if_not_exists()
                    .col(Files::_ID)
                    .col(Files::Path)
                    .to_owned(),
            )
            .await
            .expect("Failed to create index for table files");

        manager
            .create_foreign_key(
                ForeignKey::create()
                    .from_col(Files::FileTypeID)
                    .to_col(FileTypes::_ID)
                    .to_owned(),
            )
            .await
            .expect("Could not create Foreign Key for Table Files");

        // FileType Migration
        manager
            .create_table(
                Table::create()
                    .table(FileTypes::Table)
                    .if_not_exists()
                    .col(pk_auto(FileTypes::_ID))
                    .col(string(FileTypes::Name))
                    .to_owned(),
            )
            .await
            .expect("Failed to execute Migration for file_types");

        manager
            .create_table(
                Table::create()
                    .table(Tags::Table)
                    .if_not_exists()
                    .col(pk_auto(Tags::_ID))
                    .col(string(Tags::Name))
                    .col(date_time(Tags::CreatedAt))
                    .col(date_time(Tags::UpdatedAt))
                    .to_owned(),
            )
            .await
            .expect("Failed to execute Migration for tags");

        manager
            .create_table(
                Table::create()
                    .table(FileHasTags::Table)
                    .if_not_exists()
                    .col(pk_auto(FileHasTags::_ID))
                    .col(integer(FileHasTags::FileID))
                    .col(integer(FileHasTags::TagID))
                    .to_owned(),
            )
            .await
            .expect("Failed to execute Migration for file_has_tags");

        manager
            .create_table(
                Table::create()
                    .table(TagHasTags::Table)
                    .if_not_exists()
                    .col(pk_auto(TagHasTags::_ID))
                    .col(integer(TagHasTags::SuperTagId))
                    .col(integer(TagHasTags::SubTagId))
                    .to_owned(),
            )
            .await
            .expect("Failed to execute Migration for tag_has_tags");
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Files::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");

        manager
            .drop_table(Table::drop().table(FileTypes::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");

        manager
            .drop_table(Table::drop().table(Tags::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");

        manager
            .drop_table(Table::drop().table(FileHasTags::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");

        manager
            .drop_table(Table::drop().table(TagHasTags::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");

        Ok(())
    }
}
