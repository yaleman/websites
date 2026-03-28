use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0010_add_publish_on_render"
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
                        ColumnDef::new(Site::PublishOnRender)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Site::Table)
                    .drop_column(Site::PublishOnRender)
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
enum Site {
    Table,
    PublishOnRender,
}
