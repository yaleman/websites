use sea_orm::TransactionTrait;
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_add_theme_registry"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .transaction::<_, _, DbErr>(|txn| {
                Box::pin(async move {
                    let schema = SchemaManager::new(txn);

                    schema
                        .create_table(
                            Table::create()
                                .table(ThemeRegistry::Table)
                                .if_not_exists()
                                .col(
                                    ColumnDef::new(ThemeRegistry::Id)
                                        .uuid()
                                        .not_null()
                                        .primary_key(),
                                )
                                .col(ColumnDef::new(ThemeRegistry::Slug).string().not_null())
                                .col(ColumnDef::new(ThemeRegistry::RepoUrl).string().not_null())
                                .col(ColumnDef::new(ThemeRegistry::Branch).string())
                                .col(
                                    ColumnDef::new(ThemeRegistry::CreatedAt)
                                        .timestamp()
                                        .not_null(),
                                )
                                .col(
                                    ColumnDef::new(ThemeRegistry::UpdatedAt)
                                        .timestamp()
                                        .not_null(),
                                )
                                .to_owned(),
                        )
                        .await?;

                    schema
                        .create_index(
                            Index::create()
                                .name("idx_theme_registry_slug")
                                .table(ThemeRegistry::Table)
                                .col(ThemeRegistry::Slug)
                                .unique()
                                .to_owned(),
                        )
                        .await?;

                    Ok(())
                })
            })
            .await
            .map_err(|error| match error {
                sea_orm::TransactionError::Connection(err) => err,
                sea_orm::TransactionError::Transaction(err) => err,
            })
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .transaction::<_, _, DbErr>(|txn| {
                Box::pin(async move {
                    let schema = SchemaManager::new(txn);
                    schema
                        .drop_index(
                            Index::drop()
                                .name("idx_theme_registry_slug")
                                .table(ThemeRegistry::Table)
                                .to_owned(),
                        )
                        .await?;
                    schema
                        .drop_table(Table::drop().table(ThemeRegistry::Table).to_owned())
                        .await?;
                    Ok(())
                })
            })
            .await
            .map_err(|error| match error {
                sea_orm::TransactionError::Connection(err) => err,
                sea_orm::TransactionError::Transaction(err) => err,
            })
    }
}

#[derive(Iden)]
enum ThemeRegistry {
    Table,
    Id,
    Slug,
    RepoUrl,
    Branch,
    CreatedAt,
    UpdatedAt,
}
