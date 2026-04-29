use chrono::DateTime as ChronoDateTime;
use chrono::Utc;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "theme_registry")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub slug: String,
    pub repo_url: String,
    pub branch: Option<String>,
    pub ssh_key_name: Option<String>,
    pub created_at: ChronoDateTime<Utc>,
    pub updated_at: ChronoDateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
