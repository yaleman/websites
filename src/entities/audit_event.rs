use sea_orm::{ActiveValue::Set, entity::prelude::*};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "audit_event")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
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

/// Records an audit event for administrative actions.
pub async fn log_audit_event(
    db: &DatabaseConnection,
    actor_sub: &str,
    event_type: &str,
    entity_type: &str,
    entity_id: &str,
    site_id: Option<Uuid>,
    payload_json: Option<serde_json::Value>,
) -> Result<Model, String> {
    let model = ActiveModel {
        id: Set(Uuid::now_v7()),
        site_id: Set(site_id),
        actor_sub: Set(actor_sub.to_string()),
        event_type: Set(event_type.to_string()),
        entity_type: Set(entity_type.to_string()),
        entity_id: Set(entity_id.to_string()),
        created_at: Set(chrono::Utc::now()),
        payload_json: Set(payload_json),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}
