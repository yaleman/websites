use std::str::FromStr;

use chrono::DateTime as ChronoDateTime;
use chrono::Utc;
use sea_orm::prelude::StringLen;
use sea_orm::{ActiveModelBehavior, DeriveActiveEnum, EnumIter, entity::prelude::*};
use serde::{Deserialize, Serialize};

use crate::errors::SiteError;

#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
#[serde(rename_all = "snake_case")]
pub enum PublishMethod {
    Disabled,
    S3Compatible,
    RsyncSsh,
}

impl PublishMethod {
    pub fn label(&self) -> &'static str {
        match self {
            PublishMethod::Disabled => "Disabled",
            PublishMethod::S3Compatible => "S3-compatible store",
            PublishMethod::RsyncSsh => "rsync over SSH",
        }
    }
}

impl PublishMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            PublishMethod::Disabled => "disabled",
            PublishMethod::S3Compatible => "s3_compatible",
            PublishMethod::RsyncSsh => "rsync_ssh",
        }
    }
}

impl FromStr for PublishMethod {
    type Err = SiteError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let method = s.trim();
        if method == "disabled" || method.is_empty() {
            Ok(PublishMethod::Disabled)
        } else if method == "s3_compatible" {
            Ok(PublishMethod::S3Compatible)
        } else if method == "rsync_ssh" {
            Ok(PublishMethod::RsyncSsh)
        } else {
            Err(SiteError::BadRequest(format!(
                "unsupported publish method: {method}"
            )))
        }
    }
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "site_publish_config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub site_id: Uuid,
    pub method: PublishMethod,
    pub config_json: Json,
    pub created_at: ChronoDateTime<Utc>,
    pub updated_at: ChronoDateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl std::fmt::Display for PublishMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PublishMethod::Disabled => f.write_str("disabled"),
            PublishMethod::S3Compatible => f.write_str("s3_compatible"),
            PublishMethod::RsyncSsh => f.write_str("rsync_ssh"),
        }
    }
}
