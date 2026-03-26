use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_add_site_publish_workflow"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SitePublishConfig::Table)
                    .if_not_exists()
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
                            .check(
                                Expr::col(SitePublishConfig::Method).is_in(vec!["s3_compatible"]),
                            ),
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

        manager
            .create_table(
                Table::create()
                    .table(SitePublishRun::Table)
                    .if_not_exists()
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
                            .check(Expr::col(SitePublishRun::Method).is_in(vec!["s3_compatible"])),
                    )
                    .col(
                        ColumnDef::new(SitePublishRun::Status)
                            .string()
                            .not_null()
                            .check(Expr::col(SitePublishRun::Status).is_in(vec![
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

        manager
            .create_index(
                Index::create()
                    .name("idx_site_publish_run_site_created")
                    .table(SitePublishRun::Table)
                    .col(SitePublishRun::SiteId)
                    .col(SitePublishRun::CreatedAt)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SitePublishRun::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(SitePublishConfig::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum SitePublishConfig {
    Table,
    SiteId,
    Method,
    ConfigJson,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum SitePublishRun {
    Table,
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
