use super::*;
use crate::entities::{PageType, content_item, content_revision_alias, content_revision_tag, tag};
use sea_orm::{ActiveModelTrait, EntityTrait, PaginatorTrait, QueryOrder, QuerySelect, Set};
use uuid::Uuid;

#[tokio::test]
async fn populated_tables_survive_down_and_up() {
    let db = crate::db::test_db_start().await;
    let site = crate::create_site(
        &db,
        "migration".into(),
        "Migration".into(),
        "default".into(),
    )
    .await
    .expect("create site");
    let mut content_ids = Vec::new();
    for slug in ["first", "second"] {
        let content = crate::create_content(
            &db,
            crate::NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: slug.into(),
                slug: slug.into(),
                page_content: "Body".into(),
                draft: true,
                creator_sub: "migration-test".into(),
                created_at: None,
                published_at: None,
            },
        )
        .await
        .expect("create content");
        let revision = crate::list_revisions(&db, content.id)
            .await
            .expect("list revisions")
            .into_iter()
            .next()
            .expect("initial revision");
        let tag = tag::ActiveModel {
            id: Set(Uuid::now_v7()),
            site_id: Set(site.id),
            name: Set(slug.into()),
        }
        .insert(&db)
        .await
        .expect("insert tag");
        content_revision_alias::ActiveModel {
            id: Set(Uuid::now_v7()),
            revision_id: Set(revision.id),
            content_id: Set(content.id),
            alias_path: Set(format!("/{slug}")),
            kind: Set("alias".into()),
        }
        .insert(&db)
        .await
        .expect("insert revision alias");
        content_revision_tag::ActiveModel {
            id: Set(Uuid::now_v7()),
            revision_id: Set(revision.id),
            content_id: Set(content.id),
            tag_id: Set(tag.id),
        }
        .insert(&db)
        .await
        .expect("insert revision tag");
        content_ids.push(content.id);
    }
    let aliases = content_revision_alias::Entity::find()
        .order_by_asc(content_revision_alias::Column::Id)
        .all(&db)
        .await
        .expect("load aliases");
    let tags = content_revision_tag::Entity::find()
        .order_by_asc(content_revision_tag::Column::Id)
        .all(&db)
        .await
        .expect("load revision tags");
    let schema = SchemaManager::new(&db);
    Migration.down(&schema).await.expect("downgrade migration");
    for table in ["content_revision_alias", "content_revision_tag"] {
        assert!(
            !schema
                .has_column(table, "content_id")
                .await
                .expect("check old schema")
        );
    }
    assert_eq!(
        content_revision_alias::Entity::find()
            .select_only()
            .column(content_revision_alias::Column::Id)
            .count(&db)
            .await
            .expect("count aliases"),
        2
    );
    assert_eq!(
        content_revision_tag::Entity::find()
            .select_only()
            .column(content_revision_tag::Column::Id)
            .count(&db)
            .await
            .expect("count tags"),
        2
    );
    Migration.up(&schema).await.expect("upgrade migration");
    assert_eq!(
        content_revision_alias::Entity::find()
            .order_by_asc(content_revision_alias::Column::Id)
            .all(&db)
            .await
            .expect("reload aliases"),
        aliases
    );
    assert_eq!(
        content_revision_tag::Entity::find()
            .order_by_asc(content_revision_tag::Column::Id)
            .all(&db)
            .await
            .expect("reload tags"),
        tags
    );
    for content_id in content_ids {
        content_item::Entity::delete_by_id(content_id)
            .exec(&db)
            .await
            .expect("delete content");
    }
    assert_eq!(
        content_revision_alias::Entity::find()
            .count(&db)
            .await
            .expect("count remaining aliases"),
        0
    );
    assert_eq!(
        content_revision_tag::Entity::find()
            .count(&db)
            .await
            .expect("count remaining tags"),
        0
    );
}
