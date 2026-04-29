use sea_orm::TransactionTrait;
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0011_add_theme_ssh_key"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .transaction::<_, _, DbErr>(|txn| {
                Box::pin(async move {
                    SchemaManager::new(txn)
                        .alter_table(
                            Table::alter()
                                .table(ThemeRegistry::Table)
                                .add_column(ColumnDef::new(ThemeRegistry::SshKeyName).string())
                                .to_owned(),
                        )
                        .await
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
                    SchemaManager::new(txn)
                        .alter_table(
                            Table::alter()
                                .table(ThemeRegistry::Table)
                                .drop_column(ThemeRegistry::SshKeyName)
                                .to_owned(),
                        )
                        .await
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
    SshKeyName,
}
