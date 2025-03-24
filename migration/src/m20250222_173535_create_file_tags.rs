use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Files {
    Table,
    _ID,
    Name,
    Hash,
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
                    .foreign_key(
                        &mut ForeignKey::create()
                            .name("C_FK_FILES_FILETYPES_FILETYPEID_ID")
                            .from(Files::Table, Files::FileTypeID)
                            .to(FileTypes::Table, FileTypes::_ID)
                            .to_owned(),
                    )
                    .col(pk_auto(Files::_ID))
                    .col(string(Files::Name))
                    .col(string(Files::Path))
                    .col(string(Files::Hash))
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
                    .if_not_exists()
                    .name("IDX_FILES_ID_NAME")
                    .table(Files::Table)
                    .col(Files::_ID)
                    .col(Files::Path)
                    .to_owned(),
            )
            .await
            .expect("Failed to create index for table files");

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
            .create_index(
                Index::create()
                    .table(FileTypes::Table)
                    .if_not_exists()
                    .col(FileTypes::_ID)
                    .col(FileTypes::Name)
                    .name("IDX_FILETYPES_ID_NAME")
                    .to_owned(),
            )
            .await
            .expect("Failed to create index for table files");

        // Tags Migration
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
            .create_index(
                Index::create()
                    .table(Tags::Table)
                    .if_not_exists()
                    .col(Tags::_ID)
                    .col(Tags::Name)
                    .name("IDX_TAGS_ID_NAME")
                    .to_owned(),
            )
            .await
            .expect("Failed to create index for table tags.");

        // FileHasTags Migration
        manager
            .create_table(
                Table::create()
                    .table(FileHasTags::Table)
                    .if_not_exists()
                    .col(pk_auto(FileHasTags::_ID))
                    .col(integer(FileHasTags::FileID))
                    .col(integer(FileHasTags::TagID))
                    .foreign_key(
                        &mut ForeignKey::create()
                            .from(FileHasTags::Table, FileHasTags::FileID)
                            .to(Files::Table, Files::_ID)
                            .name("C_FK_FILEHASTAGS_FILES_FILEID_ID")
                            .to_owned(),
                    )
                    .foreign_key(
                        &mut ForeignKey::create()
                            .from(FileHasTags::Table, FileHasTags::TagID)
                            .to(Tags::Table, Tags::_ID)
                            .name("C_FK_FILEHASTAGS_TAGS_TAGID_ID")
                            .to_owned(),
                    )
                    .to_owned(),
            )
            .await
            .expect("Failed to execute Migration for file_has_tags");

        manager
            .create_index(
                Index::create()
                    .table(FileHasTags::Table)
                    .if_not_exists()
                    .col(FileHasTags::_ID)
                    .col(FileHasTags::TagID)
                    .col(FileHasTags::FileID)
                    .name("IDX_FILEHASTAGS_ID_TAGID_FILEID")
                    .to_owned(),
            )
            .await
            .expect("Failed to create index for table tags.");

        // TagHasTags Migration
        manager
            .create_table(
                Table::create()
                    .table(TagHasTags::Table)
                    .if_not_exists()
                    .col(pk_auto(TagHasTags::_ID))
                    .col(integer(TagHasTags::SuperTagId))
                    .col(integer(TagHasTags::SubTagId))
                    .foreign_key(
                        &mut ForeignKey::create()
                            .from(TagHasTags::Table, TagHasTags::SubTagId)
                            .to(Tags::Table, Tags::_ID)
                            .name("C_FK_TAGHASTAG_TAG_SUBTAGID_ID")
                            .to_owned(),
                    )
                    .foreign_key(
                        &mut ForeignKey::create()
                            .from(TagHasTags::Table, TagHasTags::SuperTagId)
                            .to(Tags::Table, Tags::_ID)
                            .name("C_FK_TAGHASTAG_TAG_SUPERTAGID_ID")
                            .to_owned(),
                    )
                    .to_owned(),
            )
            .await
            .expect("Failed to execute Migration for tag_has_tags");

        manager
            .create_index(
                Index::create()
                    .table(TagHasTags::Table)
                    .if_not_exists()
                    .col(TagHasTags::_ID)
                    .col(TagHasTags::SuperTagId)
                    .col(TagHasTags::SubTagId)
                    .name("IDX_TAGHASTAGS_ID_SUPERTAGID_SUBTAGID")
                    .to_owned(),
            )
            .await
            .expect("Failed to create index for table TagHasTags.");
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Dropping Index on Files
        manager
            .drop_index(Index::drop().name("IDX_FILES_ID_NAME").to_owned())
            .await
            .expect("Could not execute drop for index idx_files_id_name");

        // Dropping Table Files
        manager
            .drop_table(Table::drop().table(Files::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");

        // Dropping Index on FileTypes
        manager
            .drop_index(Index::drop().name("IDX_FILETYPES_ID_NAME").to_owned())
            .await
            .expect("Could not execute drop statement for index IDX_FILETYPES_ID_NAME");

        // Dropping table filetypes
        manager
            .drop_table(Table::drop().table(FileTypes::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");

        // Dropping index on table tags
        manager
            .drop_index(Index::drop().name("IDX_TAGS_ID_NAME").to_owned())
            .await
            .expect("Could not execute drop for index IDX_TAGS_ID_NAME");

        // Dropping table tags
        manager
            .drop_table(Table::drop().table(Tags::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");

        // Dropping index on table FileHasTags
        manager
            .drop_index(
                Index::drop()
                    .name("IDX_FILEHASTAGS_ID_TAGID_FILEID")
                    .to_owned(),
            )
            .await
            .expect("Could not execute drop for index IDX_FILEHASTAGS_ID_TAGID_FILEID");

        //Dropping table FileHasTags
        manager
            .drop_table(Table::drop().table(FileHasTags::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");

        // Dropping index on table TagHasTags
        manager
            .drop_index(
                Index::drop()
                    .name("IDX_TAGHASTAGS_ID_SUPERTAGID_SUBTAGID")
                    .to_owned(),
            )
            .await
            .expect("Could not execute drop for index IDX_TAGHASTAGS_ID_SUPERTAGID_SUBTAGID");

        // Dropping table TagHasTags
        manager
            .drop_table(Table::drop().table(TagHasTags::Table).to_owned())
            .await
            .expect("Could not execute drop for table tag_has_tags");
        Ok(())
    }
}
