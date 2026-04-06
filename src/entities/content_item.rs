use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::entities::PageType;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, DeriveEntityModel)]
#[sea_orm(table_name = "content_item")]
#[schema(
    title = "ContentItem",
    description = "A content item, such as a page or blog post.",
    as = crate::entities::content_item::Model
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub site_id: Uuid,
    pub page_type: PageType,
    pub title: String,
    pub slug: String,
    pub page_content: String,
    pub draft: bool,
    pub creator_sub: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_updated: Option<chrono::DateTime<chrono::Utc>>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::site::Entity",
        from = "Column::SiteId",
        to = "super::site::Column::Id",
        on_delete = "Cascade"
    )]
    Site,
}

impl Related<super::site::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Site.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// Returns the publish timestamp of the content item, which is either the published_at timestamp if it exists, or the created_at timestamp if published_at is None.
    pub fn content_publish_timestamp(&self) -> String {
        self.published_at.unwrap_or(self.created_at).to_rfc2822()
    }
}
