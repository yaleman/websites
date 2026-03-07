use crate::entities::PageType;
use crate::entities::audit_event::log_audit_event;
use crate::errors::SiteError;
use crate::middleware::log_requests;
use crate::tls::build_tls_config;
use crate::{
    NewAsset, NewAssetVariant, NewContent, cli::OidcConfig, content_primary_route, create_asset,
    create_asset_variant, create_content, create_membership, create_site, delete_membership,
    get_content, get_membership_by_id, get_revision, get_revision_by_number, get_site,
    list_aliases, list_asset_variants, list_assets, list_content, list_content_tags,
    list_memberships, list_revisions, list_sites, list_tags, list_users_by_ids, render_site,
    search_content, update_content, update_membership_role,
};
use anyhow::Context;
use askama::Template;
use askama_web::WebTemplate;
use axum::middleware::{Next, from_fn};
use axum::{
    Router,
    extract::{Form, Multipart, Path, Query, Request, State},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use chrono::{DateTime, Utc};
use image::{GenericImageView, ImageFormat};
use openidconnect::{
    AuthorizationCode, ClientId, CsrfToken, EndpointMaybeSet, EndpointNotSet, EndpointSet,
    IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
};
use reqwest::redirect::Policy;
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};
use serde::Deserialize;
use serde_json::json;
use similar::TextDiff;
use std::collections::HashMap;
use std::env;
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::{Path as StdPath, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tower_http::services::ServeDir;
use tower_sessions::{Expiry, Session, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;
use url::Url;
use uuid::Uuid;

#[derive(Clone)]
struct AdminState {
    db: Arc<DatabaseConnection>,
    oidc_client_id: Option<String>,
    oidc_frontend_url: Url,
    oidc_discovery_url: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "admin/page.html")]
struct AdminPageTemplate {
    title: String,
    heading: String,
    message: String,
    rows: Vec<AdminRow>,
    links: Vec<AdminLink>,
    inline_body: String,
    pre_body: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "admin/sites.html")]
struct AdminSitesTemplate {
    title: String,
    heading: String,
    message: String,
    sites: Vec<AdminSiteRow>,
    links: Vec<AdminLink>,
}

#[derive(Template, WebTemplate)]
#[template(path = "admin/content_new.html")]
struct AdminContentNewTemplate {
    title: String,
    site_id: String,
    site_short_name: String,
    message: String,
    content_href: String,
    settings_href: String,
    tags: Vec<AdminTagOption>,
}

#[derive(Template, WebTemplate)]
#[template(path = "admin/memberships.html")]
struct AdminMembershipsTemplate {
    title: String,
    site_id: String,
    site_short_name: String,
    message: String,
    content_href: String,
    settings_href: String,
    create_href: String,
    memberships: Vec<AdminMembershipRow>,
}

#[derive(Debug)]
struct AdminRow {
    label: String,
    value: String,
}

#[derive(Debug)]
struct AdminSiteRow {
    short_name: String,
    full_title: String,
    content_href: String,
}

#[derive(Debug)]
struct AdminTagOption {
    name: String,
}

#[derive(Debug)]
struct AdminMembershipRow {
    subject: String,
    role: String,
    update_href: String,
    remove_href: String,
}

#[derive(Debug)]
struct AdminLink {
    href: String,
    label: String,
}

#[derive(Debug, Deserialize)]
struct CreateSiteForm {
    short_name: String,
    full_title: String,
    template_name: Option<String>,
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

#[derive(Debug, Deserialize)]
struct MembershipCreateForm {
    subject: String,
    role: String,
}

#[derive(Debug, Deserialize)]
struct MembershipUpdateForm {
    role: String,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

type OidcClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

const ADMIN_ACTOR_SUB: &str = "web-admin";
const DEFAULT_TEMPLATE_NAME: &str = "default";
const UPLOAD_ROOT: &str = "./uploads/media-storage";
const THUMBNAIL_MAX_SIZE: u32 = 320;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum SiteRole {
    Viewer,
    Author,
    Editor,
    Owner,
}

impl SiteRole {
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "viewer" => Some(Self::Viewer),
            "author" => Some(Self::Author),
            "editor" => Some(Self::Editor),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Author => "author",
            Self::Editor => "editor",
            Self::Owner => "owner",
        }
    }
}

async fn current_user_sub(session: &Session) -> Result<String, SiteError> {
    session
        .get::<String>("user_sub")
        .await
        .map_err(|_| SiteError::internal("failed to read session".to_string()))?
        .ok_or_else(|| SiteError::UnAuthorized("missing user session".to_string()))
}

async fn require_site_role(
    _state: &AdminState,
    session: &Session,
    _site_id: Uuid,
    _required: SiteRole,
) -> Result<(), SiteError> {
    let _ = current_user_sub(session).await?;
    Ok(())
}

pub async fn run_admin_server(
    db: Arc<DatabaseConnection>,
    listen: &str,
    oidc: &OidcConfig,
) -> Result<(), anyhow::Error> {
    let state = AdminState {
        db: db.clone(),
        oidc_client_id: oidc.oidc_client_id.clone(),
        oidc_frontend_url: oidc.frontend_url.clone(),
        oidc_discovery_url: oidc.oidc_discovery_url.clone(),
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
            "/admin/site/{site_id}/assets/new",
            get(admin_site_assets_new).post(admin_site_assets_create),
        )
        .route("/admin/site/{site_id}/settings", get(admin_site_settings))
        .route("/admin/site/{site_id}/render", get(admin_site_render))
        .layer(from_fn(require_admin_session));

    tracing::info!(
        "admin server listening on http://{listen} / {}",
        state.oidc_frontend_url
    );

    let assets_dir = resolve_admin_assets_dir();
    tracing::info!("admin assets dir: {}", assets_dir.display());
    if !assets_dir.join("editor.js").exists() {
        tracing::warn!(
            "admin editor assets not found; run `pnpm run build:admin` to generate them"
        );
    }

    let app = Router::new()
        .route("/", get(admin_root))
        .route("/admin/login", get(admin_login))
        .route("/oauth2/callback", get(admin_login_callback))
        .route("/admin/logout", get(admin_logout))
        .nest_service("/admin/assets", ServeDir::new(assets_dir))
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

async fn admin_root() -> Redirect {
    Redirect::to("/admin")
}

async fn require_admin_session(session: Session, request: Request, next: Next) -> Response {
    let is_authenticated = session
        .get::<String>("user_sub")
        .await
        .unwrap_or(None)
        .is_some();
    if is_authenticated {
        next.run(request).await
    } else {
        Redirect::to("/admin/login").into_response()
    }
}

async fn admin_index(State(_state): State<AdminState>) -> AdminPageTemplate {
    let links = vec![
        link("/admin/sites", "Sites"),
        link("/admin/login", "Login"),
        link("/admin/logout", "Logout"),
    ];

    AdminPageTemplate {
        title: "Admin".to_string(),
        heading: "Administration".to_string(),
        message: "Use the route set below to browse admin surfaces.".to_string(),
        rows: vec![
            AdminRow {
                label: "Dashboard".to_string(),
                value: "/admin".to_string(),
            },
            AdminRow {
                label: "Sites list".to_string(),
                value: "/admin/sites".to_string(),
            },
        ],
        links,
        inline_body: String::new(),
        pre_body: String::new(),
    }
}

async fn admin_sites(State(state): State<AdminState>) -> Result<AdminSitesTemplate, SiteError> {
    match list_sites(state.db.as_ref()).await {
        Ok(sites) => {
            let rows = sites
                .into_iter()
                .map(|site| AdminSiteRow {
                    short_name: site.short_name,
                    full_title: site.full_title,
                    content_href: format!("/admin/site/{}/content", site.id),
                })
                .collect();

            Ok(AdminSitesTemplate {
                title: "Sites".to_string(),
                heading: "Managed Sites".to_string(),
                message: "Manage sites and browse site zones from here.".to_string(),
                links: vec![link("/admin/sites/new", "New site")],
                sites: rows,
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load sites: {error}"
        ))),
    }
}

async fn admin_sites_new() -> Response {
    AdminPageTemplate {
        title: "New Site".to_string(),
        heading: "Create Site".to_string(),
        message: "Use this form to create a site.".to_string(),
        rows: vec![],
        links: vec![link("/admin/sites", "Back to sites")],
        inline_body: admin_sites_new_form_html().to_string(),
        pre_body: String::new(),
    }
    .into_response()
}

async fn ensure_site_owner_membership<C: ConnectionTrait>(
    db: &C,
    user_sub: &str,
    site_id: Uuid,
) -> Result<Option<crate::entities::site_membership::Model>, SiteError> {
    let user = crate::upsert_user_login(db, user_sub)
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
            role: "owner".to_string(),
        },
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to create membership: {error}")))?;

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
    let user_sub = current_user_sub(&session).await?;
    let short_name = form.short_name;
    let full_title = form.full_title;

    state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            let user_sub = user_sub.clone();
            let template_name = template_name.clone();
            Box::pin(async move {
                let site = create_site(txn, short_name, full_title, template_name)
                    .await
                    .map_err(|error| {
                        SiteError::internal(format!("failed to create site: {error}"))
                    })?;
                log_audit_event(
                    txn,
                    ADMIN_ACTOR_SUB,
                    "create_site",
                    "site",
                    &site.id.to_string(),
                    Some(site.id),
                    Some(json!({ "short_name": &site.short_name, "full_title": &site.full_title })),
                )
                .await
                .map_err(|error| {
                    SiteError::internal(format!("failed to log site audit: {error}"))
                })?;

                if let Some(membership) =
                    ensure_site_owner_membership(txn, &user_sub, site.id).await?
                {
                    log_audit_event(
                        txn,
                        ADMIN_ACTOR_SUB,
                        "create_membership",
                        "site_membership",
                        &membership.id.to_string(),
                        Some(membership.site_id),
                        Some(json!({
                            "site_id": membership.site_id.to_string(),
                            "user_id": membership.user_id.to_string(),
                            "role": membership.role
                        })),
                    )
                    .await
                    .map_err(|error| {
                        SiteError::internal(format!("failed to log membership audit: {error}"))
                    })?;
                }

                Ok(site)
            })
        })
        .await
        .map_err(|error| SiteError::internal(format!("failed to create site: {error}")))?;

    Ok(Redirect::to("/admin/sites"))
}

fn admin_sites_new_form_html() -> &'static str {
    r#"
      <form method="post" action="/admin/sites/new">
        <label for="short_name">Short Name</label>
        <input id="short_name" name="short_name" required />

        <label for="full_title">Full Title</label>
        <input id="full_title" name="full_title" required />

        <label for="template_name">Template Name</label>
        <input id="template_name" name="template_name" value="default" />

        <button type="submit">Create site</button>
      </form>
    "#
}

async fn admin_login(
    State(state): State<AdminState>,
    session: Session,
) -> Result<Response, SiteError> {
    let http_client = match build_http_client() {
        Ok(client) => client,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to build http client: {error}"
            )));
        }
    };
    let client = match build_oidc_client(&state, &http_client).await {
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
        .insert("oidc_state", csrf_state.secret().to_string())
        .await
        .is_err()
        || session
            .insert("oidc_pkce", pkce_verifier.secret().to_string())
            .await
            .is_err()
        || session
            .insert("oidc_nonce", nonce.secret().to_string())
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

async fn admin_login_callback(
    State(state): State<AdminState>,
    Query(query): Query<OidcCallbackQuery>,
    session: Session,
) -> Result<Redirect, SiteError> {
    if let Some(error) = query.error {
        let description = query.error_description.unwrap_or_default();
        return Err(SiteError::internal(format!(
            "OIDC error: {error} {description}"
        )));
    }

    let code = match query.code {
        Some(code) => code,
        None => {
            return Err(SiteError::UnAuthorized(
                "missing authorization code".to_string(),
            ));
        }
    };
    let state_value = match query.state {
        Some(state) => state,
        None => return Err(SiteError::UnAuthorized("missing state".to_string())),
    };

    let stored_state = session
        .get::<String>("oidc_state")
        .await
        .unwrap_or(None)
        .unwrap_or_default();
    if stored_state != state_value {
        return Err(SiteError::UnAuthorized("OIDC state mismatch".to_string()));
    }

    let pkce_verifier = session
        .get::<String>("oidc_pkce")
        .await
        .unwrap_or(None)
        .unwrap_or_default();
    let nonce_value = session
        .get::<String>("oidc_nonce")
        .await
        .unwrap_or(None)
        .unwrap_or_default();

    let http_client = match build_http_client() {
        Ok(client) => client,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to build http client: {error}"
            )));
        }
    };
    let client = match build_oidc_client(&state, &http_client).await {
        Ok(client) => client,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to initialize OIDC client: {error}"
            )));
        }
    };

    let token_request = match client.exchange_code(AuthorizationCode::new(code)) {
        Ok(request) => request,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to build token request: {error}"
            )));
        }
    };
    let token_response = match token_request
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
        .request_async(&http_client)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to exchange code: {error}"
            )));
        }
    };

    let id_token = match token_response.id_token() {
        Some(token) => token,
        None => {
            return Err(SiteError::internal(
                "missing id_token in response".to_string(),
            ));
        }
    };

    let nonce = Nonce::new(nonce_value);
    let claims = match id_token.claims(&client.id_token_verifier(), &nonce) {
        Ok(claims) => claims,
        Err(error) => {
            return Err(SiteError::UnAuthorized(format!(
                "failed to verify id_token: {error}"
            )));
        }
    };

    let subject = claims.subject().as_str().to_string();
    if session.insert("user_sub", subject.clone()).await.is_err() {
        return Err(SiteError::internal("failed to store session".to_string()));
    }

    let _ = crate::upsert_user_login(state.db.as_ref(), &subject).await;

    Ok(Redirect::to("/admin"))
}

async fn admin_logout(session: Session) -> Redirect {
    let _ = session.clear().await;
    Redirect::to("/admin/login")
}

fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::ClientBuilder::new()
        .redirect(Policy::none())
        .build()
        .map_err(|error| error.to_string())
}

async fn build_oidc_client(
    state: &AdminState,
    http_client: &reqwest::Client,
) -> Result<OidcClient, String> {
    let discovery_url = state
        .oidc_discovery_url
        .clone()
        .ok_or_else(|| "missing discovery url".to_string())?;
    let client_id = state
        .oidc_client_id
        .clone()
        .ok_or_else(|| "missing client id".to_string())?;
    let frontend_url = state.oidc_frontend_url.clone();

    let provider_metadata = CoreProviderMetadata::discover_async(
        IssuerUrl::new(discovery_url).map_err(|error| error.to_string())?,
        http_client,
    )
    .await
    .map_err(|error| error.to_string())?;

    let redirect_url = frontend_url
        .join("/oauth2/callback")
        .map_err(|error| error.to_string())?;
    let client =
        CoreClient::from_provider_metadata(provider_metadata, ClientId::new(client_id), None)
            .set_redirect_uri(RedirectUrl::from_url(redirect_url));

    Ok(client)
}

async fn admin_site_content_list(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Viewer).await?;
    match list_content(state.db.as_ref(), site_id_uuid, None).await {
        Ok(content) => {
            let rows = content
                .into_iter()
                .map(|item| AdminRow {
                    label: item.title,
                    value: format!(
                        "{0} (type: {1}) [detail: /admin/site/{2}/content/{3}]",
                        item.id, item.page_type, item.site_id, item.id
                    ),
                })
                .collect();

            Ok(AdminPageTemplate {
                title: "Content".to_string(),
                heading: format!("Site Content {site_id}"),
                message: "Browse linked content routes for each item.".to_string(),
                rows,
                links: vec![
                    link(&format!("/admin/site/{site_id}/settings"), "Site settings"),
                    link(&format!("/admin/site/{site_id}/memberships"), "Memberships"),
                    link(&format!("/admin/site/{site_id}/tags"), "Tags"),
                    link(&format!("/admin/site/{site_id}/assets"), "Assets"),
                    link(&format!("/admin/site/{site_id}/render"), "Render"),
                    link(&format!("/admin/site/{site_id}/content/new"), "New content"),
                    link(&format!("/admin/site/{site_id}/search"), "Search content"),
                ],
                inline_body: String::new(),
                pre_body: String::new(),
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
    Path(site_id): Path<String>,
) -> Result<AdminMembershipsTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Owner).await?;
    let site = get_site(state.db.as_ref(), site_id_uuid)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let memberships = list_memberships(state.db.as_ref(), site_id_uuid)
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
        .map(|user| (user.id, user.subject))
        .collect::<HashMap<_, _>>();
    let membership_rows = memberships
        .into_iter()
        .map(|membership| {
            let subject = user_map
                .get(&membership.user_id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            AdminMembershipRow {
                subject,
                role: membership.role,
                update_href: format!("/admin/site/{site_id}/memberships/{}/update", membership.id),
                remove_href: format!("/admin/site/{site_id}/memberships/{}/remove", membership.id),
            }
        })
        .collect();

    Ok(AdminMembershipsTemplate {
        title: "Memberships".to_string(),
        site_id: site.id.to_string(),
        site_short_name: site.short_name,
        message: "Manage site membership roles.".to_string(),
        content_href: format!("/admin/site/{site_id}/content"),
        settings_href: format!("/admin/site/{site_id}/settings"),
        create_href: format!("/admin/site/{site_id}/memberships/new"),
        memberships: membership_rows,
    })
}

async fn admin_site_membership_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
    Form(form): Form<MembershipCreateForm>,
) -> Result<Redirect, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Owner).await?;
    let role = SiteRole::from_str(form.role.as_str())
        .ok_or_else(|| SiteError::internal("invalid role".to_string()))?;
    let subject = form.subject.trim();
    if subject.is_empty() {
        return Err(SiteError::internal("missing subject".to_string()));
    }
    let actor = current_user_sub(&session).await?;
    state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            let actor = actor.clone();
            let subject = subject.to_string();
            Box::pin(async move {
                let user = crate::upsert_user_login(txn, &subject)
                    .await
                    .map_err(|error| {
                        SiteError::internal(format!("failed to upsert user: {error}"))
                    })?;
                let membership = create_membership(
                    txn,
                    crate::NewMembership {
                        site_id: site_id_uuid,
                        user_id: user.id,
                        role: role.label().to_string(),
                    },
                )
                .await
                .map_err(|error| {
                    SiteError::internal(format!("failed to create membership: {error}"))
                })?;
                log_audit_event(
                    txn,
                    &actor,
                    "create_membership",
                    "site_membership",
                    &membership.id.to_string(),
                    Some(membership.site_id),
                    Some(json!({
                        "user_id": membership.user_id.to_string(),
                        "role": membership.role
                    })),
                )
                .await
                .map_err(|error| {
                    SiteError::internal(format!("failed to log membership audit: {error}"))
                })?;
                Ok(())
            })
        })
        .await
        .map_err(|error| SiteError::internal(error.to_string()))?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/memberships")))
}

async fn admin_site_membership_update(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, membership_id)): Path<(String, String)>,
    Form(form): Form<MembershipUpdateForm>,
) -> Result<Redirect, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Owner).await?;
    let membership_id_uuid = parse_uuid_param(&membership_id, "membership_id")?;
    let membership = get_membership_by_id(state.db.as_ref(), membership_id_uuid)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;
    let membership =
        membership.ok_or_else(|| SiteError::internal("membership not found".to_string()))?;
    if membership.site_id != site_id_uuid {
        return Err(SiteError::UnAuthorized(
            "membership does not belong to site".to_string(),
        ));
    }
    let role = SiteRole::from_str(form.role.as_str())
        .ok_or_else(|| SiteError::internal("invalid role".to_string()))?;
    let actor = current_user_sub(&session).await?;
    state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            let actor = actor.clone();
            let role = role.label().to_string();
            Box::pin(async move {
                let updated = update_membership_role(txn, membership.id, role)
                    .await
                    .map_err(|error| {
                        SiteError::internal(format!("failed to update membership: {error}"))
                    })?;
                log_audit_event(
                    txn,
                    &actor,
                    "update_membership",
                    "site_membership",
                    &updated.id.to_string(),
                    Some(updated.site_id),
                    Some(json!({
                        "site_id": updated.site_id.to_string(),
                        "user_id": updated.user_id.to_string(),
                        "role": updated.role
                    })),
                )
                .await
                .map_err(|error| {
                    SiteError::internal(format!("failed to log membership audit: {error}"))
                })?;
                Ok(())
            })
        })
        .await
        .map_err(|error| SiteError::internal(error.to_string()))?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/memberships")))
}

async fn admin_site_membership_remove(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, membership_id)): Path<(String, String)>,
) -> Result<Redirect, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Owner).await?;
    let membership_id_uuid = parse_uuid_param(&membership_id, "membership_id")?;
    let membership = get_membership_by_id(state.db.as_ref(), membership_id_uuid)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;
    let membership =
        membership.ok_or_else(|| SiteError::internal("membership not found".to_string()))?;
    if membership.site_id != site_id_uuid {
        return Err(SiteError::UnAuthorized(
            "membership does not belong to site".to_string(),
        ));
    }
    let actor = current_user_sub(&session).await?;
    state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            let actor = actor.clone();
            Box::pin(async move {
                delete_membership(txn, membership.id)
                    .await
                    .map_err(|error| {
                        SiteError::internal(format!("failed to remove membership: {error}"))
                    })?;
                log_audit_event(
                    txn,
                    &actor,
                    "delete_membership",
                    "site_membership",
                    &membership.id.to_string(),
                    Some(membership.site_id),
                    Some(json!({
                        "site_id": membership.site_id.to_string(),
                        "user_id": membership.user_id.to_string(),
                        "role": membership.role
                    })),
                )
                .await
                .map_err(|error| {
                    SiteError::internal(format!("failed to log membership audit: {error}"))
                })?;
                Ok(())
            })
        })
        .await
        .map_err(|error| SiteError::internal(error.to_string()))?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/memberships")))
}

async fn admin_site_search(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Viewer).await?;
    let query_text = query.q.unwrap_or_default();
    let mut rows = Vec::new();
    let mut message = "Search content by title, slug, or body text.".to_string();

    if !query_text.trim().is_empty() {
        match search_content(state.db.as_ref(), site_id_uuid, query_text.trim()).await {
            Ok(items) => {
                message = format!("Found {} result(s) for \"{}\".", items.len(), query_text);
                rows = items
                    .into_iter()
                    .map(|item| AdminRow {
                        label: item.title,
                        value: format!(
                            "{} ({}) [detail: /admin/site/{}/content/{}]",
                            item.id, item.page_type, item.site_id, item.id
                        ),
                    })
                    .collect();
            }
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to search content: {error}"
                )));
            }
        }
    }

    Ok(AdminPageTemplate {
        title: "Search".to_string(),
        heading: format!("Search Content {site_id}"),
        message,
        rows,
        links: vec![
            link(&format!("/admin/site/{site_id}/content"), "Back to content"),
            link(&format!("/admin/site/{site_id}/content/new"), "New content"),
        ],
        inline_body: admin_site_search_form_html(&query_text),
        pre_body: String::new(),
    })
}

fn admin_site_search_form_html(query: &str) -> String {
    let query = escape_html(query);
    format!(
        r#"
      <form method="get" action="">
        <label for="q">Search Query</label>
        <input id="q" name="q" value="{query}" />

        <button type="submit">Search</button>
      </form>
    "#
    )
}

async fn admin_site_content_new(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
) -> Result<AdminContentNewTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Author).await?;
    let tags = list_tags(state.db.as_ref(), site_id_uuid)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load tags: {error}")))?;
    let tags = tags
        .into_iter()
        .map(|tag| AdminTagOption { name: tag.name })
        .collect();

    match get_site(state.db.as_ref(), site_id_uuid).await {
        Ok(site) => Ok(AdminContentNewTemplate {
            title: "New Content".to_string(),
            site_id: site.id.to_string(),
            site_short_name: site.short_name,
            message: "Create a page or post and start drafting immediately.".to_string(),
            content_href: format!("/admin/site/{site_id}/content"),
            settings_href: format!("/admin/site/{site_id}/settings"),
            tags,
        }),
        Err(error) => Err(SiteError::internal(format!(
            "failed to load site {site_id}: {error}"
        ))),
    }
}

async fn admin_site_content_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
    Form(form): Form<CreateContentForm>,
) -> Result<Redirect, SiteError> {
    let page_type = PageType::from_str(&form.page_type)
        .map_err(|error| SiteError::internal(error.to_string()))?;
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Author).await?;
    let tag_names = form.tags.clone();
    let title = form.title;
    let slug = form.slug;
    let page_content = form.page_content;
    let draft = form.draft.unwrap_or(false);

    let site = get_site(state.db.as_ref(), site_id_uuid)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let site_id_uuid = site.id;

    let content = state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            let tag_names = tag_names.clone();
            Box::pin(async move {
                let content = create_content(
                    txn,
                    NewContent {
                        site_id: site_id_uuid,
                        page_type,
                        title,
                        slug,
                        page_content,
                        draft,
                        creator_sub: ADMIN_ACTOR_SUB.to_string(),
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
                    let revision = get_revision_by_number(txn, content.id, 1)
                        .await
                        .map_err(|error| {
                            SiteError::internal(format!(
                                "failed to load revision for tags: {error}"
                            ))
                        })?
                        .ok_or_else(|| {
                            SiteError::internal("missing revision for new content".to_string())
                        })?;
                    crate::assign_tags_to_content(
                        txn,
                        content.site_id,
                        content.id,
                        revision.id,
                        tag_names,
                    )
                    .await
                    .map_err(|error| {
                        SiteError::internal(format!("failed to assign tags: {error}"))
                    })?;
                }

                log_audit_event(
                    txn,
                    ADMIN_ACTOR_SUB,
                    "create_content",
                    "content_item",
                    &content.id.to_string(),
                    Some(content.site_id),
                    Some(json!({
                        "page_type": content.page_type.to_string(),
                        "slug": &content.slug,
                        "title": &content.title,
                        "draft": content.draft
                    })),
                )
                .await
                .map_err(|error| {
                    SiteError::internal(format!("failed to log content audit: {error}"))
                })?;

                Ok(content)
            })
        })
        .await
        .map_err(|error| SiteError::internal(error.to_string()))?;

    Ok(Redirect::to(&format!(
        "/admin/site/{}/content/{}",
        content.site_id, content.id
    )))
}

async fn admin_site_content_detail(
    State(state): State<AdminState>,
    session: Session,
    Path((_site_id, content_id)): Path<(String, String)>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&_site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Viewer).await?;
    let content_id_uuid = parse_uuid_param(&content_id, "content_id")?;
    let content = get_content(state.db.as_ref(), content_id_uuid)
        .await
        .map_err(|err| {
            SiteError::internal(format!("failed to load content {content_id}: {err}"))
        })?;
    let content_id = content.id.to_string();

    let published_at = content
        .published_at
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| "n/a".to_string());
    let page_type = content.page_type.to_string();
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
            value: page_type,
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
    Ok(AdminPageTemplate {
        title: "Content Detail".to_string(),
        heading: format!("Content: /{route}"),
        message: format!("Creator: {}", content.creator_sub),
        rows,
        links: vec![
            link(
                &format!(
                    "/admin/site/{}/content/{}/source",
                    content.site_id, content.id
                ),
                "Source",
            ),
            link(
                &format!(
                    "/admin/site/{}/content/{}/advanced",
                    content.site_id, content.id
                ),
                "Advanced",
            ),
            link(
                &format!(
                    "/admin/site/{}/content/{}/revisions",
                    content.site_id, content.id
                ),
                "Revisions",
            ),
            link(
                &format!("/admin/site/{}/content", content.site_id),
                "Back to content",
            ),
        ],
        inline_body: String::new(),
        pre_body: String::new(),
    })
}

#[axum::debug_handler]
async fn admin_site_content_source(
    State(state): State<AdminState>,
    session: Session,
    Path((_site_id, content_id)): Path<(String, String)>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&_site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Author).await?;
    let content_id_uuid = parse_uuid_param(&content_id, "content_id")?;
    let content = get_content(state.db.as_ref(), content_id_uuid)
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to load source for {content_id}: {error}"))
        })?;
    let assets_html = render_asset_embed_library(state.db.as_ref(), content.site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load assets: {error}")))?;

    Ok(AdminPageTemplate {
        title: "Content Source".to_string(),
        heading: format!("Source: {}", content.title),
        message: "Edit raw markdown and metadata, then save to create a revision.".to_string(),
        rows: vec![],
        links: vec![link(
            &format!("/admin/site/{}/content/{}", content.site_id, content.id),
            "Back to content",
        )],
        inline_body: format!(
            "{}{}",
            admin_site_content_source_form_html(&content),
            assets_html
        ),
        pre_body: String::new(),
    })
}

async fn admin_site_content_source_update(
    State(state): State<AdminState>,
    session: Session,
    Path((_site_id, content_id)): Path<(String, String)>,
    Form(form): Form<UpdateContentForm>,
) -> Result<Redirect, SiteError> {
    let site_id_uuid = parse_uuid_param(&_site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Author).await?;
    let draft = matches!(form.draft.as_str(), "true" | "1" | "yes");
    let published_at =
        parse_optional_datetime(normalize_optional(form.published_at), "published_at")?;
    let content_id_uuid = parse_uuid_param(&content_id, "content_id")?;
    let page_type = PageType::from_str(&form.page_type)
        .map_err(|error| SiteError::internal(error.to_string()))?;
    let title = form.title;
    let slug = form.slug;
    let page_content = form.page_content;

    let content = state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            Box::pin(async move {
                let content = update_content(
                    txn,
                    crate::UpdateContent {
                        content_id: content_id_uuid,
                        page_type: Some(page_type),
                        title: Some(title),
                        slug: Some(slug),
                        page_content: Some(page_content),
                        draft: Some(draft),
                        published_at,
                        editor_sub: ADMIN_ACTOR_SUB.to_string(),
                    },
                )
                .await
                .map_err(|error| {
                    SiteError::internal(format!("failed to update content {content_id}: {error}"))
                })?;

                log_audit_event(
                    txn,
                    ADMIN_ACTOR_SUB,
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
                .map_err(|error| {
                    SiteError::internal(format!("failed to log update audit: {error}"))
                })?;

                Ok(content)
            })
        })
        .await
        .map_err(|error| SiteError::internal(error.to_string()))?;

    Ok(Redirect::to(&format!(
        "/admin/site/{}/content/{}",
        content.site_id, content.id
    )))
}

fn admin_site_content_source_form_html(content: &crate::entities::content_item::Model) -> String {
    let title = escape_html(content.title.as_str());
    let slug = escape_html(content.slug.as_str());
    let content_body = escape_html(content.page_content.as_str());
    let published_at = escape_html(&content.content_publish_timestamp());

    let post_selected = if content.page_type.is_post() {
        "selected"
    } else {
        ""
    };
    let page_selected = if content.page_type.is_page() {
        "selected"
    } else {
        ""
    };
    let draft_selected = if content.draft { "selected" } else { "" };
    let published_selected = if content.draft { "" } else { "selected" };

    format!(
        r#"
      <form method="post" action="">
        <label for="page_type">Page Type</label>
        <select id="page_type" name="page_type" required>
          <option value="post" {post_selected}>Post</option>
          <option value="page" {page_selected}>Page</option>
        </select>

        <label for="title">Title</label>
        <input id="title" name="title" value="{title}" required />

        <label for="slug">Slug</label>
        <input id="slug" name="slug" value="{slug}" required />

        <label for="draft">Publish State</label>
        <select id="draft" name="draft">
          <option value="true" {draft_selected}>Draft</option>
          <option value="false" {published_selected}>Published</option>
        </select>

        <label for="published_at">Published At (RFC3339)</label>
        <input id="published_at" name="published_at" value="{published_at}" />

        <label for="page_content">Content</label>
        <div id="editor" class="editor-shell"></div>
        <textarea id="page_content" name="page_content" rows="14" class="editor-source">{content_body}</textarea>

        <button type="submit">Save content</button>
      </form>
    "#
    )
}

async fn render_asset_embed_library(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> Result<String, String> {
    let assets = list_assets(db, site_id).await?;
    if assets.is_empty() {
        return Ok(
            "<section class=\"embed-library\"><h2>Media Embeds</h2><p>No assets uploaded.</p></section>"
                .to_string(),
        );
    }

    let mut rows = String::new();
    for asset in assets {
        let variants = list_asset_variants(db, asset.id).await?;
        let mut variant_links = Vec::new();
        variant_links.push(format!(
            "<code>![{}](/media/images/{})</code>",
            escape_html(&asset.original_filename),
            escape_html(&asset.storage_basename)
        ));
        for variant in variants {
            if variant.filename == asset.storage_basename {
                continue;
            }
            variant_links.push(format!(
                "<code>![{}](/media/images/{})</code>",
                escape_html(&asset.original_filename),
                escape_html(&variant.filename)
            ));
        }

        rows.push_str(&format!(
            "<li><strong>{}</strong> ({})<div class=\"embed-snippets\">{}</div></li>",
            escape_html(&asset.original_filename),
            escape_html(&asset.mime_type),
            variant_links.join("<br/>")
        ));
    }

    Ok(format!(
        r#"
      <section class="embed-library">
        <h2>Media Embeds</h2>
        <p>Use these snippets to embed assets.</p>
        <ul>{}</ul>
      </section>
    "#,
        rows
    ))
}

async fn admin_site_content_advanced(
    State(state): State<AdminState>,
    session: Session,
    Path((_site_id, content_id)): Path<(String, String)>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&_site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Viewer).await?;
    let content_id_uuid = parse_uuid_param(&content_id, "content_id")?;
    let content = get_content(state.db.as_ref(), content_id_uuid)
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to load content {content_id}: {error}"))
        })?;

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

    Ok(AdminPageTemplate {
        title: "Content Advanced".to_string(),
        heading: "Advanced content details".to_string(),
        message: "Computed route and revision-aware detail fields available here.".to_string(),
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
        links: vec![link(
            &format!("/admin/site/{}/content/{}", content.site_id, content.id),
            "Back to content",
        )],
        inline_body: String::new(),
        pre_body: String::new(),
    })
}

async fn admin_site_content_revisions(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(String, String)>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Viewer).await?;
    let content_id_uuid = parse_uuid_param(&content_id, "content_id")?;
    let revisions = list_revisions(state.db.as_ref(), content_id_uuid)
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

    Ok(AdminPageTemplate {
        title: "Content Revisions".to_string(),
        heading: format!("Revisions for {content_id}"),
        message: "Latest revision is first in list order by revision number.".to_string(),
        rows,
        links: vec![link(
            &format!("/admin/site/{site_id}/content/{content_id}"),
            "Back to content",
        )],
        inline_body: if diff_links.is_empty() {
            "<p>No diffs available for the first revision.</p>".to_string()
        } else {
            format!(
                "<section class=\"revision-diffs\"><h2>Revision Diffs</h2><ul>{}</ul></section>",
                diff_links
            )
        },
        pre_body: String::new(),
    })
}

async fn admin_site_revision_diff(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id, revision_id)): Path<(String, String, String)>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Viewer).await?;
    let revision_id_uuid = parse_uuid_param(&revision_id, "revision_id")?;
    let content_id_uuid = parse_uuid_param(&content_id, "content_id")?;
    let revision = get_revision(state.db.as_ref(), revision_id_uuid)
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to load revision {revision_id}: {error}"))
        })?;
    if revision.content_id != content_id_uuid || revision.site_id != site_id_uuid {
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

    Ok(AdminPageTemplate {
        title: "Revision Diff".to_string(),
        heading: format!("Diff for rev-{}", revision.revision_number),
        message: format!(
            "Comparing revision {} for content {}.",
            revision.revision_number, revision.content_id
        ),
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
        links: vec![
            link(
                &format!("/admin/site/{}/content/{}/revisions", site_id, content_id),
                "Back to revisions",
            ),
            link(
                &format!("/admin/site/{}/content/{}", site_id, content_id),
                "Back to content",
            ),
        ],
        inline_body: String::new(),
        pre_body: diff_text,
    })
}

async fn admin_site_tags(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Viewer).await?;
    match list_tags(state.db.as_ref(), site_id_uuid).await {
        Ok(tags) => {
            let rows = tags
                .into_iter()
                .map(|tag| AdminRow {
                    label: tag.name,
                    value: tag.id.to_string(),
                })
                .collect();

            Ok(AdminPageTemplate {
                title: "Tags".to_string(),
                heading: format!("Site Tags ({site_id})"),
                message: "Tag definitions for this site.".to_string(),
                rows,
                links: vec![link(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to content",
                )],
                inline_body: String::new(),
                pre_body: String::new(),
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load tags for site {site_id}: {error}"
        ))),
    }
}

async fn admin_site_assets(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Viewer).await?;
    match list_assets(state.db.as_ref(), site_id_uuid).await {
        Ok(assets) => {
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

            Ok(AdminPageTemplate {
                title: "Assets".to_string(),
                heading: format!("Site Assets ({site_id})"),
                message: "Upload assets and review metadata and thumbnails.".to_string(),
                rows,
                links: vec![
                    link(&format!("/admin/site/{site_id}/assets/new"), "Upload asset"),
                    link(&format!("/admin/site/{site_id}/content"), "Back to content"),
                ],
                inline_body: String::new(),
                pre_body: String::new(),
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load assets for site {site_id}: {error}"
        ))),
    }
}

async fn admin_site_assets_new(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Author).await?;
    match get_site(state.db.as_ref(), site_id_uuid).await {
        Ok(site) => Ok(AdminPageTemplate {
            title: "Upload Asset".to_string(),
            heading: format!("Upload Asset {}", site.short_name),
            message: "Upload a media asset and generate a thumbnail.".to_string(),
            rows: vec![AdminRow {
                label: "site_id".to_string(),
                value: site.id.to_string(),
            }],
            links: vec![
                link(&format!("/admin/site/{site_id}/assets"), "Back to assets"),
                link(&format!("/admin/site/{site_id}/content"), "Back to content"),
            ],
            inline_body: admin_site_assets_new_form_html().to_string(),
            pre_body: String::new(),
        }),
        Err(error) => Err(SiteError::internal(format!(
            "failed to load site {site_id}: {error}"
        ))),
    }
}

async fn admin_site_assets_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Redirect, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Author).await?;
    let site = match get_site(state.db.as_ref(), site_id_uuid).await {
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
    let storage_path = StdPath::new(UPLOAD_ROOT).join(&storage_basename);

    if let Err(error) = fs::create_dir_all(UPLOAD_ROOT).await {
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
    let asset = state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            let original_filename = original_filename.clone();
            let storage_basename = storage_basename.clone();
            let mime_type = mime_type.clone();
            Box::pin(async move {
                let asset = create_asset(
                    txn,
                    NewAsset {
                        site_id: site.id,
                        uploader_sub: ADMIN_ACTOR_SUB.to_string(),
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
                    txn,
                    ADMIN_ACTOR_SUB,
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
                .map_err(|error| {
                    SiteError::internal(format!("failed to log asset audit: {error}"))
                })?;

                create_asset_variant(
                    txn,
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
                .map_err(|error| {
                    SiteError::internal(format!("failed to create asset variant: {error}"))
                })?;

                if let Some(thumbnail) = thumbnail {
                    let stem = StdPath::new(&asset.storage_basename)
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("asset");
                    let filename = format!("{stem}_thumb.{}", thumbnail.extension);
                    let thumb_path = StdPath::new(UPLOAD_ROOT).join(&filename);
                    if let Err(error) = fs::write(&thumb_path, &thumbnail.bytes).await {
                        return Err(SiteError::internal(format!(
                            "failed to write thumbnail: {error}"
                        )));
                    }

                    create_asset_variant(
                        txn,
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

                Ok(asset)
            })
        })
        .await
        .map_err(|error| SiteError::internal(error.to_string()))?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/assets")))
}

fn admin_site_assets_new_form_html() -> &'static str {
    r#"
      <form method="post" action="" enctype="multipart/form-data">
        <label for="file">Upload File</label>
        <input id="file" name="file" type="file" required />

        <button type="submit">Upload</button>
      </form>
    "#
}

struct ThumbnailResult {
    bytes: Vec<u8>,
    extension: String,
    mime_type: String,
    byte_length: i32,
    width: Option<i32>,
    height: Option<i32>,
}

async fn generate_thumbnail(
    bytes: Vec<u8>,
    extension: &str,
) -> Result<(Option<(u32, u32)>, Option<ThumbnailResult>), String> {
    let extension = extension.to_string();
    tokio::task::spawn_blocking(move || {
        let format = ImageFormat::from_extension(&extension);
        let image = match format {
            Some(format) => image::load_from_memory_with_format(&bytes, format),
            None => image::load_from_memory(&bytes),
        };

        let Ok(image) = image else {
            return Ok((None, None));
        };

        let (width, height) = image.dimensions();
        let thumbnail = image.thumbnail(THUMBNAIL_MAX_SIZE, THUMBNAIL_MAX_SIZE);
        let thumb_format = format.unwrap_or(ImageFormat::Png);
        let thumb_ext = image_format_extension(thumb_format).to_string();
        let mut thumb_bytes = Vec::new();
        thumbnail
            .write_to(&mut Cursor::new(&mut thumb_bytes), thumb_format)
            .map_err(|error| error.to_string())?;

        let mime_type = mime_from_extension(thumb_ext.as_str()).to_string();
        let byte_length = i32::try_from(thumb_bytes.len()).unwrap_or(i32::MAX);
        let width_i32 = i32::try_from(thumbnail.width()).ok();
        let height_i32 = i32::try_from(thumbnail.height()).ok();

        Ok((
            Some((width, height)),
            Some(ThumbnailResult {
                bytes: thumb_bytes,
                extension: thumb_ext,
                mime_type,
                byte_length,
                width: width_i32,
                height: height_i32,
            }),
        ))
    })
    .await
    .map_err(|error| error.to_string())?
}

fn image_format_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Png => "png",
        ImageFormat::Gif => "gif",
        ImageFormat::WebP => "webp",
        _ => "png",
    }
}

fn mime_from_extension(extension: &str) -> &'static str {
    match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
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

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn parse_uuid_param(value: &str, label: &str) -> Result<Uuid, SiteError> {
    Uuid::parse_str(value).map_err(|error| SiteError::internal(format!("invalid {label}: {error}")))
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

async fn admin_site_settings(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<String>,
) -> Result<AdminPageTemplate, SiteError> {
    let site_id_uuid = parse_uuid_param(&site_id, "site_id")?;
    require_site_role(&state, &session, site_id_uuid, SiteRole::Viewer).await?;
    get_site(state.db.as_ref(), site_id_uuid)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))
        .map(|site| {
            let rows = vec![
                AdminRow {
                    label: "id".to_string(),
                    value: site.id.to_string(),
                },
                AdminRow {
                    label: "short_name".to_string(),
                    value: site.short_name,
                },
                AdminRow {
                    label: "full_title".to_string(),
                    value: site.full_title,
                },
                AdminRow {
                    label: "template_name".to_string(),
                    value: site.template_name,
                },
            ];

            AdminPageTemplate {
                title: "Site Settings".to_string(),
                heading: format!("Site settings {site_id}"),
                message: "Site metadata and configuration are shown in this view.".to_string(),
                rows,
                links: vec![link(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to content",
                )],
                inline_body: String::new(),
                pre_body: String::new(),
            }
        })
}

async fn admin_site_render(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminPageTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Editor).await?;
    render_site(state.db.as_ref(), site_id, "templates", "./rendered")
        .await
        .map_err(|error| SiteError::internal(format!("failed to render site {site_id}: {error}")))
        .map(|files_written| AdminPageTemplate {
            title: "Render".to_string(),
            heading: format!("Rendered site {site_id}"),
            message: format!("Rendered {files_written} file(s)."),
            rows: vec![],
            links: vec![link(
                &format!("/admin/site/{site_id}/content"),
                "Back to content",
            )],
            inline_body: String::new(),
            pre_body: String::new(),
        })
}

fn link(href: &str, label: &str) -> AdminLink {
    AdminLink {
        href: href.to_string(),
        label: label.to_string(),
    }
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

        let first = match ensure_site_owner_membership(db.as_ref(), "tester", site.id).await {
            Ok(value) => value,
            Err(_) => panic!("failed to create membership"),
        };
        assert!(first.is_some(), "expected membership on first call");
        if let Some(membership) = first {
            assert_eq!(membership.role, "owner");
        }

        let second = match ensure_site_owner_membership(db.as_ref(), "tester", site.id).await {
            Ok(value) => value,
            Err(_) => panic!("failed to check membership"),
        };
        assert!(second.is_none(), "expected no membership on second call");
    }
}
