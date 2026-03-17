use sea_orm::prelude::StringLen;
use sea_orm::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
pub mod asset;
pub mod asset_variant;
pub mod audit_event;
pub mod content_alias;
pub mod content_item;
pub mod content_revision;
pub mod content_revision_alias;
pub mod content_revision_tag;
pub mod content_tag;
pub mod setting;
pub mod site;
pub mod site_membership;
pub mod tag;
pub mod user;
pub mod user_api_token;

#[derive(
    EnumIter, DeriveActiveEnum, Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize, ToSchema,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "lowercase"
)]
#[serde(rename_all = "lowercase")]
pub enum PageType {
    Post,
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

    pub fn template(&self) -> &'static str {
        match self {
            PageType::Post => "post.html",
            PageType::Page => "page.html",
        }
    }

    pub fn css_class(&self) -> &'static str {
        match self {
            PageType::Post => "post",
            PageType::Page => "page",
        }
    }
}

#[test]
fn test_pagetype() {
    use std::str::FromStr;
    let post = PageType::Post;
    let page = PageType::Page;

    assert_eq!(post.as_ref(), "post");
    assert_eq!(page.as_ref(), "page");

    assert_eq!(post.to_string(), "post");
    assert_eq!(page.to_string(), "page");

    assert_eq!(
        PageType::from_str("post").expect("failed to parse page type"),
        PageType::Post
    );
    assert_eq!(
        PageType::from_str("page").expect("failed to parse page type"),
        PageType::Page
    );

    assert!(post.is_post());
    assert!(!post.is_page());

    assert!(!page.is_post());
    assert!(page.is_page());
}
