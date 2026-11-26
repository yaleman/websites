use super::*;
use crate::entities::{site, site_publish_config as config, site_publish_run as run};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QueryOrder, Set};
use uuid::Uuid;

#[tokio::test]
async fn rebuild_preserves_records_and_filters_rsync_on_downgrade() {
    let db = crate::db::test_db_start().await;
    let mut configs = Vec::new();
    let mut runs = Vec::new();
    for method in [
        config::PublishMethod::Disabled,
        config::PublishMethod::S3Compatible,
        config::PublishMethod::RsyncSsh,
    ] {
        let site = crate::create_site(
            &db,
            method.as_str().into(),
            "Publish".into(),
            "default".into(),
        )
        .await
        .expect("create site");
        let now = Utc::now();
        let row = config::ActiveModel {
            site_id: Set(site.id),
            method: Set(method),
            config_json: Set(serde_json::json!({"path": "a/quoted'path", "nested": [1, 2]})),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert config");
        if method != config::PublishMethod::RsyncSsh {
            configs.push(row);
        }
        if method == config::PublishMethod::Disabled {
            continue;
        }
        for status in [
            run::PublishRunStatus::Queued,
            run::PublishRunStatus::Running,
            run::PublishRunStatus::Succeeded,
            run::PublishRunStatus::Failed,
        ] {
            let row = run::ActiveModel {
                id: Set(Uuid::now_v7()),
                site_id: Set(site.id),
                method: Set(method),
                status: Set(status),
                actor_sub: Set("migration-test".into()),
                rendered_file_count: Set(12),
                published_file_count: Set(10),
                deleted_object_count: Set(2),
                error_message: Set(
                    (status == run::PublishRunStatus::Failed).then(|| "failed 'publish'".into())
                ),
                created_at: Set(now),
                started_at: Set((status != run::PublishRunStatus::Queued).then_some(now)),
                finished_at: Set(matches!(
                    status,
                    run::PublishRunStatus::Succeeded | run::PublishRunStatus::Failed
                )
                .then_some(now)),
            }
            .insert(&db)
            .await
            .expect("insert run");
            if method == config::PublishMethod::S3Compatible {
                runs.push(row);
            }
        }
    }
    let schema = SchemaManager::new(&db);
    Migration
        .down(&schema)
        .await
        .expect("downgrade publish schema");
    configs.sort_by_key(|row| row.site_id);
    assert_eq!(
        config::Entity::find()
            .order_by_asc(config::Column::SiteId)
            .all(&db)
            .await
            .expect("load configs"),
        configs
    );
    assert_eq!(
        run::Entity::find()
            .order_by_asc(run::Column::Id)
            .all(&db)
            .await
            .expect("load runs"),
        runs
    );

    let mut rsync_config = configs
        .first()
        .expect("retained config")
        .clone()
        .into_active_model();
    rsync_config.method = Set(config::PublishMethod::RsyncSsh);
    assert!(rsync_config.clone().update(&db).await.is_err());
    let mut rsync_run = runs
        .first()
        .expect("retained run")
        .clone()
        .into_active_model();
    rsync_run.method = Set(config::PublishMethod::RsyncSsh);
    assert!(rsync_run.clone().update(&db).await.is_err());
    Migration.up(&schema).await.expect("upgrade publish schema");
    assert_eq!(
        config::Entity::find()
            .order_by_asc(config::Column::SiteId)
            .all(&db)
            .await
            .expect("reload configs"),
        configs
    );
    assert_eq!(
        run::Entity::find()
            .order_by_asc(run::Column::Id)
            .all(&db)
            .await
            .expect("reload runs"),
        runs
    );
    rsync_config
        .update(&db)
        .await
        .expect("accept rsync config after upgrade");
    rsync_run
        .update(&db)
        .await
        .expect("accept rsync run after upgrade");

    site::Entity::delete_many()
        .exec(&db)
        .await
        .expect("delete sites");
    assert!(
        config::Entity::find()
            .all(&db)
            .await
            .expect("remaining configs")
            .is_empty()
    );
    assert!(
        run::Entity::find()
            .all(&db)
            .await
            .expect("remaining runs")
            .is_empty()
    );
}

#[tokio::test]
async fn rebuild_rolls_back_partial_schema_changes() {
    let db = crate::db::test_db_start().await;
    let schema = SchemaManager::new(&db);
    schema
        .create_table(
            Table::create()
                .table(SitePublishRun::OldTable)
                .col(ColumnDef::new(SitePublishRun::Id).uuid())
                .to_owned(),
        )
        .await
        .expect("create conflicting temporary table");
    assert!(Migration.up(&schema).await.is_err());
    assert!(
        schema
            .has_table(SitePublishConfig::Table.to_string())
            .await
            .expect("original config table exists")
    );
    assert!(
        !schema
            .has_table(SitePublishConfig::OldTable.to_string())
            .await
            .expect("config rename rolled back")
    );
    assert!(
        schema
            .has_table(SitePublishRun::Table.to_string())
            .await
            .expect("original run table exists")
    );
}
