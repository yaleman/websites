use chrono::DateTime as ChronoDateTime;
use chrono::Utc;
use sea_orm::prelude::StringLen;
use sea_orm::{ActiveModelBehavior, DeriveActiveEnum, EnumIter, entity::prelude::*};
use serde::{Deserialize, Serialize};

use super::site_publish_config::PublishMethod;

#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
#[serde(rename_all = "snake_case")]
pub enum PublishRunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "site_publish_run")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub site_id: Uuid,
    pub method: PublishMethod,
    pub status: PublishRunStatus,
    pub actor_sub: String,
    pub rendered_file_count: i32,
    pub published_file_count: i32,
    pub deleted_object_count: i32,
    pub error_message: Option<String>,
    pub created_at: ChronoDateTime<Utc>,
    pub started_at: Option<ChronoDateTime<Utc>>,
    pub finished_at: Option<ChronoDateTime<Utc>>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl std::fmt::Display for PublishRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PublishRunStatus::Queued => f.write_str("queued"),
            PublishRunStatus::Running => f.write_str("running"),
            PublishRunStatus::Succeeded => f.write_str("succeeded"),
            PublishRunStatus::Failed => f.write_str("failed"),
        }
    }
}
