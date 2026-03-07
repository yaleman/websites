use sea_orm::entity::prelude::*;

use crate::entities::PageType;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_item")]
pub struct Model {
    #[sea_orm(primary_key)]
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
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    pub fn content_publish_timestamp(&self) -> String {
        self.published_at.unwrap_or(self.created_at).to_rfc3339()
    }

    pub fn content_publish_timestamp_rfc2822(&self) -> String {
        self.published_at.unwrap_or(self.created_at).to_rfc2822()
    }
}
