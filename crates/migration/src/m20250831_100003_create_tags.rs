use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Tags {
    Table,
    Id,
    Name,
    CreatedAt,
    UpdatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create Tags table
        manager
            .create_table(
                Table::create()
                    .table(Tags::Table)
                    .if_not_exists()
                    .col(pk_auto(Tags::Id))
                    .col(string(Tags::Name))
                    .col(date_time(Tags::CreatedAt))
                    .col(date_time(Tags::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        // Create index for Tags
        manager
            .create_index(
                Index::create()
                    .table(Tags::Table)
                    .if_not_exists()
                    .col(Tags::Id)
                    .col(Tags::Name)
                    .name("idx_tags_id_name")
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop index first
        manager
            .drop_index(Index::drop().name("idx_tags_id_name").to_owned())
            .await?;

        // Drop Tags table
        manager
            .drop_table(Table::drop().table(Tags::Table).to_owned())
            .await?;

        Ok(())
    }
}
