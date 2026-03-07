use sea_orm::prelude::StringLen;
use sea_orm::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
pub mod asset;
pub mod asset_variant;
pub mod audit_event;
pub mod content_alias;
pub mod content_item;
pub mod content_revision;
pub mod content_revision_alias;
pub mod content_revision_tag;
pub mod content_tag;
pub mod site;
pub mod site_membership;
pub mod tag;
pub mod user;

#[derive(EnumIter, DeriveActiveEnum, Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "lowercase"
)]
#[serde(rename_all = "lowercase")]
pub enum PageType {
    // #[sea_orm(string_value = "post")]
    Post,
    // #[sea_orm(string_value = "page")]
    Page,
}

impl TryFrom<&str> for PageType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "post" => Ok(PageType::Post),
            "page" => Ok(PageType::Page),
            _ => Err(format!("unsupported page type: {value}")),
        }
    }
}

impl AsRef<str> for PageType {
    fn as_ref(&self) -> &str {
        match self {
            PageType::Post => "post",
            PageType::Page => "page",
        }
    }
}

impl std::fmt::Display for PageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl std::str::FromStr for PageType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "post" => Ok(PageType::Post),
            "page" => Ok(PageType::Page),
            _ => Err(format!("unsupported page type: {value}")),
        }
    }
}

impl PageType {
    pub fn is_post(&self) -> bool {
        matches!(self, PageType::Post)
    }

    pub fn is_page(&self) -> bool {
        matches!(self, PageType::Page)
    }
}
