use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "audit_event")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub site_id: Option<String>,
    pub actor_sub: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub created_at: String,
    pub payload_json: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
