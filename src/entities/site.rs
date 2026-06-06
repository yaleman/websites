use chrono::DateTime as ChronoDateTime;
use chrono::Utc;
use sea_orm::FromJsonQueryResult;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use crate::errors::SiteError;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "site")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub short_name: String,
    pub full_title: String,
    pub template_name: String,
    pub publish_on_render: bool,
    pub internal_domains: InternalDomains,
    pub mass_import_assets: Option<String>,
    pub created_at: ChronoDateTime<Utc>,
    pub updated_at: Option<ChronoDateTime<Utc>>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
#[serde(transparent)]
pub struct InternalDomains(pub Vec<String>);

impl InternalDomains {
    pub fn into_vec(self) -> Vec<String> {
        self.0
    }
}

impl From<Vec<String>> for InternalDomains {
    fn from(value: Vec<String>) -> Self {
        Self(value)
    }
}

impl std::ops::Deref for InternalDomains {
    type Target = Vec<String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<Vec<String>> for InternalDomains {
    fn eq(&self, other: &Vec<String>) -> bool {
        &self.0 == other
    }
}

impl PartialEq<InternalDomains> for Vec<String> {
    fn eq(&self, other: &InternalDomains) -> bool {
        self == &other.0
    }
}

/// Returns a single site by id.
pub async fn get_by_id(db: &DatabaseConnection, site_id: Uuid) -> Result<Model, SiteError> {
    let model = Entity::find_by_id(site_id)
        .one(db)
        .await?
        .ok_or(SiteError::NotFound)?;

    Ok(model)
}
