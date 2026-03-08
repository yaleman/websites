use crate::constants::{
    DEFAULT_TEMPLATE_NAME, SESSION_OIDC_NONCE_KEY, SESSION_OIDC_PKCE_KEY, SESSION_OIDC_STATE_KEY,
    SESSION_USER,
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
    create_asset_variant, create_content, create_membership, create_site, delete_membership,
    get_membership_by_id, get_membership_for_subject, get_revision, get_revision_by_number,
    get_user_by_id, list_aliases, list_assets, list_content, list_content_tags, list_memberships,
    list_memberships_for_user_id, list_revisions, list_sites, list_sites_for_subject, list_tags,
    list_users_by_ids, render_content_preview, render_site, resolve_upload_root, search_content,
    update_content, update_membership_role, update_site_settings,
};
use anyhow::Context;
use askama::Template;
use askama_web::WebTemplate;
use axum::middleware::{Next, from_fn};
use axum::{
    Json, Router,
    body::Body,
    extract::{Form, Multipart, Path, Query, Request, State},
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

    rows: Vec<AdminRow>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_advanced.html")]
struct AdminContentAdvancedTemplate {
    template_shared: AdminTemplateData,

    rows: Vec<AdminRow>,
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

    rows: Vec<AdminRow>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_assets_new.html")]
struct AdminAssetsNewTemplate {
    template_shared: AdminTemplateData,

    rows: Vec<AdminRow>,
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

    rows: Vec<AdminRow>,
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
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_user_profile.html")]
struct AdminUserProfileTemplate {
    template_shared: AdminTemplateData,

    user_id: Uuid,
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
struct AdminTagOption {
    name: String,
}

#[derive(Debug)]
struct AdminMembershipRow {
    subject: String,
    email: String,
    role: SiteRole,
    profile_href: Option<String>,
    update_href: String,
    remove_href: String,
}

#[derive(Debug)]
struct AdminUserMembershipRow {
    site_title: String,
    site_short_name: String,
    role: SiteRole,
    site_href: String,
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
struct CreateContentForm {
    page_type: String,
    title: String,
    slug: String,
    page_content: String,
    draft: Option<bool>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateContentForm {
    page_type: String,
    title: String,
    slug: String,
    page_content: String,
    draft: String,
    published_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MembershipCreateForm {
    subject: String,
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
            "/admin/site/{site_id}/content/{content_id}/source",
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
        .route("/admin/site/{site_id}/render", get(admin_site_render))
        .layer(from_fn(require_session));

    info!(
        "admin server listening on http://{listen} / {}",
        state.oidc_frontend_url
    );

    let assets_dir = resolve_admin_assets_dir();
    let upload_root = resolve_upload_root();
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
        .nest_service("/admin/assets", ServeDir::new(assets_dir))
        .nest_service("/media/images", ServeDir::new(upload_root))
        .merge(protected_routes)
        .layer(session_layer)
        .layer(from_fn(log_requests))
        .layer(from_fn(crate::middleware::dont_cache_me))
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
    let user = upsert_user_login(db, user_sub, user_email)
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

    // TODO make sure we get the user's email from the OIDC claims
    if let Some(membership) = ensure_site_owner_membership(&txn, &user_sub, None, site.id).await? {
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
    let membership_rows = memberships
        .into_iter()
        .map(|membership| {
            let (subject, email) = user_map
                .get(&membership.user_id)
                .cloned()
                .unwrap_or_else(|| ("unknown".to_string(), None));
            AdminMembershipRow {
                subject,
                email: email.unwrap_or_else(|| "unknown@unknown.com".to_string()),
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
            target.email.as_ref().unwrap_or(&target.subject)
        )),

        user_id: target.id,
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
    let subject = form.subject.trim();
    if subject.is_empty() {
        return Err(SiteError::internal("missing subject".to_string()));
    }
    let actor = current_user(&session).await?.subject;
    let txn = state.db.begin().await?;
    let actor = actor.clone();
    let subject = subject.to_string();
    // TODO the user should already exist here
    let user = upsert_user_login(&txn, &subject, None)
        .await
        .map_err(|error| SiteError::internal(format!("failed to upsert user: {error}")))?;
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
        .map(|tag| AdminTagOption { name: tag.name })
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
    let tag_names = form.tags.clone();
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
    let content_id = content.id.to_string();

    let published_at = content
        .published_at
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| "n/a".to_string());
    let slug = content.slug.clone();
    let title = content.title.clone();
    let rows = vec![
        AdminRow {
            label: "id".to_string(),
            value: content_id,
        },
        AdminRow {
            label: "site_id".to_string(),
            value: content.site_id.to_string(),
        },
        AdminRow {
            label: "title".to_string(),
            value: title,
        },
        AdminRow {
            label: "slug".to_string(),
            value: slug,
        },
        AdminRow {
            label: "page_type".to_string(),
            value: content.page_type.to_string(),
        },
        AdminRow {
            label: "draft".to_string(),
            value: content.draft.to_string(),
        },
        AdminRow {
            label: "published_at".to_string(),
            value: published_at,
        },
    ];

    let route = content_primary_route(&content);
    Ok(AdminContentDetailTemplate {
        template_shared: AdminTemplateData::new(format!("Content: /{route}"))
            .with_message(format!("Creator: {}", content.creator_sub))
            .with_site_id(content.site_id)
            .with_links(vec![
                AdminLink::new(
                    &format!(
                        "/admin/site/{}/content/{}/source",
                        content.site_id, content.id
                    ),
                    "Return to editor",
                ),
                AdminLink::new(
                    &format!(
                        "/admin/site/{}/content/{}/advanced",
                        content.site_id, content.id
                    ),
                    "Advanced",
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

        rows,
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
    let page_type = content.page_type.to_string();
    let draft = content.draft;
    let published_at = content.content_publish_timestamp();
    let title = content.title;
    let slug = content.slug;
    let page_content = content.page_content;

    let template_shared = AdminTemplateData::new(format!("Source: {}", title))
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

        title,
        slug,
        page_type,
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
        "/admin/site/{}/content/{}/source?saved=1",
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
) -> Result<AdminContentAdvancedTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let content = entities::content_item::Entity::find_by_id(content_id)
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .one(state.db.as_ref())
        .await
        .map_err(|err| SiteError::internal(format!("failed to load content {content_id}: {err}")))?
        .ok_or(SiteError::NotFound)?;

    let aliases = list_aliases(state.db.as_ref(), content.site_id, Some(content.id))
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load aliases for content {content_id}: {error}"
            ))
        })?;
    let tags = list_content_tags(state.db.as_ref(), content.id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load tags for content {content_id}: {error}"
            ))
        })?;

    Ok(AdminContentAdvancedTemplate {
        template_shared: AdminTemplateData::new("Content Advanced View").with_links(vec![
            AdminLink::new(
                &format!("/admin/site/{}/content/{}", content.site_id, content.id),
                "Back to site dashboard",
            ),
        ]),

        rows: vec![
            AdminRow {
                label: "alias_count".to_string(),
                value: aliases.len().to_string(),
            },
            AdminRow {
                label: "tag_count".to_string(),
                value: tags.len().to_string(),
            },
            AdminRow {
                label: "created_at".to_string(),
                value: content.created_at.to_rfc3339(),
            },
            AdminRow {
                label: "updated_at".to_string(),
                value: content
                    .last_updated
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_else(|| "n/a".to_string()),
            },
        ],
    })
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
            let rows = tags
                .into_iter()
                .map(|tag| AdminRow {
                    label: tag.name,
                    value: tag.id.to_string(),
                })
                .collect();

            Ok(AdminTagsTemplate {
                template_shared: AdminTemplateData::new(format!("Site Tags ({site_id})"))
                    .with_links(vec![AdminLink::new(
                        &format!("/admin/site/{site_id}/content"),
                        "Back to site dashboard",
                    )]),

                rows,
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load tags for site {site_id}: {error}"
        ))),
    }
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
    let rows = assets
        .into_iter()
        .map(|asset| AdminRow {
            label: asset.original_filename,
            value: format!(
                "{} {} [{} bytes]",
                asset.storage_basename, asset.mime_type, asset.byte_length
            ),
        })
        .collect();

    Ok(AdminAssetsTemplate {
        template_shared: AdminTemplateData::new(format!("Site Assets ({})", site.short_name))
            .with_links(vec![
                AdminLink::new(&format!("/admin/site/{site_id}/assets/new"), "Upload asset"),
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
            ]),
        rows,
    })
}

async fn admin_site_assets_new(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminAssetsNewTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    match get_by_id(state.db.as_ref(), site_id).await {
        Ok(site) => Ok(AdminAssetsNewTemplate {
            template_shared: AdminTemplateData::new(format!("Upload Asset {}", site.short_name))
                .with_links(vec![
                    AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Back to assets"),
                    AdminLink::new(
                        &format!("/admin/site/{site_id}/content"),
                        "Back to site dashboard",
                    ),
                ]),

            rows: vec![AdminRow {
                label: "site_id".to_string(),
                value: site.id.to_string(),
            }],
        }),
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
        if field.name() != Some("file") {
            continue;
        }

        original_filename = field.file_name().map(|value| value.to_string());
        mime_type = field.content_type().map(|value| value.to_string());
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to read upload: {error}"
                )));
            }
        };
        upload_bytes = Some(bytes.to_vec());
    }

    let Some(bytes) = upload_bytes else {
        return Err(SiteError::internal("missing file upload".to_string()));
    };
    let Some(original_filename) = original_filename else {
        return Err(SiteError::internal("missing original filename".to_string()));
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

async fn admin_site_settings(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminSiteSettingsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;

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
        let viewer = crate::entities::user::create_user(db.as_ref(), "viewer", None, false)
            .await
            .expect("failed to create viewer");
        let target = crate::entities::user::create_user(db.as_ref(), "target", None, false)
            .await
            .expect("failed to create target");
        let admin = crate::entities::user::create_user(db.as_ref(), "admin", None, true)
            .await
            .expect("failed to create admin");

        assert!(can_view_user_profile(&viewer, &viewer));
        assert!(!can_view_user_profile(&viewer, &target));
        assert!(can_view_user_profile(&admin, &target));
    }
}
