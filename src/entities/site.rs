use chrono::DateTime as ChronoDateTime;
use chrono::Utc;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "site")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub short_name: String,
    pub full_title: String,
    pub template_name: String,
    pub created_at: ChronoDateTime<Utc>,
    pub updated_at: Option<ChronoDateTime<Utc>>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
