use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_item")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub site_id: String,
    pub page_type: String,
    pub title: String,
    pub slug: String,
    pub page_content: String,
    pub draft: bool,
    pub creator_sub: String,
    pub created_at: String,
    pub last_updated: String,
    pub published_at: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
