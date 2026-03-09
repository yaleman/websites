use crate::constants::{
    CUSTOMIZABLE_TEMPLATE_FILES, DEFAULT_TEMPLATE_NAME, SESSION_OIDC_NONCE_KEY,
    SESSION_OIDC_PKCE_KEY, SESSION_OIDC_STATE_KEY, SESSION_USER,
};
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
    delete_membership, delete_tag, get_membership_by_id, get_membership_for_subject, get_revision,
    get_revision_by_number, get_user_by_id, list_aliases, list_assets, list_content,
    list_content_tags, list_memberships, list_memberships_for_user_id, list_revisions, list_sites,
    list_sites_for_subject, list_tags, list_users, list_users_by_ids, render_content_preview,
    render_site, resolve_site_template_override_root, resolve_upload_root, search_content,
    sync_tags_to_content, update_content, update_membership_role, update_site_settings,
};
use anyhow::Context;
use askama::Template;
use askama_web::WebTemplate;
use axum::middleware::{Next, from_fn};
use axum::{
    Json, Router,
    body::Body,
    extract::{Form, Multipart, OriginalUri, Path, Query, Request, State},
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
use std::collections::HashMap;
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
use uuid::Uuid;

/// Holds all the common template-shared data for admin pages.
struct AdminTemplateData {
    page_title: String,
    page_message: Option<String>,
    page_message_is_toast: bool,
    clear_query_param: Option<String>,
    site_id: Option<Uuid>,
    links: Vec<AdminLink>,
}

impl AdminTemplateData {
    pub fn new(title: impl ToString) -> Self {
        Self {
            page_title: title.to_string(),
            page_message: None,
            page_message_is_toast: false,
            clear_query_param: None,
            site_id: None,
            links: vec![],
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

    pub fn with_site_id(self, site_id: Uuid) -> Self {
        Self {
            site_id: Some(site_id),
            ..self
        }
    }

    pub fn with_links(self, links: Vec<AdminLink>) -> Self {
        Self { links, ..self }
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
#[template(path = "admin_content_search.html")]
struct AdminSearchTemplate {
    template_shared: AdminTemplateData,
    results: Vec<entities::content_item::Model>,
    query: String,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_detail.html")]
struct AdminContentDetailTemplate {
    template_shared: AdminTemplateData,
    title: String,
    page_type: String,
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
    site_short_name: String,
    assets: Vec<AdminAssetRow>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_assets_new.html")]
struct AdminAssetsNewTemplate {
    template_shared: AdminTemplateData,
    site_id: Uuid,
    site_short_name: String,
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
    content_items: Vec<entities::content_item::Model>,
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
#[template(path = "sites.html")]
struct AdminSitesTemplate {
    template_shared: AdminTemplateData,

    sites: Vec<crate::entities::site::Model>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "content_new.html")]
struct AdminContentNewTemplate {
    template_shared: AdminTemplateData,
    tags: Vec<AdminTagOption>,

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
#[template(path = "site_settings.html")]
struct AdminSiteSettingsTemplate {
    template_shared: AdminTemplateData,

    site_id: Uuid,
    site_short_name: String,
    full_title: String,
    template_name: String,
    templates: Vec<String>,
    template_files: Vec<AdminSiteTemplateFileRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "site_template_editor.html")]
struct AdminSiteTemplateEditorTemplate {
    template_shared: AdminTemplateData,
    site_id: Uuid,
    site_short_name: String,
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
}

impl AdminLink {
    fn new(href: &str, label: &str) -> Self {
        Self {
            href: href.to_string(),
            label: label.to_string(),
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

fn parse_tag_list(raw: Option<String>) -> Vec<String> {
    raw.unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[derive(Debug, Deserialize)]
struct AssetLibraryQuery {
    q: Option<String>,
    limit: Option<u64>,
    r#type: Option<String>,
}

#[derive(Debug, Serialize)]
struct AssetLibraryResponse {
    assets: Vec<AssetLibraryItem>,
}

#[derive(Debug, Serialize)]
struct AssetLibraryItem {
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
        .route("/admin", get(admin_index))
        .route("/admin/sites", get(admin_sites))
        .route("/admin/users/me", get(admin_user_profile_redirect))
        .route("/admin/users/{user_id}", get(admin_user_profile))
        .route(
            "/admin/sites/new",
            get(admin_sites_new).post(admin_sites_create),
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
        .route("/admin/site/{site_id}/search", get(admin_site_search))
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
        .layer(from_fn(require_session));

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

    let app = Router::new()
        .route("/", get(admin_root))
        .route("/admin/login", get(admin_login))
        .route("/oauth2/callback", get(admin_login_callback))
        .route("/admin/logout", get(admin_logout))
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

async fn require_session(session: Session, request: Request, next: Next) -> Response {
    let is_authenticated = session
        .get::<entities::user::Model>(SESSION_USER)
        .await
        .unwrap_or(None)
        .is_some();
    if is_authenticated {
        next.run(request).await
    } else {
        Redirect::to("/admin/login").into_response()
    }
}

async fn admin_index(session: Session) -> Result<AdminIndexTemplate, SiteError> {
    let user = current_user(&session).await?;
    Ok(AdminIndexTemplate {
        template_shared: AdminTemplateData::new("Admin Dashboard").with_links(vec![
            AdminLink::new(&format!("/admin/users/{}", user.id), "My profile"),
        ]),
    })
}

async fn admin_sites(State(state): State<AdminState>) -> Result<AdminSitesTemplate, SiteError> {
    let sites = list_sites(state.db.as_ref()).await?;

    Ok(AdminSitesTemplate {
        template_shared: AdminTemplateData::new("Sites")
            .with_links(vec![AdminLink::new("/admin/sites/new", "New site")]),

        sites,
    })
}

async fn admin_sites_new() -> Response {
    AdminSitesNewTemplate {
        template_shared: AdminTemplateData::new("Create Site")
            .with_links(vec![AdminLink::new("/admin/sites", "Back to sites")]),

        templates: get_template_names().await,
    }
    .into_response()
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
    Ok(Redirect::to("/admin/sites"))
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
) -> Result<AdminContentListTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;

    match list_content(state.db.as_ref(), site_id, None).await {
        Ok(pages) => Ok(AdminContentListTemplate {
            template_shared: AdminTemplateData::new(site.full_title).with_links(vec![
                AdminLink::new(&format!("/admin/site/{site_id}/content/new"), "New content"),
                AdminLink::new(&format!("/admin/site/{site_id}/search"), "Search content"),
                AdminLink::new(&format!("/admin/site/{site_id}/memberships"), "Memberships"),
                AdminLink::new(&format!("/admin/site/{site_id}/tags"), "Tags"),
                AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Assets"),
                AdminLink::new(&format!("/admin/site/{site_id}/render"), "Render"),
                AdminLink::new(&format!("/admin/site/{site_id}/settings"), "Site settings"),
            ]),

            site_id,
            content_items: pages,
        }),
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
        template_shared: AdminTemplateData::new("Memberships").with_links(vec![
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

async fn admin_site_search(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<SearchQuery>,
) -> Result<AdminSearchTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let query_text = query.q.unwrap_or_default();
    let mut results = Vec::new();
    let mut message = "Search content by title, slug, or body text.".to_string();

    if !query_text.trim().is_empty() {
        match search_content(state.db.as_ref(), site_id, query_text.trim()).await {
            Ok(items) => {
                message = format!("Found {} result(s) for \"{}\".", items.len(), query_text);
                results = items.into_iter().collect();
            }
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to search content: {error}"
                )));
            }
        }
    }

    Ok(AdminSearchTemplate {
        template_shared: AdminTemplateData::new("Search")
            .with_message(message)
            .with_links(vec![
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
                AdminLink::new(&format!("/admin/site/{site_id}/content/new"), "New content"),
            ]),
        results,
        query: query_text.trim().to_string(),
    })
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
        template_shared: AdminTemplateData::new(format!("{} - Create Content", &site.short_name)),

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

    let route = content_primary_route(&content);
    Ok(AdminContentDetailTemplate {
        template_shared: AdminTemplateData::new(format!("Content: /{route}"))
            .with_message(format!("Creator: {}", content.creator_sub))
            .with_site_id(content.site_id)
            .with_links(vec![
                AdminLink::new(
                    &format!(
                        "/admin/site/{}/content/{}/edit",
                        content.site_id, content.id
                    ),
                    "Return to editor",
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
        page_type: content.page_type.to_string(),
        status: content_status_label(content.draft),
        primary_route: display_route_path(&route),
        revisions_summary: latest_revision_summary(&revisions),
        tags,
        aliases,
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

    let template_shared = AdminTemplateData::new(format!("Editing: {}", title))
        .with_links(vec![
            AdminLink::new(&preview_href, "Preview"),
            AdminLink::new(&back_href, "Back to site dashboard"),
        ])
        .with_site_id(content.site_id);
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
        template_shared: AdminTemplateData::new(format!("Revisions for {content_id}")).with_links(
            vec![AdminLink::new(
                &format!("/admin/site/{site_id}/content/{content_id}"),
                "Back to site dashboard",
            )],
        ),
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
                template_shared: AdminTemplateData::new(format!("Site Tags ({site_id})"))
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
        template_shared: AdminTemplateData::new(format!("Site Assets ({})", site.short_name))
            .with_links(vec![
                AdminLink::new(&format!("/admin/site/{site_id}/assets/new"), "Upload asset"),
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
            ]),
        site_id,
        site_short_name: site.short_name,
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
                template_shared: AdminTemplateData::new(format!(
                    "Upload Asset {}",
                    site.short_name
                ))
                .with_links(vec![
                    AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Back to assets"),
                    AdminLink::new(
                        &format!("/admin/site/{site_id}/content"),
                        "Back to site dashboard",
                    ),
                ]),
                site_id: site.id,
                site_short_name: site.short_name,
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

    let extension = StdPath::new(&original_filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin")
        .to_lowercase();
    let storage_basename = format!("{}.{}", Uuid::now_v7(), extension);
    let upload_root = resolve_upload_root();
    let storage_path = upload_root.join(&storage_basename);

    if let Err(error) = fs::create_dir_all(&upload_root).await {
        return Err(SiteError::internal(format!(
            "failed to create upload directory: {error}"
        )));
    }

    match fs::File::create(&storage_path).await {
        Ok(mut file) => {
            if let Err(error) = file.write_all(&bytes).await {
                return Err(SiteError::internal(format!(
                    "failed to write upload file: {error}"
                )));
            }
        }
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to create upload file: {error}"
            )));
        }
    }

    let byte_length = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
    let (dimensions, thumbnail) = match generate_thumbnail(bytes, &extension).await {
        Ok(result) => result,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to process image: {error}"
            )));
        }
    };

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

    let db_txn = state.db.begin().await?;

    let original_filename = original_filename.clone();
    let storage_basename = storage_basename.clone();
    let mime_type = mime_type.clone();
    let asset = create_asset(
        &db_txn,
        NewAsset {
            site_id: site.id,
            uploader_sub: actor.subject.clone(),
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

    create_asset_variant(
        &db_txn,
        NewAssetVariant {
            asset_id: asset.id,
            variant_kind: "original".to_string(),
            filename: storage_basename.clone(),
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
        if let Err(error) = fs::write(&thumb_path, &thumbnail.bytes).await {
            return Err(SiteError::internal(format!(
                "failed to write thumbnail: {error}"
            )));
        }

        create_asset_variant(
            &db_txn,
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
            SiteError::internal(format!("failed to create thumbnail variant: {error}"))
        })?;
    }

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
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let template_files = build_site_template_file_rows(site.id, &site.template_name).await?;

    Ok(AdminSiteSettingsTemplate {
        template_shared: AdminTemplateData::new("Site Settings").with_links(vec![
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
        .with_site_id(site.id)
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
        site_short_name: site.short_name,
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
        template_shared: AdminTemplateData::new(format!("Rendered site '{}'", site.full_title))
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
