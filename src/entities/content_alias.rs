use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_alias")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub content_id: String,
    pub site_id: String,
    pub alias_path: String,
    pub kind: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
