use crate::api_docs::ApiDoc;
use crate::constants::{
    CUSTOMIZABLE_TEMPLATE_FILES, DEFAULT_TEMPLATE_NAME, SESSION_OIDC_NONCE_KEY,
    SESSION_OIDC_PKCE_KEY, SESSION_OIDC_STATE_KEY, SESSION_USER,
};
use crate::content_scan::{
    AssetReference, ContentScanReport, ScanAction, ScanContext, ScanIssue, format_asset_shortcode,
    scan_content,
};
use crate::csrf::SessionCsrfExt;
use crate::entities::audit_event::log_audit_event;
use crate::entities::site::get_by_id;
use crate::entities::user::upsert_user_login;
use crate::entities::{self, PageType};
use crate::errors::SiteError;
use crate::images::{generate_thumbnail, mime_from_extension};
use crate::middleware::log_requests;
use crate::oidc::{admin_login_callback, build_http_client, build_oidc_client};
use crate::tls::build_tls_config;
use crate::{
    NewAsset, NewAssetVariant, NewContent, cli::OidcConfig, content_primary_route, create_asset,
    create_asset_variant, create_content, create_membership, create_site, create_tag,
    delete_membership, delete_site, delete_tag, export_site, get_membership_by_id,
    get_membership_for_subject, get_revision, get_revision_by_number, get_user_by_id,
    import_site_json, list_aliases, list_assets, list_content, list_content_tags, list_memberships,
    list_memberships_for_user_id, list_revisions, list_sites, list_sites_for_subject, list_tags,
    list_users, list_users_by_ids, render_content_preview, render_site,
    resolve_site_template_override_root, resolve_upload_root, search_all_content, search_content,
    serialize_site_export_pretty, sync_tags_to_content, update_content, update_membership_role,
    update_site_settings,
};
use anyhow::Context;
use askama::Template;
use askama_web::WebTemplate;
use axum::middleware::from_fn;
use axum::{
    Json, Router,
    body::Body,
    extract::{Form, Multipart, OriginalUri, Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use chrono::{DateTime, Utc};
use openidconnect::ClientSecret;
use openidconnect::{
    ClientId, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge, Scope, core::CoreAuthenticationFlow,
};
use sea_orm::prelude::StringLen;

use sea_orm::{
    ColumnTrait as _, Condition, ConnectionTrait, DatabaseConnection, DeriveActiveEnum,
    EntityTrait, EnumIter, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use similar::TextDiff;
use std::collections::{HashMap, HashSet};
use std::env;
use std::net::SocketAddr;
use std::path::{Component, Path as StdPath, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tower_http::services::ServeDir;
use tower_sessions::{Expiry, Session, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;
use tracing::{error, info};
use url::Url;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

/// Holds all the common template-shared data for admin pages.
struct AdminTemplateData {
    page_title: String,
    document_title: String,

    /// Feedback to the user
    page_message: Option<String>,
    /// Render the page message as a toast notification instead of an inline banner.
    page_message_is_toast: bool,
    /// Query parameter to clear after displaying a toast message.
    clear_query_param: Option<String>,
    /// Used when you're in a site context, to link back to the site homepage, e.g. in the header.
    site_id: Option<Uuid>,
    /// Full site title shown in shared admin chrome for site-scoped pages.
    site_full_title: Option<String>,
    /// Extra "actions" links in a secondary navbar, e.g. "New site", "Back to sites", etc.
    links: Vec<AdminLink>,
    nav_search_action: String,
    nav_search_value: String,
}

impl AdminTemplateData {
    pub fn new(title: impl ToString) -> Self {
        let page_title = title.to_string();
        Self {
            document_title: page_title.clone(),
            page_title,
            page_message: None,
            page_message_is_toast: false,
            clear_query_param: None,
            site_id: None,
            site_full_title: None,
            links: vec![],
            nav_search_action: "/admin/search".to_string(),
            nav_search_value: String::new(),
        }
    }

    pub fn with_message(self, message: impl ToString) -> Self {
        Self {
            page_message: Some(message.to_string()),
            page_message_is_toast: false,
            ..self
        }
    }

    pub fn with_toast_message(
        self,
        message: impl ToString,
        clear_query_param: impl ToString,
    ) -> Self {
        Self {
            page_message: Some(message.to_string()),
            page_message_is_toast: true,
            clear_query_param: Some(clear_query_param.to_string()),
            ..self
        }
    }

    pub fn with_site_context(self, site_id: Uuid, site_full_title: impl ToString) -> Self {
        let site_full_title = site_full_title.to_string();
        Self {
            document_title: format!("{} - {}", self.page_title, site_full_title),
            site_id: Some(site_id),
            site_full_title: Some(site_full_title),
            nav_search_action: format!("/admin/site/{site_id}/search"),
            ..self
        }
    }

    pub fn with_links(self, links: Vec<AdminLink>) -> Self {
        Self { links, ..self }
    }

    pub fn with_nav_search_value(self, value: impl ToString) -> Self {
        Self {
            nav_search_value: value.to_string(),
            ..self
        }
    }
}

#[derive(Clone)]
pub(crate) struct AdminState {
    pub(crate) db: Arc<DatabaseConnection>,
    pub(crate) oidc_client_id: ClientId,
    pub(crate) oidc_client_secret: Option<ClientSecret>,
    pub(crate) oidc_frontend_url: Url,
    pub(crate) oidc_discovery_url: IssuerUrl,
    pub(crate) oidc_client: Arc<reqwest::Client>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_index.html")]
struct AdminIndexTemplate {
    template_shared: AdminTemplateData,
    sites: Vec<crate::entities::site::Model>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "not_found.html")]
struct NotFoundTemplate {
    requested_path: String,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_sites_new.html")]
struct AdminSitesNewTemplate {
    template_shared: AdminTemplateData,

    templates: Vec<String>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_sites_import.html")]
struct AdminSitesImportTemplate {
    template_shared: AdminTemplateData,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_search.html")]
struct AdminSearchTemplate {
    template_shared: AdminTemplateData,
    rows: Vec<AdminSearchRow>,
    show_site_column: bool,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_detail.html")]
struct AdminContentDetailTemplate {
    template_shared: AdminTemplateData,
    title: String,
    page_type: PageType,
    status: String,
    primary_route: String,
    revisions_summary: String,
    tags: Vec<String>,
    aliases: Vec<AdminContentAliasRow>,
    creator_sub: String,
    content_id: Uuid,
    site_id: Uuid,
    slug: String,
    created_at: String,
    updated_at: String,
    published_at: String,
    page_content: String,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_revisions.html")]
struct AdminContentRevisionsTemplate {
    template_shared: AdminTemplateData,

    rows: Vec<AdminRow>,
    inline_body: String,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_revision_diff.html")]
struct AdminRevisionDiffTemplate {
    template_shared: AdminTemplateData,

    rows: Vec<AdminRow>,
    pre_body: String,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_assets.html")]
struct AdminAssetsTemplate {
    template_shared: AdminTemplateData,
    site_id: Uuid,
    site_full_title: String,
    assets: Vec<AdminAssetRow>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_assets_new.html")]
struct AdminAssetsNewTemplate {
    template_shared: AdminTemplateData,
    site_id: Uuid,
    site_full_title: String,
    recent_assets: Vec<AdminAssetRow>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_render.html")]
struct AdminRenderTemplate {
    template_shared: AdminTemplateData,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_list.html")]
struct AdminContentListTemplate {
    template_shared: AdminTemplateData,

    site_id: Uuid,
    page_type_options: Vec<AdminSelectOption>,
    current_sort_by: &'static str,
    sort_headers: Vec<AdminContentListSortHeader>,
    content_rows: Vec<AdminContentListRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_tags.html")]
struct AdminTagsTemplate {
    template_shared: AdminTemplateData,
    site_id: Uuid,
    tags: Vec<AdminSiteTagRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "content_new.html")]
struct AdminContentNewTemplate {
    template_shared: AdminTemplateData,
    tags: Vec<AdminTagOption>,
    page_content: String,
    site_id: Uuid,
    allow_external_image: bool,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_source.html")]
struct AdminContentSourceTemplate {
    template_shared: AdminTemplateData,
    tags: Vec<AdminTagOption>,
    title: String,
    slug: String,
    page_type: String,
    draft: bool,
    published_at: String,
    page_content: String,
    site_id: Uuid,
    allow_external_image: bool,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_scan.html")]
struct AdminContentScanTemplate {
    template_shared: AdminTemplateData,
    site_id: Uuid,
    domains: String,
    scan_limit: usize,
    filter_options: Vec<AdminSelectOption>,
    current_filter: String,
    results: Vec<AdminContentScanResult>,
    summary: Option<AdminContentScanSummary>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "site_settings.html")]
struct AdminSiteSettingsTemplate {
    template_shared: AdminTemplateData,

    site_id: Uuid,
    site_short_name: String,
    full_title: String,
    template_name: String,
    can_delete_site: bool,
    export_href: Option<String>,
    templates: Vec<String>,
    template_files: Vec<AdminSiteTemplateFileRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "site_delete_confirm.html")]
struct AdminSiteDeleteConfirmTemplate {
    template_shared: AdminTemplateData,

    site_id: Uuid,
    site_full_title: String,
    site_short_name: String,
    csrf_token: String,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "site_template_editor.html")]
struct AdminSiteTemplateEditorTemplate {
    template_shared: AdminTemplateData,
    site_id: Uuid,
    site_full_title: String,
    template_name: String,
    file_name: String,
    source: String,
    source_origin: String,
    override_exists: bool,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "memberships.html")]
struct AdminMembershipsTemplate {
    template_shared: AdminTemplateData,

    site_id: Uuid,
    site_full_title: String,
    roles: Vec<SiteRole>,
    memberships: Vec<AdminMembershipRow>,
    membership_candidates: Vec<AdminMembershipCandidateRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_user_profile.html")]
struct AdminUserProfileTemplate {
    template_shared: AdminTemplateData,

    user_id: Uuid,
    display_name: String,
    subject: String,
    email: String,
    created_at: String,
    last_login_at: String,
    is_admin: bool,
    memberships: Vec<AdminUserMembershipRow>,
}

#[derive(Debug)]
struct AdminRow {
    label: String,
    value: String,
}

#[derive(Debug)]
struct AdminContentAliasRow {
    kind: String,
    path: String,
}

#[derive(Debug)]
struct AdminTagOption {
    name: String,
    selected: bool,
}

#[derive(Debug)]
struct AdminSelectOption {
    label: &'static str,
    value: &'static str,
    selected: bool,
}

#[derive(Debug)]
struct AdminSearchRow {
    site_title: String,
    edit_href: String,
    title: String,
    created_at: String,
}

#[derive(Debug)]
struct AdminContentListRow {
    edit_href: String,
    title: String,
    page_type: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug)]
struct AdminContentListSortHeader {
    label: &'static str,
    href: String,
    indicator: &'static str,
}

#[derive(Debug)]
struct AdminSiteTagRow {
    id: Uuid,
    name: String,
    delete_href: String,
}

#[derive(Debug)]
struct AdminMembershipRow {
    subject: String,
    email: Option<String>,
    role: SiteRole,
    profile_href: Option<String>,
    update_href: String,
    remove_href: String,
}

#[derive(Debug)]
struct AdminMembershipCandidateRow {
    user_id: Uuid,
    subject: String,
    email: Option<String>,
    search_value: String,
}

#[derive(Debug)]
struct AdminUserMembershipRow {
    site_title: String,
    site_short_name: String,
    role: SiteRole,
    site_href: String,
}

#[derive(Debug)]
struct AdminAssetRow {
    id: Uuid,
    original_filename: String,
    storage_basename: String,
    uploader_sub: String,
    mime_type: String,
    byte_length: i32,
    dimensions: String,
    created_at: String,
    original_url: String,
    thumbnail_url: Option<String>,
}

#[derive(Debug)]
struct AdminContentScanResult {
    title: String,
    edit_href: String,
    detail_href: String,
    issue_count: usize,
    fixable_count: usize,
    review_count: usize,
    issues: Vec<AdminContentScanIssueRow>,
}

#[derive(Debug)]
struct AdminContentScanIssueRow {
    issue_id: String,
    kind: String,
    label: String,
    current_value: String,
    proposed_value: String,
    snippet: String,
    can_apply: bool,
    selected: bool,
    needs_asset: bool,
    can_import_remote: bool,
    remote_url: String,
    selected_asset_id: String,
    selected_asset_label: String,
}

#[derive(Debug)]
struct AdminContentScanSummary {
    inspected_count: usize,
    result_count: usize,
    issue_count: usize,
    applied_count: usize,
    updated_items: Vec<AdminContentScanUpdatedItem>,
    skipped_messages: Vec<String>,
}

#[derive(Debug)]
struct AdminContentScanUpdatedItem {
    title: String,
    applied_count: usize,
}

#[derive(Debug)]
struct AdminSiteTemplateFileRow {
    file_name: String,
    source_origin: String,
    edit_href: String,
    reset_href: String,
    override_exists: bool,
}

#[derive(Debug)]
struct AdminLink {
    href: String,
    label: String,
    target: Option<String>,
}

impl AdminLink {
    fn new(href: &str, label: &str) -> Self {
        Self {
            href: href.to_string(),
            label: label.to_string(),
            target: None,
        }
    }
    fn with_target_blank(self) -> Self {
        Self {
            target: Some("_blank".to_string()),
            ..self
        }
    }
}

#[derive(Debug, Deserialize)]
struct CreateSiteForm {
    short_name: String,
    full_title: String,
    template_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateSiteSettingsForm {
    full_title: String,
    template_name: String,
}

#[derive(Debug, Deserialize)]
struct CsrfTokenForm {
    csrf_token: String,
}

#[derive(Debug, Deserialize)]
struct DashboardQuery {
    imported: Option<String>,
    deleted: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AdminContentListQuery {
    page_type: Option<String>,
    sort_by: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateSiteTemplateOverrideForm {
    source: String,
}

#[derive(Debug, Deserialize)]
struct CreateContentForm {
    page_type: String,
    title: String,
    slug: String,
    page_content: String,
    draft: Option<bool>,
    tag_list: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateContentForm {
    page_type: String,
    title: String,
    slug: String,
    page_content: String,
    draft: String,
    published_at: Option<String>,
    tag_list: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateTagForm {
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MembershipCreateForm {
    subject: String,
    user_id: Option<Uuid>,
    role: SiteRole,
}

#[derive(Debug, Deserialize)]
struct MembershipUpdateForm {
    role: SiteRole,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SourceEditorQuery {
    saved: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentScanForm {
    domains: String,
    scan_limit: Option<usize>,
    filter: Option<String>,
    content_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct ContentScanApplyForm {
    domains: String,
    scan_limit: Option<usize>,
    filter: Option<String>,
    selected_issue_ids_json: Option<String>,
    remote_import_issue_ids_json: Option<String>,
    asset_selections_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ManualAssetSelection {
    asset_id: Uuid,
    variant: String,
    asset_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentListPageTypeFilter {
    All,
    Page,
    Post,
}

impl ContentListPageTypeFilter {
    fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("page") => Self::Page,
            Some("post") => Self::Post,
            _ => Self::All,
        }
    }

    fn page_type(self) -> Option<PageType> {
        match self {
            Self::All => None,
            Self::Page => Some(PageType::Page),
            Self::Post => Some(PageType::Post),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Page => "page",
            Self::Post => "post",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All types",
            Self::Page => "Pages",
            Self::Post => "Posts",
        }
    }

    fn options(self) -> Vec<AdminSelectOption> {
        [Self::All, Self::Page, Self::Post]
            .into_iter()
            .map(|option| AdminSelectOption {
                label: option.label(),
                value: option.as_str(),
                selected: option == self,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentListSortBy {
    TitleAsc,
    TitleDesc,
    TypeAsc,
    TypeDesc,
    CreatedDesc,
    CreatedAsc,
    UpdatedDesc,
    UpdatedAsc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentListSortColumn {
    Title,
    Type,
    Created,
    Updated,
}

impl ContentListSortBy {
    fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("title_asc") => Self::TitleAsc,
            Some("title_desc") => Self::TitleDesc,
            Some("type_asc") => Self::TypeAsc,
            Some("type_desc") => Self::TypeDesc,
            Some("created_asc") | Some("oldest") => Self::CreatedAsc,
            Some("updated_desc") => Self::UpdatedDesc,
            Some("updated_asc") => Self::UpdatedAsc,
            Some("created_desc") | Some("newest") | None => Self::CreatedDesc,
            Some(_) => Self::CreatedDesc,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::TitleAsc => "title_asc",
            Self::TitleDesc => "title_desc",
            Self::TypeAsc => "type_asc",
            Self::TypeDesc => "type_desc",
            Self::CreatedDesc => "created_desc",
            Self::CreatedAsc => "created_asc",
            Self::UpdatedDesc => "updated_desc",
            Self::UpdatedAsc => "updated_asc",
        }
    }

    fn next_for_column(self, column: ContentListSortColumn) -> Self {
        match column {
            ContentListSortColumn::Title => match self {
                Self::TitleAsc => Self::TitleDesc,
                Self::TitleDesc => Self::TitleAsc,
                _ => Self::TitleAsc,
            },
            ContentListSortColumn::Type => match self {
                Self::TypeAsc => Self::TypeDesc,
                Self::TypeDesc => Self::TypeAsc,
                _ => Self::TypeAsc,
            },
            ContentListSortColumn::Created => match self {
                Self::CreatedDesc => Self::CreatedAsc,
                Self::CreatedAsc => Self::CreatedDesc,
                _ => Self::CreatedDesc,
            },
            ContentListSortColumn::Updated => match self {
                Self::UpdatedDesc => Self::UpdatedAsc,
                Self::UpdatedAsc => Self::UpdatedDesc,
                _ => Self::UpdatedDesc,
            },
        }
    }

    fn indicator_for(self, column: ContentListSortColumn) -> &'static str {
        match (column, self) {
            (ContentListSortColumn::Title, Self::TitleAsc)
            | (ContentListSortColumn::Type, Self::TypeAsc)
            | (ContentListSortColumn::Created, Self::CreatedAsc)
            | (ContentListSortColumn::Updated, Self::UpdatedAsc) => " (asc)",
            (ContentListSortColumn::Title, Self::TitleDesc)
            | (ContentListSortColumn::Type, Self::TypeDesc)
            | (ContentListSortColumn::Created, Self::CreatedDesc)
            | (ContentListSortColumn::Updated, Self::UpdatedDesc) => " (desc)",
            _ => "",
        }
    }
}

fn sort_content_items(
    content_items: &mut [entities::content_item::Model],
    sort_by: ContentListSortBy,
) {
    match sort_by {
        ContentListSortBy::TitleAsc => {
            content_items.sort_by(|left, right| {
                left.title
                    .to_lowercase()
                    .cmp(&right.title.to_lowercase())
                    .then_with(|| left.created_at.cmp(&right.created_at))
            });
        }
        ContentListSortBy::TitleDesc => {
            content_items.sort_by(|left, right| {
                right
                    .title
                    .to_lowercase()
                    .cmp(&left.title.to_lowercase())
                    .then_with(|| right.created_at.cmp(&left.created_at))
            });
        }
        ContentListSortBy::TypeAsc => {
            content_items.sort_by(|left, right| {
                left.page_type
                    .as_ref()
                    .cmp(right.page_type.as_ref())
                    .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            });
        }
        ContentListSortBy::TypeDesc => {
            content_items.sort_by(|left, right| {
                right
                    .page_type
                    .as_ref()
                    .cmp(left.page_type.as_ref())
                    .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            });
        }
        ContentListSortBy::CreatedDesc => {
            content_items.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        }
        ContentListSortBy::CreatedAsc => {
            content_items.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        }
        ContentListSortBy::UpdatedDesc => {
            content_items.sort_by(
                |left, right| match (left.last_updated, right.last_updated) {
                    (Some(left_updated), Some(right_updated)) => right_updated.cmp(&left_updated),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => right.created_at.cmp(&left.created_at),
                },
            );
        }
        ContentListSortBy::UpdatedAsc => {
            content_items.sort_by(
                |left, right| match (left.last_updated, right.last_updated) {
                    (Some(left_updated), Some(right_updated)) => left_updated.cmp(&right_updated),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => left.created_at.cmp(&right.created_at),
                },
            );
        }
    }
}

fn content_list_href(
    site_id: Uuid,
    page_type_filter: ContentListPageTypeFilter,
    sort_by: ContentListSortBy,
) -> String {
    let mut href = format!("/admin/site/{site_id}/content?sort_by={}", sort_by.as_str());
    if page_type_filter != ContentListPageTypeFilter::All {
        href.push_str("&page_type=");
        href.push_str(page_type_filter.as_str());
    }
    href
}

fn build_content_list_sort_headers(
    site_id: Uuid,
    page_type_filter: ContentListPageTypeFilter,
    sort_by: ContentListSortBy,
) -> Vec<AdminContentListSortHeader> {
    [
        (ContentListSortColumn::Title, "Title"),
        (ContentListSortColumn::Type, "Type"),
        (ContentListSortColumn::Created, "Created"),
        (ContentListSortColumn::Updated, "Updated"),
    ]
    .into_iter()
    .map(|(column, label)| AdminContentListSortHeader {
        label,
        href: content_list_href(site_id, page_type_filter, sort_by.next_for_column(column)),
        indicator: sort_by.indicator_for(column),
    })
    .collect()
}

fn build_search_rows(
    items: Vec<entities::content_item::Model>,
    site_title_by_id: &HashMap<Uuid, String>,
) -> Vec<AdminSearchRow> {
    items
        .into_iter()
        .map(|row| AdminSearchRow {
            site_title: site_title_by_id
                .get(&row.site_id)
                .cloned()
                .unwrap_or_else(|| "Unknown site".to_string()),
            edit_href: format!("/admin/site/{}/content/{}", row.site_id, row.id),
            title: row.title,
            created_at: row.created_at.to_rfc3339(),
        })
        .collect()
}

fn site_delete_csrf_scope(site_id: Uuid) -> String {
    format!("delete-site:{site_id}")
}

fn parse_tag_list(raw: Option<String>) -> Vec<String> {
    raw.unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn scan_filter_value(value: Option<&str>) -> &'static str {
    match value {
        Some("fixable") => "fixable",
        Some("review") => "review",
        Some("asset") => "asset",
        _ => "all",
    }
}

fn content_scan_filter_options(current: &str) -> Vec<AdminSelectOption> {
    [
        ("All issues", "all"),
        ("Fixable only", "fixable"),
        ("Review only", "review"),
        ("Asset issues", "asset"),
    ]
    .into_iter()
    .map(|(label, value)| AdminSelectOption {
        label,
        value,
        selected: current == value,
    })
    .collect()
}

struct LoadedContentScanReports {
    reports: Vec<ContentScanReport>,
    inspected_count: usize,
}

fn normalize_content_scan_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(5).clamp(1, 50)
}

async fn load_content_scan_reports(
    db: &DatabaseConnection,
    site_id: Uuid,
    content_id: Option<Uuid>,
    domains_raw: &str,
    scan_limit: usize,
) -> Result<LoadedContentScanReports, SiteError> {
    let context = ScanContext::load(db, site_id, content_id, domains_raw).await?;
    let mut content_items = list_content(db, site_id, None)
        .await
        .map_err(SiteError::internal)?;
    content_items.sort_by(|left, right| {
        left.last_updated
            .unwrap_or(left.created_at)
            .cmp(&right.last_updated.unwrap_or(right.created_at))
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
    });

    let mut reports = Vec::new();
    let mut inspected_count = 0usize;
    for content in content_items {
        inspected_count = inspected_count.saturating_add(1);
        let report = scan_content(&content, &context);
        if report.issues.is_empty() {
            continue;
        }
        reports.push(report);
        if reports.len() >= scan_limit {
            break;
        }
    }

    Ok(LoadedContentScanReports {
        reports,
        inspected_count,
    })
}

fn build_content_scan_template(
    site: &entities::site::Model,
    site_id: Uuid,
    domains: String,
    scan_limit: usize,
    current_filter: &str,
    reports: Vec<ContentScanReport>,
    summary: Option<AdminContentScanSummary>,
) -> AdminContentScanTemplate {
    let results = build_content_scan_results(site_id, current_filter, reports);
    AdminContentScanTemplate {
        template_shared: AdminTemplateData::new("Content Remediation")
            .with_site_context(site.id, &site.full_title)
            .with_links(vec![
                AdminLink::new(&format!("/admin/site/{site_id}/content"), "Back to content"),
                AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Assets"),
            ]),
        site_id,
        domains,
        scan_limit,
        filter_options: content_scan_filter_options(current_filter),
        current_filter: current_filter.to_string(),
        results,
        summary,
    }
}

fn build_content_scan_results(
    site_id: Uuid,
    current_filter: &str,
    reports: Vec<ContentScanReport>,
) -> Vec<AdminContentScanResult> {
    reports
        .into_iter()
        .filter_map(|report| {
            let issues = report
                .issues
                .into_iter()
                .filter(|issue| issue_matches_filter(issue, current_filter))
                .map(build_scan_issue_row)
                .collect::<Vec<_>>();
            if issues.is_empty() {
                return None;
            }
            let fixable_count = issues.iter().filter(|issue| issue.can_apply).count();
            let review_count = issues.len().saturating_sub(fixable_count);
            Some(AdminContentScanResult {
                title: report.content.title,
                edit_href: format!("/admin/site/{site_id}/content/{}/edit", report.content.id),
                detail_href: format!("/admin/site/{site_id}/content/{}", report.content.id),
                issue_count: issues.len(),
                fixable_count,
                review_count,
                issues,
            })
        })
        .collect()
}

fn issue_matches_filter(issue: &ScanIssue, current_filter: &str) -> bool {
    match current_filter {
        "fixable" => !matches!(issue.action, ScanAction::ReviewOnly),
        "review" => matches!(issue.action, ScanAction::ReviewOnly),
        "asset" => matches!(issue.action, ScanAction::ReplaceAsset { .. }),
        _ => true,
    }
}

fn build_scan_issue_row(issue: ScanIssue) -> AdminContentScanIssueRow {
    let (
        can_apply,
        needs_asset,
        can_import_remote,
        remote_url,
        selected_asset_id,
        selected_asset_label,
    ) = match &issue.action {
        ScanAction::ReplaceText { .. } => (
            true,
            false,
            false,
            String::new(),
            String::new(),
            String::new(),
        ),
        ScanAction::ReplaceAsset {
            suggested_asset,
            remote_url,
            ..
        } => {
            let suggested_asset_id = suggested_asset
                .as_ref()
                .map(|asset| format!("{}:{}", asset.asset_id, asset.variant))
                .unwrap_or_default();
            let suggested_asset_label = suggested_asset
                .as_ref()
                .map(|asset| format!("{} ({})", asset.asset_label, asset.variant))
                .unwrap_or_default();
            (
                true,
                suggested_asset.is_none(),
                remote_url.is_some(),
                remote_url.clone().unwrap_or_default(),
                suggested_asset_id,
                suggested_asset_label,
            )
        }
        ScanAction::ReviewOnly => (
            false,
            false,
            false,
            String::new(),
            String::new(),
            String::new(),
        ),
    };

    AdminContentScanIssueRow {
        issue_id: issue.issue_id,
        kind: issue.kind,
        label: issue.label,
        current_value: issue.current_value,
        proposed_value: issue
            .proposed_value
            .unwrap_or_else(|| "Manual selection required".to_string()),
        snippet: issue.snippet,
        can_apply,
        selected: can_apply && !needs_asset,
        needs_asset,
        can_import_remote,
        remote_url,
        selected_asset_id,
        selected_asset_label,
    }
}

fn build_scan_summary(
    scan_reports: &LoadedContentScanReports,
    applied_count: usize,
    updated_items: Vec<AdminContentScanUpdatedItem>,
    skipped_messages: Vec<String>,
) -> AdminContentScanSummary {
    AdminContentScanSummary {
        inspected_count: scan_reports.inspected_count,
        result_count: scan_reports.reports.len(),
        issue_count: scan_reports
            .reports
            .iter()
            .map(|report| report.issues.len())
            .sum(),
        applied_count,
        updated_items,
        skipped_messages,
    }
}

fn deserialize_manual_asset_map(
    raw: Option<&str>,
) -> Result<HashMap<String, ManualAssetSelection>, SiteError> {
    let raw = raw.unwrap_or("").trim();
    if raw.is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(raw).map_err(|error| {
        SiteError::BadRequest(format!("invalid asset selections payload: {error}"))
    })
}

fn deserialize_string_set(raw: Option<&str>) -> Result<HashSet<String>, SiteError> {
    let raw = raw.unwrap_or("").trim();
    if raw.is_empty() {
        return Ok(HashSet::new());
    }
    let values = serde_json::from_str::<Vec<String>>(raw)
        .map_err(|error| SiteError::BadRequest(format!("invalid string list payload: {error}")))?;
    Ok(values.into_iter().collect())
}

#[derive(Debug, Deserialize)]
struct AssetLibraryQuery {
    q: Option<String>,
    limit: Option<u64>,
    r#type: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct AssetLibraryResponse {
    assets: Vec<AssetLibraryItem>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AssetLibraryItem {
    id: Uuid,
    original_filename: String,
    mime_type: String,
    width: Option<i32>,
    height: Option<i32>,
    created_at: String,
    original_url: String,
    thumbnail_url: Option<String>,
    has_thumbnail: bool,
}

#[derive(
    EnumIter,
    DeriveActiveEnum,
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    Hash,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "lowercase"
)]
#[serde(rename_all = "lowercase")]
pub enum SiteRole {
    Viewer,
    Author,
    Editor,
    Owner,
    Admin,
}

impl SiteRole {
    pub fn is_viewer(&self) -> bool {
        *self == Self::Viewer
    }

    pub fn is_owner(&self) -> bool {
        *self == Self::Owner
    }

    pub fn is_author(&self) -> bool {
        *self == Self::Author
    }

    pub fn is_editor(&self) -> bool {
        *self == Self::Editor
    }

    /// Are they a system admin?
    pub fn is_admin(&self) -> bool {
        *self == Self::Admin
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Author => "author",
            Self::Editor => "editor",
            Self::Owner => "owner",
            Self::Admin => "admin",
        }
    }

    pub fn all_without_admin() -> Vec<Self> {
        vec![Self::Viewer, Self::Author, Self::Editor, Self::Owner]
    }
}

impl std::fmt::Display for SiteRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SiteRole::Viewer => write!(f, "Viewer"),
            SiteRole::Author => write!(f, "Author"),
            SiteRole::Editor => write!(f, "Editor"),
            SiteRole::Owner => write!(f, "Owner"),
            SiteRole::Admin => write!(f, "Admin"),
        }
    }
}

async fn current_user(session: &Session) -> Result<entities::user::Model, SiteError> {
    session
        .get::<entities::user::Model>(SESSION_USER)
        .await
        .map_err(|_| SiteError::internal("failed to read session".to_string()))?
        .ok_or_else(|| SiteError::UnAuthorized("missing user session".to_string()))
}

async fn require_global_admin(session: &Session) -> Result<entities::user::Model, SiteError> {
    let user = current_user(session).await?;
    if user.admin {
        Ok(user)
    } else {
        Err(SiteError::UnAuthorized(
            "global admin access is required".to_string(),
        ))
    }
}

fn role_satisfies(actual: SiteRole, required: SiteRole) -> bool {
    let rank = |role: SiteRole| match role {
        SiteRole::Viewer => 0_u8,
        SiteRole::Author => 1,
        SiteRole::Editor => 2,
        SiteRole::Owner => 3,
        SiteRole::Admin => 4,
    };

    rank(actual) >= rank(required)
}

fn can_view_user_profile(viewer: &entities::user::Model, target: &entities::user::Model) -> bool {
    viewer.admin || viewer.id == target.id
}

async fn require_site_role(
    state: &AdminState,
    session: &Session,
    site_id: Uuid,
    required: SiteRole,
) -> Result<(), SiteError> {
    let user = current_user(session).await?;
    if user.admin {
        return Ok(());
    }

    let membership = get_membership_for_subject(state.db.as_ref(), site_id, &user.subject)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;

    let Some(membership) = membership else {
        return Err(SiteError::UnAuthorized(format!(
            "missing membership for site {site_id}"
        )));
    };

    if role_satisfies(membership.role, required) {
        Ok(())
    } else {
        Err(SiteError::UnAuthorized(format!(
            "site role {} does not satisfy required role {}",
            membership.role.label(),
            required.label()
        )))
    }
}

pub async fn run_admin_server(
    db: Arc<DatabaseConnection>,
    listen: &str,
    oidc: &OidcConfig,
) -> Result<(), anyhow::Error> {
    let state = AdminState {
        db: db.clone(),
        oidc_client_id: ClientId::new(oidc.oidc_client_id.clone()),
        oidc_client_secret: oidc.oidc_client_secret.clone().map(ClientSecret::new),
        oidc_frontend_url: oidc.frontend_url.clone(),
        oidc_discovery_url: IssuerUrl::new(oidc.oidc_discovery_url.clone())
            .context("Failed to parse discovery URL")?,
        oidc_client: Arc::new(build_http_client().context("Failed to build OIDC HTTP client")?),
    };

    let pool = db.get_sqlite_connection_pool();
    let session_store = SqliteStore::new((*pool).clone());

    let assets_dir = resolve_admin_assets_dir();
    let upload_root = resolve_upload_root();
    session_store
        .migrate()
        .await
        .context("Failed to migrate Session store")?;

    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnSessionEnd);

    let protected_routes = Router::new()
        .route("/admin", get(get_index))
        .route("/admin/sites", get(get_sites))
        .route("/admin/search", get(get_global_search))
        .route("/admin/users/me", get(admin_user_profile_redirect))
        .route("/admin/users/{user_id}", get(admin_user_profile))
        .route(
            "/admin/sites/new",
            get(admin_sites_new).post(admin_sites_create),
        )
        .route(
            "/admin/sites/import",
            get(admin_sites_import).post(admin_sites_import_create),
        )
        .route(
            "/admin/site/{site_id}/content",
            get(admin_site_content_list),
        )
        .route(
            "/admin/site/{site_id}/memberships",
            get(admin_site_memberships),
        )
        .route(
            "/admin/site/{site_id}/memberships/new",
            axum::routing::post(admin_site_membership_create),
        )
        .route(
            "/admin/site/{site_id}/memberships/{membership_id}/update",
            axum::routing::post(admin_site_membership_update),
        )
        .route(
            "/admin/site/{site_id}/memberships/{membership_id}/remove",
            axum::routing::post(admin_site_membership_remove),
        )
        .route("/admin/site/{site_id}/search", get(get_site_search))
        .route(
            "/admin/site/{site_id}/content/scan",
            get(admin_site_content_scan).post(admin_site_content_scan_run),
        )
        .route(
            "/admin/site/{site_id}/content/scan/apply",
            axum::routing::post(admin_site_content_scan_apply),
        )
        .route(
            "/admin/site/{site_id}/content/new",
            get(admin_site_content_new).post(admin_site_content_create),
        )
        .route(
            "/admin/site/{site_id}/content/{content_id}",
            get(admin_site_content_detail),
        )
        .route(
            "/admin/site/{site_id}/content/{content_id}/edit",
            get(admin_site_content_source).post(admin_site_content_source_update),
        )
        .route(
            "/admin/site/{site_id}/content/{content_id}/preview",
            get(admin_site_content_preview),
        )
        .route(
            "/admin/site/{site_id}/preview-assets/{*asset_path}",
            get(admin_site_preview_asset),
        )
        .route(
            "/admin/site/{site_id}/content/{content_id}/advanced",
            get(admin_site_content_advanced),
        )
        .route(
            "/admin/site/{site_id}/content/{content_id}/revisions",
            get(admin_site_content_revisions),
        )
        .route(
            "/admin/site/{site_id}/content/{content_id}/revisions/{revision_id}",
            get(admin_site_revision_diff),
        )
        .route("/admin/site/{site_id}/tags", get(admin_site_tags))
        .route(
            "/admin/site/{site_id}/tags/new",
            axum::routing::post(admin_site_tag_create),
        )
        .route(
            "/admin/site/{site_id}/tags/{tag_id}/delete",
            axum::routing::post(admin_site_tag_delete),
        )
        .route("/admin/site/{site_id}/assets", get(admin_site_assets))
        .route(
            "/admin/site/{site_id}/assets/library",
            get(admin_site_assets_library),
        )
        .route(
            "/admin/site/{site_id}/assets/new",
            get(admin_site_assets_new).post(admin_site_assets_create),
        )
        .route(
            "/admin/site/{site_id}/settings",
            get(admin_site_settings).post(admin_site_settings_update),
        )
        .route(
            "/admin/site/{site_id}/delete",
            get(admin_site_delete_confirm).post(admin_site_delete),
        )
        .route("/admin/site/{site_id}/export.json", get(admin_site_export))
        .route(
            "/admin/site/{site_id}/settings/templates/{file_name}",
            get(admin_site_template_editor).post(admin_site_template_override_update),
        )
        .route(
            "/admin/site/{site_id}/settings/templates/{file_name}/reset",
            axum::routing::post(admin_site_template_override_reset),
        )
        .route("/admin/site/{site_id}/render", get(admin_site_render))
        .nest_service("/admin/assets", ServeDir::new(&assets_dir))
        .nest_service("/media/images", ServeDir::new(&upload_root))
        .layer(from_fn(crate::middleware::require_session));

    info!(
        "admin server listening on http://{listen} / {}",
        state.oidc_frontend_url
    );
    info!("admin assets dir: {}", assets_dir.display());
    info!("upload root dir: {}", upload_root.display());
    if !assets_dir.join("editor.js").exists() {
        return Err(anyhow::anyhow!(
            "admin editor assets not found; run `pnpm run build:admin` to generate them",
        ));
    }

    // disable the validator since it won't work locally etc.
    let swagger_config =
        utoipa_swagger_ui::Config::new(["/api-doc/openapi.json"]).validator_url("none");

    let app = Router::new()
        .route("/", get(admin_root))
        .route("/admin/login", get(admin_login))
        .route("/oauth2/callback", get(admin_login_callback))
        .route("/admin/logout", get(admin_logout))
        .merge(
            SwaggerUi::new("/api-docs")
                .url("/api-doc/openapi.json", ApiDoc::openapi())
                .config(swagger_config),
        )
        .merge(protected_routes)
        .fallback(not_found)
        .layer(session_layer)
        .layer(from_fn(log_requests))
        .layer(from_fn(crate::middleware::set_cache))
        .with_state(state);

    let tls_config = build_tls_config(&oidc.tls_cert_path, &oidc.tls_key_path).await?;
    let bind_addr: SocketAddr = SocketAddr::from_str(listen)
        .with_context(|| format!("failed to parse bind address {}", listen))?;
    axum_server::bind_rustls(bind_addr, tls_config)
        .serve(app.into_make_service())
        .await
        .inspect_err(|err| error!("admin server error: {err}"))
        .context("axum rustls server exited unexpectedly")
}

fn resolve_admin_assets_dir() -> PathBuf {
    if let Ok(value) = env::var("WEBSITES_ADMIN_ASSETS_DIR") {
        let path = PathBuf::from(value);
        if path.exists() {
            return path;
        }
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut candidates = vec![cwd.join("admin-ui-assets")];

    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("admin-ui-assets"));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join("admin-ui-assets"));
        }
    }

    for candidate in candidates {
        if candidate.exists() {
            return candidate;
        }
    }

    cwd.join("admin-ui-assets")
}

fn preview_asset_prefix(site_id: Uuid) -> String {
    format!("/admin/site/{site_id}/preview-assets")
}

fn rewrite_preview_asset_urls(content: &str, site_id: Uuid) -> String {
    content.replace("/assets/", &format!("{}/", preview_asset_prefix(site_id)))
}

fn should_rewrite_preview_asset_body(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/javascript"
        || mime == "application/json"
        || mime == "image/svg+xml"
        || mime.ends_with("+xml")
}

fn sanitize_preview_asset_path(asset_path: &str) -> Result<PathBuf, SiteError> {
    let mut sanitized = PathBuf::new();

    for component in StdPath::new(asset_path).components() {
        match component {
            Component::Normal(value) => sanitized.push(value),
            _ => return Err(SiteError::NotFound),
        }
    }

    if sanitized.as_os_str().is_empty() {
        return Err(SiteError::NotFound);
    }

    Ok(sanitized)
}

async fn admin_root() -> Redirect {
    Redirect::to("/admin")
}

async fn not_found(OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        NotFoundTemplate {
            requested_path: uri.path().to_string(),
        },
    )
}

async fn get_sites(Query(query): Query<DashboardQuery>) -> Redirect {
    if query.imported.is_some() {
        Redirect::to("/admin?imported=1")
    } else if query.deleted.is_some() {
        Redirect::to("/admin?deleted=1")
    } else {
        Redirect::to("/admin")
    }
}

/// The home page
async fn get_index(
    State(state): State<AdminState>,
    session: Session,
    Query(query): Query<DashboardQuery>,
) -> Result<AdminIndexTemplate, SiteError> {
    let viewer = current_user(&session).await?;
    let sites = list_sites(state.db.as_ref()).await?;
    let mut links = vec![AdminLink::new("/admin/sites/new", "New site")];
    if viewer.admin {
        links.push(AdminLink::new("/admin/sites/import", "Import site"));
    }

    let template_shared = AdminTemplateData::new("Admin Dashboard").with_links(links);
    let template_shared = if query.imported.is_some() {
        template_shared.with_toast_message("Site import complete.", "imported")
    } else if query.deleted.is_some() {
        template_shared.with_toast_message("Site deleted.", "deleted")
    } else {
        template_shared
    };

    Ok(AdminIndexTemplate {
        template_shared,
        sites,
    })
}

async fn admin_sites_new() -> Response {
    AdminSitesNewTemplate {
        template_shared: AdminTemplateData::new("Create Site")
            .with_links(vec![AdminLink::new("/admin", "Back to dashboard")]),

        templates: get_template_names().await,
    }
    .into_response()
}

async fn admin_sites_import(session: Session) -> Result<AdminSitesImportTemplate, SiteError> {
    require_global_admin(&session).await?;
    Ok(AdminSitesImportTemplate {
        template_shared: AdminTemplateData::new("Import Site")
            .with_links(vec![AdminLink::new("/admin", "Back to dashboard")]),
    })
}

async fn admin_sites_import_create(
    State(state): State<AdminState>,
    session: Session,
    mut multipart: Multipart,
) -> Result<Redirect, SiteError> {
    let actor = require_global_admin(&session).await?;
    let mut upload_bytes: Option<Vec<u8>> = None;

    loop {
        let field = multipart.next_field().await.map_err(|error| {
            SiteError::internal(format!("failed to parse site import: {error}"))
        })?;
        let Some(field) = field else { break };
        if field.name() != Some("file") {
            continue;
        }
        let bytes = field.bytes().await.map_err(|error| {
            SiteError::internal(format!("failed to read site import upload: {error}"))
        })?;
        if bytes.is_empty() {
            continue;
        }
        upload_bytes = Some(bytes.to_vec());
    }

    let upload_bytes = upload_bytes
        .ok_or_else(|| SiteError::BadRequest("provide a site export JSON file".to_string()))?;

    let txn = state.db.begin().await?;
    let result = import_site_json(&txn, &upload_bytes).await?;
    log_audit_event(
        &txn,
        &actor.subject,
        "import_site",
        "site",
        result.site_id,
        Some(result.site_id),
        Some(json!({
            "site_short_name": &result.site_short_name,
            "created_users": result.created_users,
            "reused_users": result.reused_users,
            "warnings": &result.warnings,
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log import audit: {error}")))?;
    txn.commit().await?;

    Ok(Redirect::to("/admin?imported=1"))
}

async fn ensure_site_owner_membership<C: ConnectionTrait>(
    db: &C,
    user_sub: &str,
    user_email: Option<&str>,
    site_id: Uuid,
) -> Result<Option<crate::entities::site_membership::Model>, SiteError> {
    let user = upsert_user_login(db, user_sub, user_email, None)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load user: {error}")))?;
    let existing = crate::get_membership_for_subject(db, site_id, user_sub)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;
    if existing.is_some() {
        return Ok(None);
    }

    let membership = create_membership(
        db,
        crate::NewMembership {
            site_id,
            user_id: user.id,
            role: SiteRole::Owner,
        },
    )
    .await?;

    Ok(Some(membership))
}

async fn admin_sites_create(
    State(state): State<AdminState>,
    session: Session,
    Form(form): Form<CreateSiteForm>,
) -> Result<Redirect, SiteError> {
    let template_name = form
        .template_name
        .unwrap_or_else(|| DEFAULT_TEMPLATE_NAME.to_string());
    let actor = current_user(&session).await?;
    let user_sub = actor.subject.clone();
    let short_name = form.short_name;
    let full_title = form.full_title;

    let txn = state.db.begin().await?;

    let user_sub = user_sub.clone();
    let template_name = template_name.clone();
    let site = create_site(&txn, short_name, full_title, template_name)
        .await
        .map_err(|error| SiteError::internal(format!("failed to create site: {error}")))?;
    log_audit_event(
        &txn,
        &actor.subject,
        "create_site",
        "site",
        &site.id,
        Some(site.id),
        Some(json!({ "short_name": &site.short_name, "full_title": &site.full_title })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log site audit: {error}")))?;

    if let Some(membership) =
        ensure_site_owner_membership(&txn, &user_sub, actor.email.as_deref(), site.id).await?
    {
        log_audit_event(
            &txn,
            &actor.subject,
            "create_membership",
            "site_membership",
            &membership.id,
            Some(membership.site_id),
            Some(json!({
                "site_id": membership.site_id,
                "user_id": membership.user_id,
                "role": membership.role.label()
            })),
        )
        .await
        .map_err(|error| SiteError::internal(format!("failed to log membership audit: {error}")))?;
    }
    txn.commit().await?;
    Ok(Redirect::to("/admin"))
}

async fn admin_login(
    State(state): State<AdminState>,
    session: Session,
) -> Result<Response, SiteError> {
    let client = match build_oidc_client(&state).await {
        Ok(client) => client,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to initialize OIDC client: {error}"
            )));
        }
    };

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_state, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    if session
        .insert(SESSION_OIDC_STATE_KEY, csrf_state.secret().to_string())
        .await
        .is_err()
        || session
            .insert(SESSION_OIDC_PKCE_KEY, pkce_verifier.secret().to_string())
            .await
            .is_err()
        || session
            .insert(SESSION_OIDC_NONCE_KEY, nonce.secret().to_string())
            .await
            .is_err()
    {
        return Err(SiteError::internal(
            "failed to persist OIDC session data".to_string(),
        ));
    }

    let auth_url = auth_url.to_string();
    Ok(Redirect::to(&auth_url).into_response())
}

async fn admin_logout(session: Session) -> Redirect {
    let _ = session.clear().await;
    Redirect::to("/admin/login")
}

async fn admin_site_content_list(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AdminContentListQuery>,
) -> Result<AdminContentListTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;

    let page_type_filter = ContentListPageTypeFilter::from_query(query.page_type.as_deref());
    let sort_by = ContentListSortBy::from_query(query.sort_by.as_deref());

    match list_content(state.db.as_ref(), site_id, page_type_filter.page_type()).await {
        Ok(mut pages) => {
            sort_content_items(&mut pages, sort_by);

            Ok(AdminContentListTemplate {
                template_shared: AdminTemplateData::new("Content")
                    .with_site_context(site.id, &site.full_title)
                    .with_links(vec![
                        AdminLink::new(
                            &format!("/admin/site/{site_id}/content/new"),
                            "New content",
                        ),
                        AdminLink::new(
                            &format!("/admin/site/{site_id}/content/scan"),
                            "Scan content",
                        ),
                        AdminLink::new(&format!("/admin/site/{site_id}/search"), "Search content"),
                        AdminLink::new(
                            &format!("/admin/site/{site_id}/memberships"),
                            "Memberships",
                        ),
                        AdminLink::new(&format!("/admin/site/{site_id}/tags"), "Tags"),
                        AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Assets"),
                        AdminLink::new(&format!("/admin/site/{site_id}/render"), "Render"),
                        AdminLink::new(&format!("/admin/site/{site_id}/settings"), "Site settings"),
                    ]),

                site_id,
                page_type_options: page_type_filter.options(),
                current_sort_by: sort_by.as_str(),
                sort_headers: build_content_list_sort_headers(site_id, page_type_filter, sort_by),
                content_rows: pages
                    .into_iter()
                    .map(|item| AdminContentListRow {
                        edit_href: format!("/admin/site/{}/content/{}/edit", site_id, item.id),
                        title: item.title,
                        page_type: item.page_type.to_string(),
                        created_at: item.created_at.to_rfc3339(),
                        updated_at: item
                            .last_updated
                            .map(|value| value.to_rfc3339())
                            .unwrap_or_else(|| "-".to_string()),
                    })
                    .collect(),
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load content for site {site_id}: {error}"
        ))),
    }
}

async fn admin_site_memberships(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminMembershipsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let viewer = current_user(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let memberships = list_memberships(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load memberships: {error}")))?;
    let user_ids = memberships
        .iter()
        .map(|membership| membership.user_id)
        .collect::<Vec<_>>();
    let users = list_users_by_ids(state.db.as_ref(), user_ids)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load users: {error}")))?;
    let user_map = users
        .into_iter()
        .map(|user| (user.id, (user.subject, user.email)))
        .collect::<HashMap<_, _>>();
    let membership_user_ids = user_map
        .keys()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let membership_rows = memberships
        .into_iter()
        .map(|membership| {
            let (subject, email) = user_map
                .get(&membership.user_id)
                .cloned()
                .unwrap_or_else(|| ("unknown".to_string(), None));
            AdminMembershipRow {
                subject,
                email,
                role: membership.role,
                profile_href: if viewer.admin || viewer.id == membership.user_id {
                    Some(format!("/admin/users/{}", membership.user_id))
                } else {
                    None
                },
                update_href: format!("/admin/site/{site_id}/memberships/{}/update", membership.id),
                remove_href: format!("/admin/site/{site_id}/memberships/{}/remove", membership.id),
            }
        })
        .collect();
    let membership_candidates = list_users(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load candidate users: {error}")))?
        .into_iter()
        .filter(|user| !membership_user_ids.contains(&user.id))
        .map(|user| {
            let search_value = match &user.email {
                Some(email) => format!("{email} ({})", user.subject),
                None => user.subject.clone(),
            };
            AdminMembershipCandidateRow {
                user_id: user.id,
                subject: user.subject,
                email: user.email,
                search_value,
            }
        })
        .collect();

    Ok(AdminMembershipsTemplate {
        template_shared: AdminTemplateData::new("Memberships")
            .with_site_context(site.id, &site.full_title)
            .with_links(vec![
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
                AdminLink::new(&format!("/admin/site/{site_id}/settings"), "Site settings"),
            ]),
        site_id: site.id,
        site_full_title: site.full_title,
        memberships: membership_rows,
        membership_candidates,
        roles: SiteRole::all_without_admin(),
    })
}

async fn admin_user_profile_redirect(session: Session) -> Result<Redirect, SiteError> {
    let user = current_user(&session).await?;
    Ok(Redirect::to(&format!("/admin/users/{}", user.id)))
}

async fn admin_user_profile(
    State(state): State<AdminState>,
    session: Session,
    Path(user_id): Path<Uuid>,
) -> Result<AdminUserProfileTemplate, SiteError> {
    let viewer = current_user(&session).await?;
    let target = get_user_by_id(state.db.as_ref(), user_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load user {user_id}: {error}")))?
        .ok_or(SiteError::NotFound)?;

    if !can_view_user_profile(&viewer, &target) {
        return Err(SiteError::UnAuthorized(
            "cannot view another user's profile".to_string(),
        ));
    }

    let memberships = list_memberships_for_user_id(state.db.as_ref(), target.id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load memberships: {error}")))?;
    let role_by_site = memberships
        .into_iter()
        .map(|membership| (membership.site_id, membership.role))
        .collect::<HashMap<_, _>>();
    let sites = list_sites_for_subject(state.db.as_ref(), &target.subject)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load sites: {error}")))?;
    let membership_rows = sites
        .into_iter()
        .filter_map(|site| {
            role_by_site
                .get(&site.id)
                .copied()
                .map(|role| AdminUserMembershipRow {
                    site_title: site.full_title,
                    site_short_name: site.short_name,
                    role,
                    site_href: format!("/admin/site/{}/content", site.id),
                })
        })
        .collect::<Vec<_>>();

    Ok(AdminUserProfileTemplate {
        template_shared: AdminTemplateData::new(format!(
            "User Profile: {}",
            target
                .display_name
                .as_deref()
                .or(target.email.as_deref())
                .unwrap_or(&target.subject)
        )),

        user_id: target.id,
        display_name: target.display_name.unwrap_or_else(|| "n/a".to_string()),
        subject: target.subject,
        email: target.email.unwrap_or_else(|| "n/a".to_string()),
        created_at: target.created_at.to_rfc3339(),
        last_login_at: target
            .last_login_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "n/a".to_string()),
        is_admin: target.admin,
        memberships: membership_rows,
    })
}

async fn admin_site_membership_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<MembershipCreateForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let subject = form.subject.trim().to_string();
    let user = if let Some(user_id) = form.user_id {
        let user = get_user_by_id(state.db.as_ref(), user_id)
            .await
            .map_err(|error| SiteError::internal(format!("failed to load user: {error}")))?;
        user.ok_or_else(|| SiteError::BadRequest("unknown user".to_string()))?
    } else {
        let subject = subject.trim();
        if subject.is_empty() {
            return Err(SiteError::internal("missing subject".to_string()));
        }
        entities::user::Entity::find()
            .filter(
                Condition::any()
                    .add(entities::user::Column::Subject.eq(subject))
                    .add(entities::user::Column::Email.eq(subject)),
            )
            .one(state.db.as_ref())
            .await
            .map_err(|error| SiteError::internal(format!("failed to load user: {error}")))?
            .ok_or_else(|| {
                SiteError::BadRequest(
                    "user must log in before site access can be granted".to_string(),
                )
            })?
    };
    let actor = current_user(&session).await?.subject;
    let txn = state.db.begin().await?;
    let actor = actor.clone();
    let membership = create_membership(
        &txn,
        crate::NewMembership {
            site_id,
            user_id: user.id,
            role: form.role,
        },
    )
    .await?;
    log_audit_event(
        &txn,
        &actor,
        "create_membership",
        "site_membership",
        &membership.id,
        Some(membership.site_id),
        Some(json!({
            "user_id": membership.user_id,
            "role": membership.role
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log membership audit: {error}")))?;
    txn.commit().await.map_err(|error| {
        SiteError::internal(format!("failed to commit membership creation: {error}"))
    })?;
    Ok(Redirect::to(&format!("/admin/site/{site_id}/memberships")))
}

async fn admin_site_membership_update(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, membership_id)): Path<(Uuid, Uuid)>,
    Form(form): Form<MembershipUpdateForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;

    let membership = get_membership_by_id(state.db.as_ref(), membership_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;
    let membership =
        membership.ok_or_else(|| SiteError::internal("membership not found".to_string()))?;
    if membership.site_id != site_id {
        return Err(SiteError::UnAuthorized(
            "membership does not belong to site".to_string(),
        ));
    }

    let actor = current_user(&session).await?.subject;
    let txn = state.db.begin().await?;
    let actor = actor.clone();

    let updated = update_membership_role(&txn, membership.id, form.role)
        .await
        .map_err(|error| SiteError::internal(format!("failed to update membership: {error}")))?;
    log_audit_event(
        &txn,
        &actor,
        "update_membership",
        "site_membership",
        &updated.id,
        Some(updated.site_id),
        Some(json!({
            "site_id": updated.site_id,
            "user_id": updated.user_id,
            "role": updated.role
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log membership audit: {error}")))?;

    txn.commit().await.map_err(|error| {
        SiteError::internal(format!("failed to commit membership update: {error}"))
    })?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/memberships")))
}

async fn admin_site_membership_remove(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, membership_id)): Path<(Uuid, Uuid)>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let membership = get_membership_by_id(state.db.as_ref(), membership_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;
    let membership = membership.ok_or(SiteError::internal("membership not found".to_string()))?;
    if membership.site_id != site_id {
        return Err(SiteError::UnAuthorized(
            "membership does not belong to site".to_string(),
        ));
    }
    let actor = current_user(&session).await?.subject;
    let txn = state.db.begin().await?;
    delete_membership(&txn, membership.id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to remove membership: {error}")))?;
    log_audit_event(
        &txn,
        &actor,
        "delete_membership",
        "site_membership",
        &membership.id,
        Some(membership.site_id),
        Some(json!({
            "site_id": membership.site_id,
            "user_id": membership.user_id,
            "role": membership.role
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log membership audit: {error}")))?;
    txn.commit().await.map_err(|error| {
        SiteError::internal(format!("failed to commit membership removal: {error}"))
    })?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/memberships")))
}

async fn get_global_search(
    State(state): State<AdminState>,
    session: Session,
    Query(query): Query<SearchQuery>,
) -> Result<AdminSearchTemplate, SiteError> {
    let query_text = query.q.unwrap_or_default();
    let query_text = query_text.trim().to_string();
    let mut rows = Vec::new();
    let mut message = "Search content across all sites.".to_string();

    if !query_text.is_empty() {
        let items = search_all_content(state.db.as_ref(), &query_text).await?;
        let site_title_by_id = list_sites(state.db.as_ref())
            .await?
            .into_iter()
            .map(|site| (site.id, site.full_title))
            .collect::<HashMap<_, _>>();
        message = format!("Found {} result(s) for \"{}\".", items.len(), query_text);
        rows = build_search_rows(items, &site_title_by_id);
    }

    current_user(&session).await?;

    Ok(AdminSearchTemplate {
        template_shared: AdminTemplateData::new("Search Content")
            .with_message(message)
            .with_nav_search_value(&query_text)
            .with_links(vec![AdminLink::new("/admin", "Back to dashboard")]),
        rows,
        show_site_column: true,
    })
}

async fn get_site_search(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<SearchQuery>,
) -> Result<AdminSearchTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = entities::site::Entity::find_by_id(site_id)
        .one(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?
        .ok_or_else(|| SiteError::NotFound)?;
    let query_text = query.q.unwrap_or_default();
    let query_text = query_text.trim().to_string();
    let mut rows = Vec::new();
    let mut message: String = "Search content by title, slug, or body text.".to_string();

    if !query_text.is_empty() {
        let items = search_content(state.db.as_ref(), site_id, &query_text).await?;
        let site_title_by_id = HashMap::from_iter([(site.id, site.full_title.clone())]);
        message = format!("Found {} result(s) for \"{}\".", items.len(), query_text);
        rows = build_search_rows(items, &site_title_by_id);
    }

    Ok(AdminSearchTemplate {
        template_shared: AdminTemplateData::new("Search Content")
            .with_site_context(site.id, &site.full_title)
            .with_message(message)
            .with_nav_search_value(&query_text)
            .with_links(vec![AdminLink::new(
                &format!("/admin/site/{site_id}/content"),
                "Back to site dashboard",
            )]),
        rows,
        show_site_column: false,
    })
}

async fn admin_site_content_scan(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminContentScanTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    Ok(build_content_scan_template(
        &site,
        site_id,
        String::new(),
        5,
        "all",
        Vec::new(),
        None,
    ))
}

async fn admin_site_content_scan_run(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<ContentScanForm>,
) -> Result<AdminContentScanTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let scan_limit = normalize_content_scan_limit(form.scan_limit);
    let scan_reports = load_content_scan_reports(
        state.db.as_ref(),
        site_id,
        form.content_id,
        &form.domains,
        scan_limit,
    )
    .await?;
    let summary = Some(build_scan_summary(&scan_reports, 0, Vec::new(), Vec::new()));
    Ok(build_content_scan_template(
        &site,
        site_id,
        form.domains,
        scan_limit,
        scan_filter_value(form.filter.as_deref()),
        scan_reports.reports,
        summary,
    ))
}

async fn admin_site_content_scan_apply(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<ContentScanApplyForm>,
) -> Result<AdminContentScanTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let scan_limit = normalize_content_scan_limit(form.scan_limit);
    let selected_issue_ids = deserialize_string_set(form.selected_issue_ids_json.as_deref())?;
    let remote_import_issue_ids =
        deserialize_string_set(form.remote_import_issue_ids_json.as_deref())?;
    let manual_asset_map = deserialize_manual_asset_map(form.asset_selections_json.as_deref())?;
    let scan_reports =
        load_content_scan_reports(state.db.as_ref(), site_id, None, &form.domains, scan_limit)
            .await?;

    let mut updated_items = Vec::new();
    let mut skipped_messages = Vec::new();
    let mut applied_count = 0usize;
    let mut imported_assets: HashMap<String, AssetReference> = HashMap::new();
    let txn = state.db.begin().await?;

    for report in &scan_reports.reports {
        let selected_for_content = report
            .issues
            .iter()
            .filter(|issue| selected_issue_ids.contains(&issue.issue_id))
            .count();
        if selected_for_content == 0 {
            continue;
        }

        let mut asset_replacements = HashMap::new();
        let mut missing_remote_issues = Vec::new();
        for issue in &report.issues {
            if !selected_issue_ids.contains(&issue.issue_id) {
                continue;
            }
            if let Some(selection) = manual_asset_map.get(&issue.issue_id) {
                asset_replacements.insert(
                    issue.issue_id.clone(),
                    AssetReference {
                        asset_id: selection.asset_id,
                        variant: selection.variant.clone(),
                        asset_label: selection.asset_label.clone(),
                    },
                );
                continue;
            }
            if remote_import_issue_ids.contains(&issue.issue_id)
                && let ScanAction::ReplaceAsset {
                    remote_url: Some(remote_url),
                    alt,
                    title,
                    ..
                } = &issue.action
            {
                let asset_reference = if let Some(existing) = imported_assets.get(remote_url) {
                    existing.clone()
                } else {
                    let imported = import_remote_scan_asset(
                        &txn,
                        state.oidc_client.as_ref(),
                        site_id,
                        &actor.subject,
                        remote_url,
                    )
                    .await?;
                    imported_assets.insert(remote_url.clone(), imported.clone());
                    imported
                };
                let shortcode = format_asset_shortcode(
                    asset_reference.asset_id,
                    &asset_reference.variant,
                    alt,
                    title.as_deref(),
                );
                asset_replacements.insert(
                    issue.issue_id.clone(),
                    AssetReference {
                        asset_id: asset_reference.asset_id,
                        variant: asset_reference.variant.clone(),
                        asset_label: shortcode,
                    },
                );
            } else if let ScanAction::ReplaceAsset {
                suggested_asset: None,
                remote_url: Some(_),
                ..
            } = &issue.action
            {
                missing_remote_issues.push(issue.label.clone());
            }
        }

        if !missing_remote_issues.is_empty() {
            skipped_messages.push(format!(
                "{} was skipped because some image issues still need an asset selection or remote import.",
                report.content.title
            ));
            continue;
        }

        let applied_issues = crate::content_scan::apply_issue_replacements(
            &report.content.page_content,
            &report.issues,
            &selected_issue_ids,
            &asset_replacements,
            &remote_import_issue_ids,
        );
        if applied_issues.is_empty() {
            skipped_messages.push(format!(
                "{} had no applicable fixes after rescanning.",
                report.content.title
            ));
            continue;
        }

        let mut page_content = report.content.page_content.clone();
        for applied in &applied_issues {
            if applied.kind == "__remote_import__" {
                skipped_messages.push(format!(
                    "{} still has an unresolved remote image replacement.",
                    report.content.title
                ));
                continue;
            }
            if applied.end <= page_content.len() && applied.start <= applied.end {
                page_content.replace_range(applied.start..applied.end, &applied.replacement);
            }
        }

        let content = update_content(
            &txn,
            crate::UpdateContent {
                content_id: report.content.id,
                page_type: None,
                title: Some(report.content.title.clone()),
                slug: Some(report.content.slug.clone()),
                page_content: Some(page_content),
                draft: Some(report.content.draft),
                published_at: report.content.published_at,
                editor_sub: actor.subject.clone(),
            },
        )
        .await
        .map_err(SiteError::internal)?;
        applied_count = applied_count.saturating_add(applied_issues.len());
        updated_items.push(AdminContentScanUpdatedItem {
            title: content.title,
            applied_count: applied_issues.len(),
        });
    }

    log_audit_event(
        &txn,
        &actor.subject,
        "content_scan_apply",
        "content_item",
        &site_id.to_string(),
        Some(site_id),
        Some(json!({
            "applied_count": applied_count,
            "updated_titles": updated_items.iter().map(|item| item.title.clone()).collect::<Vec<_>>(),
            "selected_issue_count": selected_issue_ids.len(),
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log scan apply audit: {error}")))?;
    txn.commit().await?;

    let refreshed_reports =
        load_content_scan_reports(state.db.as_ref(), site_id, None, &form.domains, scan_limit)
            .await?;
    let summary = Some(build_scan_summary(
        &refreshed_reports,
        applied_count,
        updated_items,
        skipped_messages,
    ));
    Ok(build_content_scan_template(
        &site,
        site_id,
        form.domains,
        scan_limit,
        scan_filter_value(form.filter.as_deref()),
        refreshed_reports.reports,
        summary,
    ))
}

async fn admin_site_content_new(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminContentNewTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let tags = list_tags(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load tags: {error}")))?;
    let tags = tags
        .into_iter()
        .map(|tag| AdminTagOption {
            name: tag.name,
            selected: false,
        })
        .collect();

    let site = get_by_id(state.db.as_ref(), site_id).await?;
    Ok(AdminContentNewTemplate {
        template_shared: AdminTemplateData::new("Create Content")
            .with_site_context(site.id, &site.full_title),
        page_content: String::new(), // empty page content for the editor
        tags,
        site_id: site.id,
        allow_external_image: false,
    })
}

async fn admin_site_content_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<CreateContentForm>,
) -> Result<Redirect, SiteError> {
    let page_type = PageType::from_str(&form.page_type)
        .map_err(|error| SiteError::internal(error.to_string()))?;
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let tag_names = parse_tag_list(form.tag_list);
    let title = form.title;
    let slug = form.slug;
    let page_content = form.page_content;
    let draft = form.draft.unwrap_or(false);

    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let site_id = site.id;

    let tag_names = tag_names.clone();
    let txn = state.db.begin().await?;
    let content = create_content(
        &txn,
        NewContent {
            site_id,
            page_type,
            title,
            slug,
            page_content,
            draft,
            creator_sub: actor.subject.clone(),
            published_at: None,
        },
    )
    .await
    .map_err(|error| {
        SiteError::internal(format!(
            "failed to create content for site {site_id}: {error}"
        ))
    })?;

    if !tag_names.is_empty() {
        let revision = get_revision_by_number(&txn, content.id, 1)
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to load revision for tags: {error}"))
            })?
            .ok_or_else(|| SiteError::internal("missing revision for new content".to_string()))?;
        crate::assign_tags_to_content(&txn, content.site_id, content.id, revision.id, tag_names)
            .await
            .map_err(|error| SiteError::internal(format!("failed to assign tags: {error}")))?;
    }

    log_audit_event(
        &txn,
        &actor.subject,
        "create_content",
        "content_item",
        &content.id,
        Some(content.site_id),
        Some(json!({
            "page_type": content.page_type,
            "slug": &content.slug,
            "title": &content.title,
            "draft": content.draft
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log content audit: {error}")))?;

    txn.commit().await.map_err(|error| {
        SiteError::internal(format!("failed to commit content creation: {error}"))
    })?;

    Ok(Redirect::to(&format!(
        "/admin/site/{}/content/{}",
        content.site_id, content.id
    )))
}

async fn admin_site_content_detail(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<AdminContentDetailTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;

    let content = entities::content_item::Entity::find_by_id(content_id)
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .one(state.db.as_ref())
        .await
        .map_err(|err| SiteError::internal(format!("failed to load content {content_id}: {err}")))?
        .ok_or(SiteError::NotFound)?;
    let tags = list_content_tags(state.db.as_ref(), content.id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load tags for content {content_id}: {error}"
            ))
        })?
        .into_iter()
        .map(|tag| tag.name)
        .collect::<Vec<_>>();
    let aliases = list_aliases(state.db.as_ref(), content.site_id, Some(content.id))
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load aliases for content {content_id}: {error}"
            ))
        })?
        .into_iter()
        .map(|alias| AdminContentAliasRow {
            kind: alias.kind,
            path: alias.alias_path,
        })
        .collect::<Vec<_>>();
    let revisions = list_revisions(state.db.as_ref(), content.id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load revisions for content {content_id}: {error}"
            ))
        })?;
    let site = get_by_id(state.db.as_ref(), content.site_id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load site {} for content {content_id}: {error}",
                content.site_id
            ))
        })?;

    let route = content_primary_route(&content);
    Ok(AdminContentDetailTemplate {
        template_shared: AdminTemplateData::new(format!("Content: /{route}"))
            .with_site_context(site.id, &site.full_title)
            .with_links(vec![
                AdminLink::new(
                    &format!(
                        "/admin/site/{}/content/{}/edit",
                        content.site_id, content.id
                    ),
                    "Open in editor",
                ),
                AdminLink::new(
                    &format!(
                        "/admin/site/{}/content/{}/revisions",
                        content.site_id, content.id
                    ),
                    "Revisions",
                ),
                AdminLink::new(
                    &format!("/admin/site/{}/content", content.site_id),
                    "Back to site dashboard",
                ),
            ]),
        title: content.title,
        page_type: content.page_type,
        status: content_status_label(content.draft),
        primary_route: display_route_path(&route),
        revisions_summary: latest_revision_summary(&revisions),
        tags,
        aliases,
        // TODO look up the creator by sub and display their name instead of sub
        creator_sub: content.creator_sub,
        content_id: content.id,
        site_id: content.site_id,
        slug: content.slug,
        created_at: content.created_at.to_rfc3339(),
        updated_at: format_optional_datetime(content.last_updated),
        published_at: format_optional_datetime(content.published_at),
        page_content: content.page_content,
    })
}

#[axum::debug_handler]
async fn admin_site_content_source(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<SourceEditorQuery>,
) -> Result<AdminContentSourceTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;

    let content = entities::content_item::Entity::find_by_id(content_id)
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .one(state.db.as_ref())
        .await
        .map_err(|err| SiteError::internal(format!("failed to load content {content_id}: {err}")))?
        .ok_or(SiteError::NotFound)?;
    let preview_href = format!(
        "/admin/site/{}/content/{}/preview",
        content.site_id, content.id
    );
    let back_href = format!("/admin/site/{}/content/{}", content.site_id, content.id);
    let site_tags = list_tags(state.db.as_ref(), content.site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site tags: {error}")))?;
    let selected_tags = list_content_tags(state.db.as_ref(), content.id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load content tags: {error}")))?;
    let selected_tag_names = selected_tags
        .into_iter()
        .map(|tag| tag.name)
        .collect::<std::collections::HashSet<_>>();
    let tags = site_tags
        .into_iter()
        .map(|tag| AdminTagOption {
            selected: selected_tag_names.contains(&tag.name),
            name: tag.name,
        })
        .collect();

    let draft = content.draft;
    let published_at = content.content_publish_timestamp();
    let title = content.title;
    let slug = content.slug;
    let page_content = content.page_content;
    let site = get_by_id(state.db.as_ref(), content.site_id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load site {} for content {content_id}: {error}",
                content.site_id
            ))
        })?;

    let template_shared = AdminTemplateData::new(format!("Editing: {}", title))
        .with_links(vec![
            AdminLink::new(&preview_href, "Preview").with_target_blank(),
            AdminLink::new(&back_href, "Back to site dashboard"),
        ])
        .with_site_context(site.id, &site.full_title);
    let template_shared = if query.saved.is_some() {
        template_shared.with_toast_message("Content saved.", "saved")
    } else {
        template_shared
    };

    Ok(AdminContentSourceTemplate {
        template_shared,
        tags,
        title,
        slug,
        page_type: content.page_type.to_string(),
        draft,
        published_at,
        page_content,
        site_id: content.site_id,
        allow_external_image: true,
    })
}

async fn admin_site_content_source_update(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
    Form(form): Form<UpdateContentForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let draft = matches!(form.draft.as_str(), "true" | "1" | "yes");
    let published_at =
        parse_optional_datetime(normalize_optional(form.published_at), "published_at")?;
    let page_type = PageType::from_str(&form.page_type)
        .map_err(|error| SiteError::internal(error.to_string()))?;
    let title = form.title;
    let slug = form.slug;
    let page_content = form.page_content;
    let tag_names = parse_tag_list(form.tag_list);

    let txn = state.db.begin().await?;

    let content = update_content(
        &txn,
        crate::UpdateContent {
            content_id,
            page_type: Some(page_type),
            title: Some(title),
            slug: Some(slug),
            page_content: Some(page_content),
            draft: Some(draft),
            published_at,
            editor_sub: actor.subject.clone(),
        },
    )
    .await
    .map_err(|error| {
        SiteError::internal(format!("failed to update content {content_id}: {error}"))
    })?;
    let revision = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(content.id))
        .order_by_desc(entities::content_revision::Column::RevisionNumber)
        .one(&txn)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load latest revision: {error}")))?
        .ok_or_else(|| SiteError::internal("missing revision for updated content".to_string()))?;
    sync_tags_to_content(&txn, content.site_id, content.id, revision.id, tag_names)
        .await
        .map_err(|error| SiteError::internal(format!("failed to sync tags: {error}")))?;

    log_audit_event(
        &txn,
        &actor.subject,
        "update_content",
        "content_item",
        &content.id.to_string(),
        Some(content.site_id),
        Some(json!({
                "page_type": content.page_type.to_string(),
                "slug": &content.slug,
                "title": &content.title,
                "draft": content.draft
            }
        )),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log update audit: {error}")))?;
    txn.commit().await?;
    Ok(Redirect::to(&format!(
        "/admin/site/{}/content/{}/edit?saved=1",
        content.site_id, content.id
    )))
}

async fn admin_site_content_preview(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<Html<String>, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let rendered = render_content_preview(
        state.db.as_ref(),
        site_id,
        content_id,
        crate::SITE_TEMPLATES_DIR,
    )
    .await?;
    Ok(Html(rewrite_preview_asset_urls(&rendered, site_id)))
}

async fn admin_site_preview_asset(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, asset_path)): Path<(Uuid, String)>,
) -> Result<Response, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let safe_asset_path = sanitize_preview_asset_path(&asset_path)?;
    let file_path = StdPath::new(crate::SITE_TEMPLATES_DIR)
        .join(site.template_name)
        .join("assets")
        .join(safe_asset_path);
    let metadata = fs::metadata(&file_path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SiteError::NotFound
        } else {
            SiteError::internal(format!(
                "failed to inspect preview asset {}: {error}",
                file_path.display()
            ))
        }
    })?;
    if !metadata.is_file() {
        return Err(SiteError::NotFound);
    }

    let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
    let mut body = fs::read(&file_path).await.map_err(|error| {
        SiteError::internal(format!(
            "failed to read preview asset {}: {error}",
            file_path.display()
        ))
    })?;

    if should_rewrite_preview_asset_body(mime.essence_str())
        && let Ok(text) = String::from_utf8(body.clone())
    {
        body = rewrite_preview_asset_urls(&text, site_id).into_bytes();
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(Body::from(body))
        .map_err(|error| {
            SiteError::internal(format!("failed to build preview asset response: {error}"))
        })
}

async fn admin_site_content_advanced(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/content/{content_id}"
    )))
}

async fn admin_site_content_revisions(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<AdminContentRevisionsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;

    let revisions = list_revisions(state.db.as_ref(), content_id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load revisions for {content_id}: {error}"
            ))
        })?;

    let diff_links = revisions
        .iter()
        .filter(|revision| revision.revision_number > 1)
        .map(|revision| {
            format!(
                "<li><a href=\"/admin/site/{}/content/{}/revisions/{}\">Diff revision {}</a></li>",
                site_id, content_id, revision.id, revision.revision_number
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let rows = revisions
        .into_iter()
        .map(|revision| AdminRow {
            label: format!("rev-{}", revision.revision_number),
            value: format!(
                "{} updated {} by {} [{}]",
                revision.id, revision.created_at, revision.editor_sub, revision.page_type
            ),
        })
        .collect();

    Ok(AdminContentRevisionsTemplate {
        template_shared: AdminTemplateData::new(format!("Revisions for {content_id}"))
            .with_site_context(site.id, &site.full_title)
            .with_links(vec![AdminLink::new(
                &format!("/admin/site/{site_id}/content/{content_id}"),
                "Back to site dashboard",
            )]),
        rows,
        inline_body: if diff_links.is_empty() {
            "<p>No diffs available for the first revision.</p>".to_string()
        } else {
            format!(
                "<section class=\"revision-diffs\"><h2>Revision Diffs</h2><ul>{}</ul></section>",
                diff_links
            )
        },
    })
}

async fn admin_site_revision_diff(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id, revision_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<AdminRevisionDiffTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let revision = get_revision(state.db.as_ref(), revision_id)
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to load revision {revision_id}: {error}"))
        })?;
    if revision.content_id != content_id || revision.site_id != site_id {
        return Err(SiteError::internal(
            "revision does not belong to requested content".to_string(),
        ));
    }

    let previous = if revision.revision_number > 1 {
        match get_revision_by_number(
            state.db.as_ref(),
            revision.content_id,
            revision.revision_number - 1,
        )
        .await
        {
            Ok(previous) => previous,
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to load previous revision: {error}"
                )));
            }
        }
    } else {
        None
    };

    let diff_text = if let Some(previous) = previous {
        let previous_label = format!("rev-{}", previous.revision_number);
        let current_label = format!("rev-{}", revision.revision_number);
        TextDiff::from_lines(&previous.page_content, &revision.page_content)
            .unified_diff()
            .header(&previous_label, &current_label)
            .to_string()
    } else {
        "No previous revision available.".to_string()
    };

    Ok(AdminRevisionDiffTemplate {
        template_shared: AdminTemplateData::new(format!(
            "Diff for rev-{}",
            revision.revision_number
        ))
        .with_site_context(site.id, &site.full_title)
        .with_message(format!(
            "Comparing revision {} for content {}.",
            revision.revision_number, revision.content_id
        ))
        .with_links(vec![
            AdminLink::new(
                &format!("/admin/site/{}/content/{}/revisions", site_id, content_id),
                "Back to revisions",
            ),
            AdminLink::new(
                &format!("/admin/site/{}/content/{}", site_id, content_id),
                "Back to site dashboard",
            ),
        ]),

        rows: vec![
            AdminRow {
                label: "revision_id".to_string(),
                value: revision.id.to_string(),
            },
            AdminRow {
                label: "created_at".to_string(),
                value: revision.created_at.to_rfc3339(),
            },
            AdminRow {
                label: "editor_sub".to_string(),
                value: revision.editor_sub,
            },
        ],

        pre_body: diff_text,
    })
}

async fn admin_site_tags(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminTagsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    match list_tags(state.db.as_ref(), site_id).await {
        Ok(tags) => {
            let tags = tags
                .into_iter()
                .map(|tag| AdminSiteTagRow {
                    id: tag.id,
                    name: tag.name,
                    delete_href: format!("/admin/site/{site_id}/tags/{}/delete", tag.id),
                })
                .collect();

            Ok(AdminTagsTemplate {
                template_shared: AdminTemplateData::new("Tags")
                    .with_site_context(site.id, &site.full_title)
                    .with_links(vec![AdminLink::new(
                        &format!("/admin/site/{site_id}/content"),
                        "Back to site dashboard",
                    )]),

                site_id,
                tags,
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load tags for site {site_id}: {error}"
        ))),
    }
}

async fn admin_site_tag_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<CreateTagForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Editor).await?;
    let actor = current_user(&session).await?;
    let name = form.name.trim();
    if name.is_empty() {
        return Err(SiteError::BadRequest("missing tag name".to_string()));
    }

    let txn = state.db.begin().await?;
    let tag = create_tag(
        &txn,
        crate::NewTag {
            site_id,
            name: name.to_string(),
        },
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to create tag: {error}")))?;
    log_audit_event(
        &txn,
        &actor.subject,
        "create_tag",
        "tag",
        &tag.id,
        Some(site_id),
        Some(json!({
            "name": tag.name,
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log tag audit: {error}")))?;
    txn.commit()
        .await
        .map_err(|error| SiteError::internal(format!("failed to commit tag creation: {error}")))?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/tags")))
}

async fn admin_site_tag_delete(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Editor).await?;
    let actor = current_user(&session).await?;

    let txn = state.db.begin().await?;
    delete_tag(&txn, site_id, tag_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to delete tag: {error}")))?;
    log_audit_event(
        &txn,
        &actor.subject,
        "delete_tag",
        "tag",
        &tag_id,
        Some(site_id),
        None,
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log tag audit: {error}")))?;
    txn.commit()
        .await
        .map_err(|error| SiteError::internal(format!("failed to commit tag delete: {error}")))?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/tags")))
}

fn normalize_asset_mime_filter(value: &str) -> Option<&'static str> {
    match value {
        "jpeg" | "jpg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "svg" => Some("image/svg+xml"),
        "webp" => Some("image/webp"),
        "all" => None,
        _ => None,
    }
}

fn format_optional_datetime(value: Option<DateTime<Utc>>) -> String {
    value
        .map(|timestamp| timestamp.to_rfc3339())
        .unwrap_or_else(|| "n/a".to_string())
}

fn display_route_path(route: &str) -> String {
    format!("/{}", route.trim_matches('/'))
}

fn content_status_label(draft: bool) -> String {
    if draft {
        "Draft".to_string()
    } else {
        "Published".to_string()
    }
}

fn latest_revision_summary(revisions: &[entities::content_revision::Model]) -> String {
    revisions
        .first()
        .map(|revision| {
            format!(
                "{} revisions, latest is revision {} by {} on {}",
                revisions.len(),
                revision.revision_number,
                revision.editor_sub,
                revision.created_at.to_rfc3339()
            )
        })
        .unwrap_or_else(|| "No revisions recorded.".to_string())
}

fn normalize_remote_asset_url(value: String) -> Result<Option<Url>, SiteError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let parsed = Url::parse(trimmed)
        .map_err(|error| SiteError::internal(format!("invalid asset import url: {error}")))?;
    match parsed.scheme() {
        "http" | "https" => Ok(Some(parsed)),
        scheme => Err(SiteError::internal(format!(
            "unsupported asset import url scheme: {scheme}"
        ))),
    }
}

fn extension_from_mime_type(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "image/webp" => "webp",
        _ => "bin",
    }
}

async fn fetch_remote_asset(
    client: &reqwest::Client,
    source_url: Url,
) -> Result<(Vec<u8>, String, Option<String>), SiteError> {
    let response = client
        .get(source_url.clone())
        .send()
        .await
        .map_err(|error| SiteError::internal(format!("failed to fetch asset url: {error}")))?;
    if !response.status().is_success() {
        return Err(SiteError::internal(format!(
            "asset import request failed with status {}",
            response.status()
        )));
    }

    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_string());
    if let Some(mime_type) = mime_type.as_deref()
        && !mime_type.starts_with("image/")
    {
        return Err(SiteError::internal(format!(
            "asset import url did not return an image: {mime_type}"
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|error| SiteError::internal(format!("failed to read imported asset: {error}")))?;
    if bytes.is_empty() {
        return Err(SiteError::internal(
            "asset import returned an empty response body".to_string(),
        ));
    }

    let path_filename = source_url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .map(|segment| segment.to_string())
        .filter(|segment| !segment.is_empty());
    let original_filename = match path_filename {
        Some(filename) => filename,
        None => format!(
            "remote-image.{}",
            extension_from_mime_type(mime_type.as_deref().unwrap_or("application/octet-stream"))
        ),
    };

    Ok((bytes.to_vec(), original_filename, mime_type))
}

async fn store_image_asset<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    uploader_sub: &str,
    bytes: Vec<u8>,
    original_filename: String,
    mime_type: Option<String>,
) -> Result<entities::asset::Model, SiteError> {
    let extension = StdPath::new(&original_filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin")
        .to_lowercase();
    let storage_basename = format!("{}.{}", Uuid::now_v7(), extension);
    let upload_root = resolve_upload_root();
    let storage_path = upload_root.join(&storage_basename);

    fs::create_dir_all(&upload_root).await.map_err(|error| {
        SiteError::internal(format!("failed to create upload directory: {error}"))
    })?;
    let mut file = fs::File::create(&storage_path)
        .await
        .map_err(|error| SiteError::internal(format!("failed to create upload file: {error}")))?;
    file.write_all(&bytes)
        .await
        .map_err(|error| SiteError::internal(format!("failed to write upload file: {error}")))?;

    let byte_length = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
    let (dimensions, thumbnail) = generate_thumbnail(bytes, &extension)
        .await
        .map_err(|error| SiteError::internal(format!("failed to process image: {error}")))?;
    let (width, height) = dimensions.unwrap_or((0, 0));
    let width_i32 = if width > 0 {
        i32::try_from(width).ok()
    } else {
        None
    };
    let height_i32 = if height > 0 {
        i32::try_from(height).ok()
    } else {
        None
    };
    let mime_type = mime_type.unwrap_or_else(|| mime_from_extension(&extension).to_string());

    let asset = create_asset(
        db,
        NewAsset {
            site_id,
            uploader_sub: uploader_sub.to_string(),
            original_filename,
            storage_basename: storage_basename.clone(),
            mime_type: mime_type.clone(),
            byte_length,
            width: width_i32,
            height: height_i32,
        },
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to create asset: {error}")))?;

    create_asset_variant(
        db,
        NewAssetVariant {
            asset_id: asset.id,
            variant_kind: "original".to_string(),
            filename: storage_basename,
            mime_type: mime_type.clone(),
            byte_length,
            width: width_i32,
            height: height_i32,
        },
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to create asset variant: {error}")))?;

    if let Some(thumbnail) = thumbnail {
        let stem = StdPath::new(&asset.storage_basename)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("asset");
        let filename = format!("{stem}_thumb.{}", thumbnail.extension);
        let thumb_path = upload_root.join(&filename);
        fs::write(&thumb_path, &thumbnail.bytes)
            .await
            .map_err(|error| SiteError::internal(format!("failed to write thumbnail: {error}")))?;
        create_asset_variant(
            db,
            NewAssetVariant {
                asset_id: asset.id,
                variant_kind: "thumbnail".to_string(),
                filename,
                mime_type: thumbnail.mime_type,
                byte_length: thumbnail.byte_length,
                width: thumbnail.width,
                height: thumbnail.height,
            },
        )
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to create asset thumbnail: {error}"))
        })?;
    }

    Ok(asset)
}

async fn import_remote_scan_asset<C: ConnectionTrait>(
    db: &C,
    client: &reqwest::Client,
    site_id: Uuid,
    uploader_sub: &str,
    remote_url: &str,
) -> Result<AssetReference, SiteError> {
    let url = normalize_remote_asset_url(remote_url.to_string())?
        .ok_or_else(|| SiteError::BadRequest("missing remote asset url".to_string()))?;
    let (bytes, original_filename, mime_type) = fetch_remote_asset(client, url).await?;
    let asset = store_image_asset(
        db,
        site_id,
        uploader_sub,
        bytes,
        original_filename,
        mime_type,
    )
    .await?;
    Ok(AssetReference {
        asset_id: asset.id,
        variant: "original".to_string(),
        asset_label: asset.original_filename,
    })
}

fn format_asset_dimensions(width: Option<i32>, height: Option<i32>) -> String {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => format!("{width} x {height}"),
        (Some(width), None) if width > 0 => format!("{width}w"),
        (None, Some(height)) if height > 0 => format!("{height}h"),
        _ => "n/a".to_string(),
    }
}

async fn build_admin_asset_rows<C: ConnectionTrait>(
    db: &C,
    assets: Vec<entities::asset::Model>,
) -> Result<Vec<AdminAssetRow>, SiteError> {
    if assets.is_empty() {
        return Ok(Vec::new());
    }

    let asset_ids = assets.iter().map(|asset| asset.id).collect::<Vec<_>>();
    let thumbnails = entities::asset_variant::Entity::find()
        .filter(entities::asset_variant::Column::AssetId.is_in(asset_ids))
        .filter(entities::asset_variant::Column::VariantKind.eq("thumbnail"))
        .all(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load asset variants: {error}")))?;

    let thumbnails_by_asset = thumbnails
        .into_iter()
        .map(|variant| (variant.asset_id, variant))
        .collect::<HashMap<_, _>>();

    Ok(assets
        .into_iter()
        .map(|asset| {
            let thumbnail_url = thumbnails_by_asset
                .get(&asset.id)
                .map(|variant| format!("/media/images/{}", variant.filename));

            AdminAssetRow {
                id: asset.id,
                original_filename: asset.original_filename,
                storage_basename: asset.storage_basename.clone(),
                uploader_sub: asset.uploader_sub,
                mime_type: asset.mime_type,
                byte_length: asset.byte_length,
                dimensions: format_asset_dimensions(asset.width, asset.height),
                created_at: asset.created_at.to_rfc3339(),
                original_url: format!("/media/images/{}", asset.storage_basename),
                thumbnail_url,
            }
        })
        .collect())
}

#[utoipa::path(
    get,
    path = "/admin/site/{site_id}/assets/library",
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site to list assets for"),
        ("q" = String, Query, description = "Optional search query to filter assets by original filename or storage basename"),
        ("type" = String, Query, description = "Optional filter by image type (jpeg, png, gif, svg, webp)"),
    ),
    responses(
        (status = 200, description = "A list of assets matching the query", body = AssetLibraryResponse),
        (status = 400, description = "Invalid request parameters", body = String),
        (status = 401, description = "Unauthorized access", body = String),
        (status = 403, description = "Forbidden - insufficient permissions", body = String),
        (status = 500, description = "Internal server error", body = String),
    ),
)]
async fn admin_site_assets_library(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AssetLibraryQuery>,
) -> Result<Json<AssetLibraryResponse>, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;

    let query_text = query.q.unwrap_or_default();
    let query_text = query_text.trim();
    let has_query = !query_text.is_empty();
    let default_limit = if has_query { 50 } else { 12 };
    let limit = query.limit.unwrap_or(default_limit).clamp(1, 200);

    let mut asset_query = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .filter(entities::asset::Column::MimeType.like("image/%"));

    if let Some(type_filter) = query
        .r#type
        .as_deref()
        .and_then(normalize_asset_mime_filter)
    {
        asset_query = asset_query.filter(entities::asset::Column::MimeType.eq(type_filter));
    }

    if has_query {
        let condition = Condition::any()
            .add(entities::asset::Column::OriginalFilename.contains(query_text))
            .add(entities::asset::Column::StorageBasename.contains(query_text));
        asset_query = asset_query.filter(condition);
    }

    let assets = asset_query
        .order_by_desc(entities::asset::Column::CreatedAt)
        .limit(limit)
        .all(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to list assets: {error}")))?;

    if assets.is_empty() {
        return Ok(Json(AssetLibraryResponse { assets: Vec::new() }));
    }

    let asset_ids = assets.iter().map(|asset| asset.id).collect::<Vec<_>>();
    let thumbnails = entities::asset_variant::Entity::find()
        .filter(entities::asset_variant::Column::AssetId.is_in(asset_ids))
        .filter(entities::asset_variant::Column::VariantKind.eq("thumbnail"))
        .all(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load asset variants: {error}")))?;

    let mut thumbnails_by_asset: HashMap<Uuid, entities::asset_variant::Model> = HashMap::new();
    for variant in thumbnails {
        thumbnails_by_asset.insert(variant.asset_id, variant);
    }

    let items = assets
        .into_iter()
        .map(|asset| {
            let thumbnail_url = thumbnails_by_asset
                .get(&asset.id)
                .map(|variant| format!("/media/images/{}", variant.filename));
            let has_thumbnail = thumbnail_url.is_some();
            AssetLibraryItem {
                id: asset.id,
                original_filename: asset.original_filename,
                mime_type: asset.mime_type,
                width: asset.width,
                height: asset.height,
                created_at: asset.created_at.to_rfc3339(),
                original_url: format!("/media/images/{}", asset.storage_basename),
                thumbnail_url,
                has_thumbnail,
            }
        })
        .collect();

    Ok(Json(AssetLibraryResponse { assets: items }))
}

async fn admin_site_assets(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminAssetsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let assets = list_assets(state.db.as_ref(), site_id).await?;
    let site = entities::site::Entity::find_by_id(site_id)
        .one(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?
        .ok_or(SiteError::SiteNotFound(site_id.to_string()))?;
    let asset_rows = build_admin_asset_rows(state.db.as_ref(), assets).await?;

    Ok(AdminAssetsTemplate {
        template_shared: AdminTemplateData::new("Assets")
            .with_site_context(site.id, &site.full_title)
            .with_links(vec![
                AdminLink::new(&format!("/admin/site/{site_id}/assets/new"), "Upload"),
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
            ]),
        site_id,
        site_full_title: site.full_title,
        assets: asset_rows,
    })
}

async fn admin_site_assets_new(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminAssetsNewTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    match get_by_id(state.db.as_ref(), site_id).await {
        Ok(site) => {
            let mut recent_assets = list_assets(state.db.as_ref(), site_id).await?;
            recent_assets.sort_by(|left, right| right.created_at.cmp(&left.created_at));
            let recent_assets = recent_assets.into_iter().take(10).collect::<Vec<_>>();
            let recent_assets = build_admin_asset_rows(state.db.as_ref(), recent_assets).await?;

            Ok(AdminAssetsNewTemplate {
                template_shared: AdminTemplateData::new("Upload Asset")
                    .with_site_context(site.id, &site.full_title)
                    .with_links(vec![
                        AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Back to assets"),
                        AdminLink::new(
                            &format!("/admin/site/{site_id}/content"),
                            "Back to site dashboard",
                        ),
                    ]),
                site_id: site.id,
                site_full_title: site.full_title,
                recent_assets,
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load site {site_id}: {error}"
        ))),
    }
}

async fn admin_site_assets_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let site = match get_by_id(state.db.as_ref(), site_id).await {
        Ok(site) => site,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to load site {site_id}: {error}"
            )));
        }
    };

    let mut upload_bytes: Option<Vec<u8>> = None;
    let mut original_filename: Option<String> = None;
    let mut mime_type: Option<String> = None;
    let mut source_url: Option<Url> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(field) => field,
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to parse upload: {error}"
                )));
            }
        };

        let Some(field) = field else { break };
        match field.name() {
            Some("file") => {
                let field_filename = field.file_name().map(|value| value.to_string());
                let field_mime_type = field.content_type().map(|value| value.to_string());
                let bytes = match field.bytes().await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        return Err(SiteError::internal(format!(
                            "failed to read upload: {error}"
                        )));
                    }
                };
                if bytes.is_empty() {
                    continue;
                }

                original_filename = field_filename;
                mime_type = field_mime_type;
                upload_bytes = Some(bytes.to_vec());
            }
            Some("source_url") => {
                let value = match field.text().await {
                    Ok(value) => value,
                    Err(error) => {
                        return Err(SiteError::internal(format!(
                            "failed to read asset import url: {error}"
                        )));
                    }
                };
                source_url = normalize_remote_asset_url(value)?;
            }
            _ => continue,
        }
    }

    let (bytes, original_filename, mime_type) = if let Some(bytes) = upload_bytes {
        let Some(original_filename) = original_filename else {
            return Err(SiteError::internal("missing original filename".to_string()));
        };
        (bytes, original_filename, mime_type)
    } else if let Some(source_url) = source_url {
        fetch_remote_asset(state.oidc_client.as_ref(), source_url).await?
    } else {
        return Err(SiteError::internal(
            "provide a file upload or an image url".to_string(),
        ));
    };

    let db_txn = state.db.begin().await?;
    let asset = store_image_asset(
        &db_txn,
        site.id,
        &actor.subject,
        bytes,
        original_filename,
        mime_type,
    )
    .await?;

    log_audit_event(
        &db_txn,
        &actor.subject,
        "create_asset",
        "asset",
        &asset.id.to_string(),
        Some(asset.site_id),
        Some(json!({
            "original_filename": &asset.original_filename,
            "storage_basename": &asset.storage_basename,
            "mime_type": &asset.mime_type
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log asset audit: {error}")))?;

    if let Err(error) = db_txn.commit().await {
        return Err(SiteError::internal(format!(
            "failed to commit asset transaction: {error}"
        )));
    }

    Ok(Redirect::to(&format!("/admin/site/{site_id}/assets")))
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn parse_optional_datetime(
    value: Option<String>,
    label: &str,
) -> Result<Option<DateTime<Utc>>, SiteError> {
    value
        .map(|raw| {
            DateTime::parse_from_rfc3339(&raw)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|error| SiteError::internal(format!("invalid {label}: {error}")))
        })
        .transpose()
}

async fn get_template_names() -> Vec<String> {
    let mut templates = vec!["default".to_string()];
    if let Ok(mut entries) = fs::read_dir(crate::constants::SITE_TEMPLATES_DIR).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(file_type) = entry.file_type().await
                && file_type.is_dir()
                && entry.file_name() != "default"
                && let Some(name) = entry.file_name().to_str()
            {
                templates.push(name.to_string());
            }
        }
    }

    templates
}

#[derive(Debug, Deserialize)]
struct SiteTemplateEditorQuery {
    saved: Option<String>,
    reset: Option<String>,
}

fn validate_customizable_template_file(file_name: &str) -> Result<&'static str, SiteError> {
    CUSTOMIZABLE_TEMPLATE_FILES
        .iter()
        .copied()
        .find(|candidate| *candidate == file_name)
        .ok_or_else(|| SiteError::BadRequest(format!("unsupported template file: {file_name}")))
}

fn site_template_edit_href(site_id: Uuid, file_name: &str) -> String {
    format!("/admin/site/{site_id}/settings/templates/{file_name}")
}

fn site_template_reset_href(site_id: Uuid, file_name: &str) -> String {
    format!("/admin/site/{site_id}/settings/templates/{file_name}/reset")
}

async fn describe_template_source_origin(
    site_id: Uuid,
    template_name: &str,
    file_name: &str,
) -> Result<(String, bool), SiteError> {
    let override_path = resolve_site_template_override_root(site_id).join(file_name);
    if fs::metadata(&override_path).await.is_ok() {
        return Ok(("site override".to_string(), true));
    }

    let shared_path = StdPath::new(crate::constants::SITE_TEMPLATES_DIR)
        .join(template_name)
        .join(file_name);
    if fs::metadata(&shared_path).await.is_ok() {
        return Ok((format!("shared template ({template_name})"), false));
    }

    Ok((format!("default template ({DEFAULT_TEMPLATE_NAME})"), false))
}

async fn load_editable_template_source(
    site_id: Uuid,
    template_name: &str,
    file_name: &str,
) -> Result<(String, String, bool), SiteError> {
    let override_path = resolve_site_template_override_root(site_id).join(file_name);
    if let Ok(source) = fs::read_to_string(&override_path).await {
        return Ok((source, "site override".to_string(), true));
    }

    let shared_path = StdPath::new(crate::constants::SITE_TEMPLATES_DIR)
        .join(template_name)
        .join(file_name);
    if let Ok(source) = fs::read_to_string(&shared_path).await {
        return Ok((source, format!("shared template ({template_name})"), false));
    }

    let default_path = StdPath::new(crate::constants::SITE_TEMPLATES_DIR)
        .join(DEFAULT_TEMPLATE_NAME)
        .join(file_name);
    let source = fs::read_to_string(&default_path).await.map_err(|error| {
        SiteError::internal(format!(
            "failed to load default template {file_name}: {error}"
        ))
    })?;
    Ok((
        source,
        format!("default template ({DEFAULT_TEMPLATE_NAME})"),
        false,
    ))
}

async fn build_site_template_file_rows(
    site_id: Uuid,
    template_name: &str,
) -> Result<Vec<AdminSiteTemplateFileRow>, SiteError> {
    let mut rows = Vec::with_capacity(CUSTOMIZABLE_TEMPLATE_FILES.len());
    for file_name in CUSTOMIZABLE_TEMPLATE_FILES {
        let (source_origin, override_exists) =
            describe_template_source_origin(site_id, template_name, file_name).await?;
        rows.push(AdminSiteTemplateFileRow {
            file_name: file_name.to_string(),
            source_origin,
            edit_href: site_template_edit_href(site_id, file_name),
            reset_href: site_template_reset_href(site_id, file_name),
            override_exists,
        });
    }

    Ok(rows)
}

async fn admin_site_settings(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminSiteSettingsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let viewer = current_user(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let template_files = build_site_template_file_rows(site.id, &site.template_name).await?;
    let export_href = if viewer.admin {
        Some(format!("/admin/site/{site_id}/export.json"))
    } else {
        let membership = get_membership_for_subject(state.db.as_ref(), site_id, &viewer.subject)
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to load export membership: {error}"))
            })?;
        membership
            .filter(|membership| role_satisfies(membership.role, SiteRole::Owner))
            .map(|_| format!("/admin/site/{site_id}/export.json"))
    };

    Ok(AdminSiteSettingsTemplate {
        template_shared: AdminTemplateData::new("Site Settings")
            .with_site_context(site.id, &site.full_title)
            .with_links(vec![
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
                AdminLink::new(&format!("/admin/site/{site_id}/memberships"), "Memberships"),
            ]),

        site_id: site.id,
        site_short_name: site.short_name,
        full_title: site.full_title,
        template_name: site.template_name,
        can_delete_site: viewer.admin,
        export_href,
        templates: get_template_names().await,
        template_files,
    })
}

async fn admin_site_settings_update(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<UpdateSiteSettingsForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let actor = current_user(&session).await?.subject;
    let full_title = form.full_title.trim().to_string();
    let template_name = form.template_name.trim().to_string();
    if full_title.is_empty() {
        return Err(SiteError::internal("missing full title".to_string()));
    }
    if template_name.is_empty() {
        return Err(SiteError::internal("missing template name".to_string()));
    }

    let txn = state.db.begin().await?;
    let actor = actor.clone();
    let full_title = full_title.clone();
    let template_name = template_name.clone();
    let site = update_site_settings(&txn, site_id, full_title, template_name)
        .await
        .map_err(|error| SiteError::internal(format!("failed to update site: {error}")))?;
    log_audit_event(
        &txn,
        &actor,
        "update_site",
        "site",
        &site.id.to_string(),
        Some(site.id),
        Some(json!({
            "short_name": site.short_name,
            "full_title": site.full_title,
            "template_name": site.template_name
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log audit: {error}")))?;
    txn.commit().await?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/settings")))
}

async fn collect_site_media_filenames(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> Result<Vec<String>, SiteError> {
    let assets = list_assets(db, site_id).await?;
    let asset_ids = assets.iter().map(|asset| asset.id).collect::<Vec<_>>();
    let mut filenames = assets
        .into_iter()
        .map(|asset| asset.storage_basename)
        .collect::<Vec<_>>();

    if !asset_ids.is_empty() {
        let variants = entities::asset_variant::Entity::find()
            .filter(entities::asset_variant::Column::AssetId.is_in(asset_ids))
            .all(db)
            .await
            .map_err(SiteError::from)?;
        filenames.extend(variants.into_iter().map(|variant| variant.filename));
    }

    filenames.sort();
    filenames.dedup();
    Ok(filenames)
}

async fn remove_deleted_site_files(
    site_id: Uuid,
    media_filenames: &[String],
) -> Result<(), SiteError> {
    for filename in media_filenames {
        let path = resolve_upload_root().join(filename);
        match fs::remove_file(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to remove deleted site media file {}: {error}",
                    path.display()
                )));
            }
        }
    }

    let override_root = resolve_site_template_override_root(site_id);
    match fs::remove_dir_all(&override_root).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to remove deleted site template overrides {}: {error}",
                override_root.display()
            )));
        }
    }

    Ok(())
}

async fn admin_site_delete_confirm(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminSiteDeleteConfirmTemplate, SiteError> {
    require_global_admin(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let csrf_token = session
        .issue_csrf_token(&site_delete_csrf_scope(site_id))
        .await?;

    Ok(AdminSiteDeleteConfirmTemplate {
        template_shared: AdminTemplateData::new("Confirm Site Deletion")
            .with_site_context(site.id, &site.full_title)
            .with_links(vec![AdminLink::new(
                &format!("/admin/site/{site_id}/settings"),
                "Back to settings",
            )]),
        site_id: site.id,
        site_full_title: site.full_title,
        site_short_name: site.short_name,
        csrf_token,
    })
}

async fn admin_site_delete(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<CsrfTokenForm>,
) -> Result<Redirect, SiteError> {
    let actor = require_global_admin(&session).await?.subject;
    session
        .validate_csrf_token(&site_delete_csrf_scope(site_id), &form.csrf_token)
        .await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let media_filenames = collect_site_media_filenames(state.db.as_ref(), site_id).await?;

    let txn = state.db.begin().await?;
    log_audit_event(
        &txn,
        &actor,
        "delete_site",
        "site",
        &site.id.to_string(),
        Some(site.id),
        Some(json!({
            "short_name": site.short_name,
            "full_title": site.full_title,
            "template_name": site.template_name
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log site delete audit: {error}")))?;
    delete_site(&txn, site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to delete site: {error}")))?;
    txn.commit().await?;

    remove_deleted_site_files(site_id, &media_filenames).await?;

    Ok(Redirect::to("/admin?deleted=1"))
}

async fn admin_site_template_editor(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, file_name)): Path<(Uuid, String)>,
    Query(query): Query<SiteTemplateEditorQuery>,
) -> Result<AdminSiteTemplateEditorTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let file_name = validate_customizable_template_file(&file_name)?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let (source, source_origin, override_exists) =
        load_editable_template_source(site.id, &site.template_name, file_name).await?;

    let template_shared = AdminTemplateData::new(format!("Template Override: {file_name}"))
        .with_site_context(site.id, &site.full_title)
        .with_links(vec![
            AdminLink::new(
                &format!("/admin/site/{site_id}/settings"),
                "Back to settings",
            ),
            AdminLink::new(&format!("/admin/site/{site_id}/render"), "Render site"),
        ]);
    let template_shared = if query.saved.is_some() {
        template_shared.with_toast_message("Template override saved.", "saved")
    } else if query.reset.is_some() {
        template_shared.with_toast_message("Template override reset.", "reset")
    } else {
        template_shared
    };

    Ok(AdminSiteTemplateEditorTemplate {
        template_shared,
        site_id: site.id,
        site_full_title: site.full_title,
        template_name: site.template_name,
        file_name: file_name.to_string(),
        source,
        source_origin,
        override_exists,
    })
}

async fn admin_site_template_override_update(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, file_name)): Path<(Uuid, String)>,
    Form(form): Form<UpdateSiteTemplateOverrideForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let file_name = validate_customizable_template_file(&file_name)?;
    if form.source.trim().is_empty() {
        return Err(SiteError::BadRequest("missing template source".to_string()));
    }

    let actor = current_user(&session).await?.subject;
    let override_root = resolve_site_template_override_root(site_id);
    fs::create_dir_all(&override_root).await.map_err(|error| {
        SiteError::internal(format!(
            "failed to create template override directory: {error}"
        ))
    })?;
    let override_path = override_root.join(file_name);
    fs::write(&override_path, form.source.as_bytes())
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to write template override {file_name}: {error}"
            ))
        })?;

    let txn = state.db.begin().await?;
    log_audit_event(
        &txn,
        &actor,
        "update_template_override",
        "site_template_override",
        file_name,
        Some(site_id),
        Some(json!({ "file_name": file_name })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log template audit: {error}")))?;
    txn.commit().await?;

    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/settings/templates/{file_name}?saved=1"
    )))
}

async fn admin_site_template_override_reset(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, file_name)): Path<(Uuid, String)>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let file_name = validate_customizable_template_file(&file_name)?;
    let actor = current_user(&session).await?.subject;
    let override_path = resolve_site_template_override_root(site_id).join(file_name);
    match fs::remove_file(&override_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to remove template override {file_name}: {error}"
            )));
        }
    }

    let txn = state.db.begin().await?;
    log_audit_event(
        &txn,
        &actor,
        "reset_template_override",
        "site_template_override",
        file_name,
        Some(site_id),
        Some(json!({ "file_name": file_name })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log template reset audit: {error}")))?;
    txn.commit().await?;

    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/settings/templates/{file_name}?reset=1"
    )))
}

async fn admin_site_render(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminRenderTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Editor).await?;
    let site = entities::site::Entity::find_by_id(site_id)
        .one(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?
        .ok_or(SiteError::NotFound)?;
    render_site(
        state.db.as_ref(),
        site_id,
        std::path::Path::new(crate::constants::SITE_TEMPLATES_DIR),
        std::path::Path::new(crate::constants::RENDERED_DIR),
        &resolve_upload_root(),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to render site {site_id}: {error}")))
    .map(|files_written| AdminRenderTemplate {
        template_shared: AdminTemplateData::new("Render Site")
            .with_site_context(site.id, &site.full_title)
            .with_message(format!(
                "Site rendered with {} file(s) written.",
                files_written
            ))
            .with_links(vec![
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
                AdminLink::new(&format!("/admin/site/{site_id}/render"), "Run render again"),
            ]),
    })
}

async fn admin_site_export(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<Response, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let export = export_site(state.db.as_ref(), site_id).await?;
    let file_name = format!("{}-site-export.json", export.site.short_name);
    let body = serialize_site_export_pretty(&export)?;
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{file_name}\"")).map_err(
            |error| SiteError::internal(format!("failed to build export download header: {error}")),
        )?,
    );

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::SESSION_USER;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;
    use tower_sessions::{Expiry, MemoryStore, SessionManagerLayer};

    #[tokio::test]
    async fn ensure_site_owner_membership_is_idempotent() {
        let db = crate::db::db_start("sqlite::memory:")
            .await
            .expect("failed to start db");
        let site = crate::create_site(
            db.as_ref(),
            "test".to_string(),
            "Test Site".to_string(),
            DEFAULT_TEMPLATE_NAME.to_string(),
        )
        .await
        .expect("failed to create site");

        let first = match ensure_site_owner_membership(db.as_ref(), "tester", None, site.id).await {
            Ok(value) => value,
            Err(_) => panic!("failed to create membership"),
        };
        assert!(first.is_some(), "expected membership on first call");
        if let Some(membership) = first {
            assert_eq!(
                membership.role,
                SiteRole::Owner,
                "expected role to be owner"
            );
        }

        let second = match ensure_site_owner_membership(db.as_ref(), "tester", None, site.id).await
        {
            Ok(value) => value,
            Err(_) => panic!("failed to check membership"),
        };
        assert!(second.is_none(), "expected no membership on second call");
    }

    #[test]
    fn rewrite_preview_asset_urls_rewrites_root_asset_paths() {
        let site_id = Uuid::nil();
        let rendered = rewrite_preview_asset_urls(
            r#"<link href="/assets/style.css"><style>body{background:url('/assets/bg.png')}</style>"#,
            site_id,
        );

        assert!(
            rendered.contains(
                "/admin/site/00000000-0000-0000-0000-000000000000/preview-assets/style.css"
            )
        );
        assert!(
            rendered
                .contains("/admin/site/00000000-0000-0000-0000-000000000000/preview-assets/bg.png")
        );
    }

    #[test]
    fn sanitize_preview_asset_path_rejects_parent_components() {
        assert!(sanitize_preview_asset_path("../secret.txt").is_err());
        assert!(sanitize_preview_asset_path("/etc/passwd").is_err());
    }

    #[test]
    fn sanitize_preview_asset_path_allows_nested_relative_paths() {
        let path = sanitize_preview_asset_path("css/site/style.css")
            .expect("expected nested preview asset path to be accepted");

        assert_eq!(path, PathBuf::from("css/site/style.css"));
    }

    #[test]
    fn role_satisfies_enforces_site_role_hierarchy() {
        assert!(role_satisfies(SiteRole::Viewer, SiteRole::Viewer));
        assert!(role_satisfies(SiteRole::Author, SiteRole::Viewer));
        assert!(role_satisfies(SiteRole::Editor, SiteRole::Author));
        assert!(role_satisfies(SiteRole::Owner, SiteRole::Editor));
        assert!(role_satisfies(SiteRole::Admin, SiteRole::Owner));

        assert!(!role_satisfies(SiteRole::Viewer, SiteRole::Author));
        assert!(!role_satisfies(SiteRole::Author, SiteRole::Editor));
        assert!(!role_satisfies(SiteRole::Editor, SiteRole::Owner));
    }

    #[test]
    fn sort_content_items_orders_titles_descending_case_insensitively() {
        fn content(title: &str, page_type: PageType) -> entities::content_item::Model {
            entities::content_item::Model {
                id: Uuid::now_v7(),
                site_id: Uuid::now_v7(),
                page_type,
                title: title.to_string(),
                slug: title.to_lowercase().replace(' ', "-"),
                page_content: String::new(),
                draft: true,
                creator_sub: "tester".to_string(),
                created_at: DateTime::parse_from_rfc3339("2026-03-09T00:00:00Z")
                    .expect("invalid created_at")
                    .with_timezone(&Utc),
                last_updated: None,
                published_at: None,
            }
        }

        let mut items = vec![
            content("alpha page", PageType::Page),
            content("Zulu post", PageType::Post),
            content("Beta page", PageType::Page),
        ];

        sort_content_items(&mut items, ContentListSortBy::TitleDesc);

        let titles = items.into_iter().map(|item| item.title).collect::<Vec<_>>();
        assert_eq!(titles, vec!["Zulu post", "Beta page", "alpha page"]);
    }

    #[tokio::test]
    async fn can_view_user_profile_allows_self_and_admin_only() {
        let db = crate::db::db_start("sqlite::memory:")
            .await
            .expect("failed to start db");
        let viewer = crate::entities::user::create_user(db.as_ref(), "viewer", None, None, false)
            .await
            .expect("failed to create viewer");
        let target = crate::entities::user::create_user(db.as_ref(), "target", None, None, false)
            .await
            .expect("failed to create target");
        let admin = crate::entities::user::create_user(db.as_ref(), "admin", None, None, true)
            .await
            .expect("failed to create admin");

        assert!(can_view_user_profile(&viewer, &viewer));
        assert!(!can_view_user_profile(&viewer, &target));
        assert!(can_view_user_profile(&admin, &target));
    }

    async fn test_login(
        State(state): State<AdminState>,
        session: Session,
        Path(user_id): Path<Uuid>,
    ) -> Result<StatusCode, SiteError> {
        let user = get_user_by_id(state.db.as_ref(), user_id)
            .await?
            .ok_or(SiteError::NotFound)?;
        session
            .insert(SESSION_USER, user)
            .await
            .map_err(|_| SiteError::internal("failed to seed session".to_string()))?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn seed_session_cookie(router: Router, user_id: Uuid) -> String {
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/test-login/{user_id}"))
                    .body(Body::empty())
                    .expect("failed to build login request"),
            )
            .await
            .expect("failed to perform login request");
        let cookie = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .next()
            .expect("missing set-cookie header")
            .to_str()
            .expect("invalid set-cookie header");
        cookie
            .split(';')
            .next()
            .expect("missing cookie pair")
            .to_string()
    }

    fn test_admin_state(db: std::sync::Arc<DatabaseConnection>) -> AdminState {
        AdminState {
            db,
            oidc_client_id: ClientId::new("client".to_string()),
            oidc_client_secret: None,
            oidc_frontend_url: Url::parse("https://example.com").expect("invalid frontend url"),
            oidc_discovery_url: IssuerUrl::new("https://example.com".to_string())
                .expect("invalid discovery url"),
            oidc_client: std::sync::Arc::new(
                build_http_client().expect("failed to build test oidc client"),
            ),
        }
    }

    fn site_transfer_test_router(state: AdminState) -> Router {
        let session_layer = SessionManagerLayer::new(MemoryStore::default())
            .with_secure(false)
            .with_expiry(Expiry::OnSessionEnd);
        let protected = Router::new()
            .route(
                "/admin/sites/import",
                get(admin_sites_import).post(admin_sites_import_create),
            )
            .route("/admin/site/{site_id}/export.json", get(admin_site_export))
            .layer(from_fn(crate::middleware::require_session));

        Router::new()
            .route("/test-login/{user_id}", get(test_login))
            .merge(protected)
            .layer(session_layer)
            .with_state(state)
    }

    fn multipart_json_request_body(json: &str) -> (String, Vec<u8>) {
        let boundary = "site-import-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"site-export.json\"\r\nContent-Type: application/json\r\n\r\n{json}\r\n--{boundary}--\r\n"
        );
        (boundary.to_string(), body.into_bytes())
    }

    #[tokio::test]
    async fn admin_site_export_allows_owner_and_sets_download_headers() {
        let db = crate::db::db_start("sqlite::memory:")
            .await
            .expect("failed to start db");
        let site = crate::create_site(
            db.as_ref(),
            "export-site".to_string(),
            "Export Site".to_string(),
            DEFAULT_TEMPLATE_NAME.to_string(),
        )
        .await
        .expect("failed to create site");
        let owner = crate::entities::user::create_user(
            db.as_ref(),
            "owner",
            Some("owner@example.com"),
            Some("Owner"),
            false,
        )
        .await
        .expect("failed to create owner");
        crate::create_membership(
            db.as_ref(),
            crate::NewMembership {
                site_id: site.id,
                user_id: owner.id,
                role: SiteRole::Owner,
            },
        )
        .await
        .expect("failed to create owner membership");

        let router = site_transfer_test_router(test_admin_state(db.clone()));
        let cookie = seed_session_cookie(router.clone(), owner.id).await;
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/admin/site/{}/export.json", site.id))
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("failed to build export request"),
            )
            .await
            .expect("failed to call export route");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .expect("missing content-type")
                .to_str()
                .expect("invalid content-type"),
            "application/json"
        );
        assert!(
            response
                .headers()
                .get(header::CONTENT_DISPOSITION)
                .expect("missing content-disposition")
                .to_str()
                .expect("invalid content-disposition")
                .contains("attachment; filename=\"export-site-site-export.json\"")
        );
    }

    #[tokio::test]
    async fn admin_site_export_rejects_non_owner_members() {
        let db = crate::db::db_start("sqlite::memory:")
            .await
            .expect("failed to start db");
        let site = crate::create_site(
            db.as_ref(),
            "export-site".to_string(),
            "Export Site".to_string(),
            DEFAULT_TEMPLATE_NAME.to_string(),
        )
        .await
        .expect("failed to create site");
        let viewer = crate::entities::user::create_user(
            db.as_ref(),
            "viewer",
            Some("viewer@example.com"),
            Some("Viewer"),
            false,
        )
        .await
        .expect("failed to create viewer");
        crate::create_membership(
            db.as_ref(),
            crate::NewMembership {
                site_id: site.id,
                user_id: viewer.id,
                role: SiteRole::Viewer,
            },
        )
        .await
        .expect("failed to create viewer membership");

        let router = site_transfer_test_router(test_admin_state(db.clone()));
        let cookie = seed_session_cookie(router.clone(), viewer.id).await;
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/admin/site/{}/export.json", site.id))
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("failed to build export request"),
            )
            .await
            .expect("failed to call export route");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_site_import_allows_global_admin_and_creates_site() {
        let db = crate::db::db_start("sqlite::memory:")
            .await
            .expect("failed to start db");
        let admin = crate::entities::user::create_user(
            db.as_ref(),
            "admin",
            Some("admin@example.com"),
            Some("Admin"),
            true,
        )
        .await
        .expect("failed to create admin");
        let export = crate::SiteExport {
            format_version: crate::SITE_EXPORT_FORMAT_VERSION,
            exported_at: Utc::now(),
            site: crate::site_export::ExportSite {
                id: Uuid::now_v7(),
                short_name: "imported-site".to_string(),
                full_title: "Imported Site".to_string(),
                template_name: DEFAULT_TEMPLATE_NAME.to_string(),
                created_at: Utc::now(),
                updated_at: None,
            },
            memberships: Vec::new(),
            tags: Vec::new(),
            content_items: Vec::new(),
            assets: Vec::new(),
            audit_events: Vec::new(),
            template_overrides: Vec::new(),
        };
        let json =
            crate::serialize_site_export_pretty(&export).expect("failed to serialize import json");
        let (boundary, body) = multipart_json_request_body(&json);

        let router = site_transfer_test_router(test_admin_state(db.clone()));
        let cookie = seed_session_cookie(router.clone(), admin.id).await;
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/sites/import")
                    .header(header::COOKIE, cookie)
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .expect("failed to build import request"),
            )
            .await
            .expect("failed to call import route");

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .expect("missing location header")
                .to_str()
                .expect("invalid location header"),
            "/admin?imported=1"
        );

        let imported_site = crate::entities::site::Entity::find()
            .filter(crate::entities::site::Column::ShortName.eq("imported-site"))
            .one(db.as_ref())
            .await
            .expect("failed to query imported site");
        assert!(imported_site.is_some(), "expected imported site to exist");
    }

    #[tokio::test]
    async fn admin_site_import_rejects_non_admin_users() {
        let db = crate::db::db_start("sqlite::memory:")
            .await
            .expect("failed to start db");
        let user = crate::entities::user::create_user(
            db.as_ref(),
            "viewer",
            Some("viewer@example.com"),
            Some("Viewer"),
            false,
        )
        .await
        .expect("failed to create user");
        let export = crate::SiteExport {
            format_version: crate::SITE_EXPORT_FORMAT_VERSION,
            exported_at: Utc::now(),
            site: crate::site_export::ExportSite {
                id: Uuid::now_v7(),
                short_name: "blocked-import".to_string(),
                full_title: "Blocked Import".to_string(),
                template_name: DEFAULT_TEMPLATE_NAME.to_string(),
                created_at: Utc::now(),
                updated_at: None,
            },
            memberships: Vec::new(),
            tags: Vec::new(),
            content_items: Vec::new(),
            assets: Vec::new(),
            audit_events: Vec::new(),
            template_overrides: Vec::new(),
        };
        let json =
            crate::serialize_site_export_pretty(&export).expect("failed to serialize import json");
        let (boundary, body) = multipart_json_request_body(&json);

        let router = site_transfer_test_router(test_admin_state(db.clone()));
        let cookie = seed_session_cookie(router.clone(), user.id).await;
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/sites/import")
                    .header(header::COOKIE, cookie)
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .expect("failed to build import request"),
            )
            .await
            .expect("failed to call import route");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
