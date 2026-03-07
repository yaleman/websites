use std::sync::Arc;

use sea_orm::{Database, DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;

/// Runs all schema statements required by the current platform specification.
pub async fn ensure_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    crate::migration::Migrator::up(db, None).await
}

/// Start the database and run schema migrations.
pub async fn db_start(db_url: &str) -> Result<Arc<DatabaseConnection>, DbErr> {
    let db = Database::connect(db_url).await?;
    ensure_schema(&db).await?;
    Ok(Arc::new(db))
}

#[cfg(test)]
pub async fn test_db_start() -> Arc<DatabaseConnection> {
    crate::telemetry::test();
    db_start("sqlite::memory:")
        .await
        .expect("Failed to start in-memory db")
}

#[tokio::test]
pub async fn test_db_start_with_migrations() {
    test_db_start().await;
}
