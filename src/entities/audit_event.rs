use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "audit_event")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub site_id: Option<Uuid>,
    pub actor_sub: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub payload_json: Option<Json>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
