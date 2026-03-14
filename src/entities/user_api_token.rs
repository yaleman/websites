use chrono::DateTime as ChronoDateTime;
use chrono::Utc;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "user_api_token")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub issued_by_user_id: Uuid,
    pub label: String,
    pub jwt_id: String,
    pub grants_json: Option<Json>,
    pub created_at: ChronoDateTime<Utc>,
    pub last_used_at: Option<ChronoDateTime<Utc>>,
    pub inactive_expires_at: ChronoDateTime<Utc>,
    pub revoked_at: Option<ChronoDateTime<Utc>>,
    pub revoked_by_user_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
