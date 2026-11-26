use sea_orm::TransactionTrait;
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_add_rsync_publish_method"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        rebuild_publish_tables(manager, true).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        rebuild_publish_tables(manager, false).await
    }
}

async fn rebuild_publish_tables(
    manager: &SchemaManager<'_>,
    include_rsync: bool,
) -> Result<(), DbErr> {
    let txn = manager.get_connection().begin().await?;
    let schema = SchemaManager::new(&txn);
    schema
        .rename_table(
            Table::rename()
                .table(SitePublishConfig::Table, SitePublishConfig::OldTable)
                .to_owned(),
        )
        .await?;
    schema
        .rename_table(
            Table::rename()
                .table(SitePublishRun::Table, SitePublishRun::OldTable)
                .to_owned(),
        )
        .await?;

    let mut config_methods = vec!["disabled", "s3_compatible"];
    let mut run_methods = vec!["s3_compatible"];
    if include_rsync {
        config_methods.push("rsync_ssh");
        run_methods.push("rsync_ssh");
    }
    schema
        .create_table(
            Table::create()
                .table(SitePublishConfig::Table)
                .col(
                    ColumnDef::new(SitePublishConfig::SiteId)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(SitePublishConfig::Method)
                        .string()
                        .not_null()
                        .check(Expr::col(SitePublishConfig::Method).is_in(config_methods.clone())),
                )
                .col(
                    ColumnDef::new(SitePublishConfig::ConfigJson)
                        .json()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(SitePublishConfig::CreatedAt)
                        .timestamp()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(SitePublishConfig::UpdatedAt)
                        .timestamp()
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_site_publish_config_site")
                        .from(SitePublishConfig::Table, SitePublishConfig::SiteId)
                        .to(Site::Table, Site::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
    schema
        .create_table(
            Table::create()
                .table(SitePublishRun::Table)
                .col(
                    ColumnDef::new(SitePublishRun::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(SitePublishRun::SiteId).uuid().not_null())
                .col(
                    ColumnDef::new(SitePublishRun::Method)
                        .string()
                        .not_null()
                        .check(Expr::col(SitePublishRun::Method).is_in(run_methods.clone())),
                )
                .col(
                    ColumnDef::new(SitePublishRun::Status)
                        .string()
                        .not_null()
                        .check(Expr::col(SitePublishRun::Status).is_in([
                            "queued",
                            "running",
                            "succeeded",
                            "failed",
                        ])),
                )
                .col(ColumnDef::new(SitePublishRun::ActorSub).string().not_null())
                .col(
                    ColumnDef::new(SitePublishRun::RenderedFileCount)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(SitePublishRun::PublishedFileCount)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(SitePublishRun::DeletedObjectCount)
                        .integer()
                        .not_null(),
                )
                .col(ColumnDef::new(SitePublishRun::ErrorMessage).string())
                .col(
                    ColumnDef::new(SitePublishRun::CreatedAt)
                        .timestamp()
                        .not_null(),
                )
                .col(ColumnDef::new(SitePublishRun::StartedAt).timestamp())
                .col(ColumnDef::new(SitePublishRun::FinishedAt).timestamp())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_site_publish_run_site")
                        .from(SitePublishRun::Table, SitePublishRun::SiteId)
                        .to(Site::Table, Site::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    let config_columns = [
        SitePublishConfig::SiteId,
        SitePublishConfig::Method,
        SitePublishConfig::ConfigJson,
        SitePublishConfig::CreatedAt,
        SitePublishConfig::UpdatedAt,
    ];
    let mut insert = Query::insert();
    insert
        .into_table(SitePublishConfig::Table)
        .columns(config_columns);
    insert
        .select_from(
            Query::select()
                .columns(config_columns)
                .from(SitePublishConfig::OldTable)
                .and_where(Expr::col(SitePublishConfig::Method).is_in(config_methods))
                .to_owned(),
        )
        .map_err(|source| DbErr::TryIntoErr {
            from: "SelectStatement",
            into: "InsertStatement",
            source: std::sync::Arc::new(source),
        })?;
    schema.execute(insert).await?;

    let run_columns = [
        SitePublishRun::Id,
        SitePublishRun::SiteId,
        SitePublishRun::Method,
        SitePublishRun::Status,
        SitePublishRun::ActorSub,
        SitePublishRun::RenderedFileCount,
        SitePublishRun::PublishedFileCount,
        SitePublishRun::DeletedObjectCount,
        SitePublishRun::ErrorMessage,
        SitePublishRun::CreatedAt,
        SitePublishRun::StartedAt,
        SitePublishRun::FinishedAt,
    ];
    let mut insert = Query::insert();
    insert
        .into_table(SitePublishRun::Table)
        .columns(run_columns);
    insert
        .select_from(
            Query::select()
                .columns(run_columns)
                .from(SitePublishRun::OldTable)
                .and_where(Expr::col(SitePublishRun::Method).is_in(run_methods))
                .to_owned(),
        )
        .map_err(|source| DbErr::TryIntoErr {
            from: "SelectStatement",
            into: "InsertStatement",
            source: std::sync::Arc::new(source),
        })?;
    schema.execute(insert).await?;

    schema
        .drop_index(
            Index::drop()
                .name("idx_site_publish_run_site_created")
                .table(SitePublishRun::OldTable)
                .if_exists()
                .to_owned(),
        )
        .await?;
    schema
        .create_index(
            Index::create()
                .name("idx_site_publish_run_site_created")
                .table(SitePublishRun::Table)
                .col(SitePublishRun::SiteId)
                .col(SitePublishRun::CreatedAt)
                .to_owned(),
        )
        .await?;
    schema
        .drop_table(Table::drop().table(SitePublishRun::OldTable).to_owned())
        .await?;
    schema
        .drop_table(Table::drop().table(SitePublishConfig::OldTable).to_owned())
        .await?;
    txn.commit().await
}

#[derive(Clone, Copy, Iden)]
enum SitePublishConfig {
    Table,
    #[iden = "site_publish_config_old"]
    OldTable,
    SiteId,
    Method,
    ConfigJson,
    CreatedAt,
    UpdatedAt,
}

#[derive(Clone, Copy, Iden)]
enum SitePublishRun {
    Table,
    #[iden = "site_publish_run_old"]
    OldTable,
    Id,
    SiteId,
    Method,
    Status,
    ActorSub,
    RenderedFileCount,
    PublishedFileCount,
    DeletedObjectCount,
    ErrorMessage,
    CreatedAt,
    StartedAt,
    FinishedAt,
}

#[derive(Iden)]
enum Site {
    Table,
    Id,
}

#[cfg(test)]
mod tests;
