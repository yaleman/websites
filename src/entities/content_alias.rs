use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_alias")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub content_id: Uuid,
    pub site_id: Uuid,
    pub alias_path: String,
    pub kind: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
