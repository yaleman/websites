use chrono::DateTime as ChronoDateTime;
use chrono::Utc;
use sea_orm::entity::prelude::*;

use crate::errors::SiteError;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "site")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
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

/// Returns a single site by id.
pub async fn get_by_id(db: &DatabaseConnection, site_id: Uuid) -> Result<Model, SiteError> {
    let model = Entity::find_by_id(site_id)
        .one(db)
        .await?
        .ok_or(SiteError::NotFound)?;

    Ok(model)
}
