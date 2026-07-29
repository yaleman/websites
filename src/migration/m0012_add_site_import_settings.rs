use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0012_add_site_import_settings"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Site::Table)
                    .add_column(
                        ColumnDef::new(Site::InternalDomains)
                            .json()
                            .not_null()
                            .default("[]"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Site::Table)
                    .add_column(ColumnDef::new(Site::MassImportAssets).string())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Site::Table)
                    .drop_column(Site::MassImportAssets)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Site::Table)
                    .drop_column(Site::InternalDomains)
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
enum Site {
    Table,
    InternalDomains,
    MassImportAssets,
}
