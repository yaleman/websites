use sea_orm::entity::prelude::*;

use crate::entities::PageType;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_revision")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub content_id: Uuid,
    pub site_id: Uuid,
    pub revision_number: i32,
    pub title: String,
    pub slug: String,
    pub page_content: String,
    pub draft: bool,
    pub page_type: PageType,
    pub editor_sub: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::content_item::Entity",
        from = "Column::ContentId",
        to = "super::content_item::Column::Id",
        on_delete = "Cascade"
    )]
    ContentItem,
}

impl Related<super::content_item::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ContentItem.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
