use sea_orm::prelude::StringLen;
use sea_orm::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
pub mod asset;
pub mod asset_variant;
pub mod audit_event;
pub mod content_alias;
pub mod content_item;
pub mod content_revision;
pub mod content_revision_alias;
pub mod content_revision_tag;
pub mod content_tag;
pub mod setting;
pub mod site;
pub mod site_membership;
pub mod site_publish_config;
pub mod site_publish_run;
pub mod tag;
pub mod theme_registry;
pub mod user;
pub mod user_api_token;

#[derive(
    EnumIter, DeriveActiveEnum, Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize, ToSchema,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "lowercase"
)]
#[serde(rename_all = "lowercase")]
pub enum PageType {
    Post,
    Page,
}

impl TryFrom<&str> for PageType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "post" => Ok(PageType::Post),
            "page" => Ok(PageType::Page),
            _ => Err(format!("unsupported page type: {value}")),
        }
    }
}

impl AsRef<str> for PageType {
    fn as_ref(&self) -> &str {
        match self {
            PageType::Post => "post",
            PageType::Page => "page",
        }
    }
}

impl std::fmt::Display for PageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl std::str::FromStr for PageType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "post" => Ok(PageType::Post),
            "page" => Ok(PageType::Page),
            _ => Err(format!("unsupported page type: {value}")),
        }
    }
}

impl PageType {
    pub fn is_post(&self) -> bool {
        matches!(self, PageType::Post)
    }

    pub fn is_page(&self) -> bool {
        matches!(self, PageType::Page)
    }

    pub fn template(&self) -> &'static str {
        match self {
            PageType::Post => "post.html",
            PageType::Page => "page.html",
        }
    }

    pub fn css_class(&self) -> &'static str {
        match self {
            PageType::Post => "post",
            PageType::Page => "page",
        }
    }
}

#[test]
fn test_pagetype() {
    use std::str::FromStr;
    let post = PageType::Post;
    let page = PageType::Page;

    assert_eq!(post.as_ref(), "post");
    assert_eq!(page.as_ref(), "page");

    assert_eq!(post.to_string(), "post");
    assert_eq!(page.to_string(), "page");

    assert_eq!(
        PageType::from_str("post").expect("failed to parse page type"),
        PageType::Post
    );
    assert_eq!(
        PageType::from_str("page").expect("failed to parse page type"),
        PageType::Page
    );

    assert!(post.is_post());
    assert!(!post.is_page());

    assert!(!page.is_post());
    assert!(page.is_page());
}

#[cfg(test)]
mod revision_entity_tests {
    use super::*;
    use crate::db::test_db_start;
    use crate::{
        NewAlias, NewContent, NewContentTag, PageType, UpdateContent, add_content_tag,
        create_alias, create_content, create_site, update_content,
    };
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    async fn create_revision_fixture(
        db: &sea_orm::DatabaseConnection,
        slug: &str,
    ) -> crate::entities::content_item::Model {
        let site = create_site(
            db,
            "revision-site".to_string(),
            "Revision Site".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create site");

        create_content(
            db,
            NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: "Revision Post".to_string(),
                slug: slug.to_string(),
                page_content: "Initial body".to_string(),
                draft: true,
                creator_sub: "creator".to_string(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create content")
    }

    #[tokio::test]
    async fn content_revision_loads_parent_content_item() {
        let db = test_db_start().await;
        let content = create_revision_fixture(&db, "revision-post").await;

        let (revision, parent_content) = content_revision::Entity::find()
            .filter(content_revision::Column::ContentId.eq(content.id))
            .find_also_related(content_item::Entity)
            .one(&db)
            .await
            .expect("failed to load revision with parent content")
            .expect("missing revision row");

        assert_eq!(revision.content_id, content.id);
        assert_eq!(
            parent_content.expect("missing related content").id,
            content.id
        );
    }

    #[tokio::test]
    async fn content_revision_alias_loads_parent_revision_and_content_item() {
        let db = test_db_start().await;
        let content = create_revision_fixture(&db, "alias-post").await;

        let alias = create_alias(
            &db,
            NewAlias {
                content_id: content.id,
                site_id: content.site_id,
                alias_path: "/legacy/alias-post".to_string(),
                kind: "alias".to_string(),
            },
        )
        .await
        .expect("failed to create content alias");

        update_content(
            &db,
            UpdateContent {
                content_id: content.id,
                page_type: None,
                title: Some("Alias Post Updated".to_string()),
                slug: Some("alias-post-updated".to_string()),
                page_content: Some("Updated body".to_string()),
                draft: Some(false),
                published_at: None,
                editor_sub: "editor".to_string(),
            },
        )
        .await
        .expect("failed to update content");

        let (revision_alias, parent_revision) = content_revision_alias::Entity::find()
            .filter(content_revision_alias::Column::AliasPath.eq(alias.alias_path.clone()))
            .find_also_related(content_revision::Entity)
            .one(&db)
            .await
            .expect("failed to load revision alias with parent revision")
            .expect("missing revision alias row");

        assert_eq!(revision_alias.content_id, content.id);
        assert_eq!(revision_alias.alias_path, alias.alias_path);
        assert_eq!(
            parent_revision
                .expect("missing related revision")
                .content_id,
            content.id
        );

        let (_, parent_content) = content_revision_alias::Entity::find()
            .filter(content_revision_alias::Column::AliasPath.eq("/legacy/alias-post"))
            .find_also_related(content_item::Entity)
            .one(&db)
            .await
            .expect("failed to load revision alias with content item")
            .expect("missing revision alias row for content join");

        assert_eq!(
            parent_content.expect("missing related content").id,
            content.id
        );
    }

    #[tokio::test]
    async fn content_revision_tag_loads_parent_revision_content_and_tag() {
        let db = test_db_start().await;
        let content = create_revision_fixture(&db, "tag-post").await;

        let tag_link = add_content_tag(
            &db,
            NewContentTag {
                content_id: content.id,
                site_id: content.site_id,
                tag_name: "Docs".to_string(),
            },
        )
        .await
        .expect("failed to create content tag");

        update_content(
            &db,
            UpdateContent {
                content_id: content.id,
                page_type: None,
                title: Some("Tag Post Updated".to_string()),
                slug: Some("tag-post-updated".to_string()),
                page_content: Some("Updated body".to_string()),
                draft: Some(false),
                published_at: None,
                editor_sub: "editor".to_string(),
            },
        )
        .await
        .expect("failed to update content");

        let revision = content_revision::Entity::find()
            .filter(content_revision::Column::ContentId.eq(content.id))
            .filter(content_revision::Column::RevisionNumber.eq(2))
            .one(&db)
            .await
            .expect("failed to load revision 2")
            .expect("missing revision 2");

        let (revision_tag, parent_revision) = content_revision_tag::Entity::find()
            .filter(content_revision_tag::Column::RevisionId.eq(revision.id))
            .find_also_related(content_revision::Entity)
            .one(&db)
            .await
            .expect("failed to load revision tag with parent revision")
            .expect("missing revision tag row");

        assert_eq!(revision_tag.content_id, content.id);
        assert_eq!(revision_tag.tag_id, tag_link.tag_id);
        assert_eq!(
            parent_revision.expect("missing related revision").id,
            revision.id
        );

        let (_, parent_content) = content_revision_tag::Entity::find()
            .filter(content_revision_tag::Column::RevisionId.eq(revision.id))
            .find_also_related(content_item::Entity)
            .one(&db)
            .await
            .expect("failed to load revision tag with content item")
            .expect("missing revision tag row for content join");

        assert_eq!(
            parent_content.expect("missing related content").id,
            content.id
        );

        let (_, related_tag) = content_revision_tag::Entity::find()
            .filter(content_revision_tag::Column::RevisionId.eq(revision.id))
            .find_also_related(tag::Entity)
            .one(&db)
            .await
            .expect("failed to load revision tag with tag")
            .expect("missing revision tag row for tag join");

        assert_eq!(
            related_tag.expect("missing related tag").name,
            "Docs".to_string()
        );
    }
}
