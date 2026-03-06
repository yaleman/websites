use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_revision_alias")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub revision_id: String,
    pub alias_path: String,
    pub kind: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
