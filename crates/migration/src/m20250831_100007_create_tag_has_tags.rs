use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum TagHasTags {
    Table,
    Id,
    SuperTagId,
    SubTagId,
}

#[derive(DeriveIden)]
enum Tags {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create TagHasTags junction table for tag hierarchies
        manager
            .create_table(
                Table::create()
                    .table(TagHasTags::Table)
                    .if_not_exists()
                    .col(pk_auto(TagHasTags::Id))
                    .col(integer(TagHasTags::SuperTagId))
                    .col(integer(TagHasTags::SubTagId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_tag_has_tags_super_tag")
                            .from(TagHasTags::Table, TagHasTags::SuperTagId)
                            .to(Tags::Table, Tags::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_tag_has_tags_sub_tag")
                            .from(TagHasTags::Table, TagHasTags::SubTagId)
                            .to(Tags::Table, Tags::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index for TagHasTags
        manager
            .create_index(
                Index::create()
                    .table(TagHasTags::Table)
                    .if_not_exists()
                    .col(TagHasTags::Id)
                    .col(TagHasTags::SuperTagId)
                    .col(TagHasTags::SubTagId)
                    .name("idx_tag_has_tags_id_super_sub")
                    .to_owned(),
            )
            .await?;

        // Create unique constraint to prevent duplicate tag hierarchies
        manager
            .create_index(
                Index::create()
                    .table(TagHasTags::Table)
                    .if_not_exists()
                    .col(TagHasTags::SuperTagId)
                    .col(TagHasTags::SubTagId)
                    .name("idx_tag_has_tags_unique_super_sub")
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
                    .name("idx_tag_has_tags_unique_super_sub")
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_tag_has_tags_id_super_sub")
                    .to_owned(),
            )
            .await?;

        // Drop TagHasTags table (foreign keys will be dropped automatically)
        manager
            .drop_table(Table::drop().table(TagHasTags::Table).to_owned())
            .await?;

        Ok(())
    }
}
