use sea_orm::{ConnectionTrait, TransactionTrait};
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
        let db = manager.get_connection();
        let txn = db.begin().await?;

        txn.execute_unprepared(
            r#"
            ALTER TABLE site_publish_config RENAME TO site_publish_config_old;
            "#,
        )
        .await?;
        txn.execute_unprepared(
            r#"
            ALTER TABLE site_publish_run RENAME TO site_publish_run_old;
            "#,
        )
        .await?;

        txn.execute_unprepared(
            r#"
            CREATE TABLE site_publish_config (
                site_id TEXT NOT NULL PRIMARY KEY,
                method TEXT NOT NULL CHECK (method IN ('disabled', 's3_compatible', 'rsync_ssh')),
                config_json JSON NOT NULL,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                FOREIGN KEY (site_id) REFERENCES site (id) ON DELETE CASCADE
            );
            "#,
        )
        .await?;
        txn.execute_unprepared(
            r#"
            CREATE TABLE site_publish_run (
                id TEXT NOT NULL PRIMARY KEY,
                site_id TEXT NOT NULL,
                method TEXT NOT NULL CHECK (method IN ('s3_compatible', 'rsync_ssh')),
                status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
                actor_sub TEXT NOT NULL,
                rendered_file_count INTEGER NOT NULL,
                published_file_count INTEGER NOT NULL,
                deleted_object_count INTEGER NOT NULL,
                error_message TEXT,
                created_at TIMESTAMP NOT NULL,
                started_at TIMESTAMP,
                finished_at TIMESTAMP,
                FOREIGN KEY (site_id) REFERENCES site (id) ON DELETE CASCADE
            );
            "#,
        )
        .await?;

        txn.execute_unprepared(
            r#"
            INSERT INTO site_publish_config (site_id, method, config_json, created_at, updated_at)
            SELECT site_id, method, config_json, created_at, updated_at
            FROM site_publish_config_old
            WHERE method IN ('disabled', 's3_compatible', 'rsync_ssh');
            "#,
        )
        .await?;
        txn.execute_unprepared(
            r#"
            INSERT INTO site_publish_run (
                id,
                site_id,
                method,
                status,
                actor_sub,
                rendered_file_count,
                published_file_count,
                deleted_object_count,
                error_message,
                created_at,
                started_at,
                finished_at
            )
            SELECT
                id,
                site_id,
                method,
                status,
                actor_sub,
                rendered_file_count,
                published_file_count,
                deleted_object_count,
                error_message,
                created_at,
                started_at,
                finished_at
            FROM site_publish_run_old
            WHERE method IN ('s3_compatible', 'rsync_ssh');
            "#,
        )
        .await?;

        txn.execute_unprepared("DROP INDEX IF EXISTS idx_site_publish_run_site_created;")
            .await?;
        txn.execute_unprepared(
            r#"
            CREATE INDEX idx_site_publish_run_site_created
            ON site_publish_run (site_id, created_at);
            "#,
        )
        .await?;

        txn.execute_unprepared("DROP TABLE site_publish_run_old;")
            .await?;
        txn.execute_unprepared("DROP TABLE site_publish_config_old;")
            .await?;
        txn.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let txn = db.begin().await?;

        txn.execute_unprepared(
            r#"
            ALTER TABLE site_publish_config RENAME TO site_publish_config_new;
            "#,
        )
        .await?;
        txn.execute_unprepared(
            r#"
            ALTER TABLE site_publish_run RENAME TO site_publish_run_new;
            "#,
        )
        .await?;

        txn.execute_unprepared(
            r#"
            CREATE TABLE site_publish_config (
                site_id TEXT NOT NULL PRIMARY KEY,
                method TEXT NOT NULL CHECK (method IN ('disabled', 's3_compatible')),
                config_json JSON NOT NULL,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                FOREIGN KEY (site_id) REFERENCES site (id) ON DELETE CASCADE
            );
            "#,
        )
        .await?;
        txn.execute_unprepared(
            r#"
            CREATE TABLE site_publish_run (
                id TEXT NOT NULL PRIMARY KEY,
                site_id TEXT NOT NULL,
                method TEXT NOT NULL CHECK (method IN ('s3_compatible')),
                status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
                actor_sub TEXT NOT NULL,
                rendered_file_count INTEGER NOT NULL,
                published_file_count INTEGER NOT NULL,
                deleted_object_count INTEGER NOT NULL,
                error_message TEXT,
                created_at TIMESTAMP NOT NULL,
                started_at TIMESTAMP,
                finished_at TIMESTAMP,
                FOREIGN KEY (site_id) REFERENCES site (id) ON DELETE CASCADE
            );
            "#,
        )
        .await?;

        txn.execute_unprepared(
            r#"
            INSERT INTO site_publish_config (site_id, method, config_json, created_at, updated_at)
            SELECT site_id, method, config_json, created_at, updated_at
            FROM site_publish_config_new
            WHERE method IN ('disabled', 's3_compatible');
            "#,
        )
        .await?;
        txn.execute_unprepared(
            r#"
            INSERT INTO site_publish_run (
                id,
                site_id,
                method,
                status,
                actor_sub,
                rendered_file_count,
                published_file_count,
                deleted_object_count,
                error_message,
                created_at,
                started_at,
                finished_at
            )
            SELECT
                id,
                site_id,
                method,
                status,
                actor_sub,
                rendered_file_count,
                published_file_count,
                deleted_object_count,
                error_message,
                created_at,
                started_at,
                finished_at
            FROM site_publish_run_new
            WHERE method IN ('s3_compatible');
            "#,
        )
        .await?;

        txn.execute_unprepared("DROP INDEX IF EXISTS idx_site_publish_run_site_created;")
            .await?;
        txn.execute_unprepared(
            r#"
            CREATE INDEX idx_site_publish_run_site_created
            ON site_publish_run (site_id, created_at);
            "#,
        )
        .await?;

        txn.execute_unprepared("DROP TABLE site_publish_run_new;")
            .await?;
        txn.execute_unprepared("DROP TABLE site_publish_config_new;")
            .await?;
        txn.commit().await
    }
}
