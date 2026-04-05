use super::state::*;
use super::*;
use super::{assets::*, content::*, dashboard::*, sites::*, themes::*};
use axum::extract::DefaultBodyLimit;

use crate::constants::ASSET_UPLOAD_MAX_BYTES;

const WORDPRESS_IMPORT_MAX_BYTES: usize = 64 * 1024 * 1024;

pub(crate) async fn health_check() -> Json<&'static str> {
    Json("ok")
}

pub(crate) fn build_admin_app(
    state: AdminState,
    assets_dir: &StdPath,
    upload_root: &StdPath,
) -> Router {
    let swagger_config =
        utoipa_swagger_ui::Config::new(["/api-doc/openapi.json"]).validator_url("none");

    let admin_routes = Router::new()
        .route("/admin", get(get_index))
        .route("/admin/sites", get(get_sites))
        .route("/admin/search", get(get_global_search))
        .route("/admin/users", get(admin_users).post(admin_users_create))
        .route("/admin/users/me", get(admin_user_profile_redirect))
        .route("/admin/users/{user_id}", get(admin_user_profile))
        .route(
            "/admin/users/{user_id}/tokens",
            axum::routing::post(admin_user_token_issue),
        )
        .route(
            "/admin/users/{user_id}/tokens/{token_id}/revoke",
            axum::routing::post(admin_user_token_revoke),
        )
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
            "/admin/site/{site_id}/assets/new",
            get(admin_site_assets_new)
                .post(admin_site_assets_create)
                .layer(DefaultBodyLimit::max(ASSET_UPLOAD_MAX_BYTES)),
        )
        .route(
            "/admin/site/{site_id}/assets/{asset_id}/replace",
            get(admin_site_asset_replace).post(admin_site_asset_replace_update),
        )
        .route(
            "/admin/site/{site_id}/settings",
            get(admin_site_settings).post(admin_site_settings_update),
        )
        .route(
            "/admin/site/{site_id}/settings/wordpress-import",
            axum::routing::post(admin_site_wordpress_import)
                .layer(DefaultBodyLimit::max(WORDPRESS_IMPORT_MAX_BYTES)),
        )
        .route(
            "/admin/site/{site_id}/publish",
            get(admin_site_publish).post(admin_site_publish_update),
        )
        .route(
            "/admin/site/{site_id}/publish/run",
            axum::routing::post(admin_site_publish_run),
        )
        .route(
            "/admin/site/{site_id}/publish/run/{run_id}",
            get(admin_site_publish_run_detail),
        )
        .route("/admin/themes", get(admin_themes).post(admin_themes_create))
        .route(
            "/admin/themes/{slug}/update",
            axum::routing::post(admin_theme_update),
        )
        .route(
            "/admin/themes/{slug}/delete",
            axum::routing::post(admin_theme_delete),
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
        .nest_service("/admin/assets", ServeDir::new(assets_dir))
        .nest_service("/media/images", ServeDir::new(upload_root))
        .layer(from_fn(crate::middleware::require_session));
    Router::new()
        .route("/", get(admin_root))
        .route("/health", get(health_check))
        .route("/admin/login", get(admin_login))
        .route("/oauth2/callback", get(admin_login_callback))
        .route("/admin/logout", get(admin_logout))
        .route("/admin/logs", get(admin_logs))
        .merge(
            SwaggerUi::new("/api-docs")
                .url("/api-doc/openapi.json", ApiDoc::openapi())
                .config(swagger_config),
        )
        .merge(admin_routes)
        .merge(api::routes())
        .fallback(not_found)
        .with_state(state)
}

pub async fn run_admin_server(
    db: Arc<DatabaseConnection>,
    listen: &str,
    oidc: &OidcConfig,
    site_templates_root: PathBuf,
    upload_root: PathBuf,
    rendered_root: PathBuf,
    log_path: PathBuf,
) -> Result<(), anyhow::Error> {
    let jwt_secret = ensure_jwt_hs256_secret(db.as_ref())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let jwt_signer =
        signer_from_secret(&jwt_secret).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let state = AdminState {
        db: db.clone(),
        oidc_client_id: ClientId::new(oidc.oidc_client_id.clone()),
        oidc_client_secret: oidc.oidc_client_secret.clone().map(ClientSecret::new),
        oidc_frontend_url: oidc.frontend_url.clone(),
        oidc_discovery_url: IssuerUrl::new(oidc.oidc_discovery_url.clone())
            .context("Failed to parse discovery URL")?,
        oidc_client: Arc::new(build_http_client().context("Failed to build OIDC HTTP client")?),
        jwt_signer: Arc::new(jwt_signer),
        jwt_issuer: oidc.frontend_url.to_string(),
        upload_root: upload_root.clone(),
        log_path: log_path.clone(),
        site_templates_root,
        rendered_root,
    };

    let pool = db.get_sqlite_connection_pool();
    let session_store = SqliteStore::new((*pool).clone());

    let assets_dir = resolve_admin_assets_dir();
    session_store
        .migrate()
        .await
        .context("Failed to migrate Session store")?;

    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnSessionEnd);

    if !assets_dir.join("editor.js").exists() {
        return Err(anyhow::anyhow!(
            "admin editor assets not found; run `pnpm run build:admin` to generate them",
        ));
    }
    if !assets_dir.join("admin.js").exists() {
        return Err(anyhow::anyhow!(
            "admin UI assets not found; run `pnpm run build:admin` to generate them",
        ));
    }
    debug!("admin assets dir: {}", assets_dir.display());
    debug!("upload root dir: {}", upload_root.display());
    debug!("log path: {}", log_path.display());

    info!(
        "Starting server on https://{listen} / {}",
        state.oidc_frontend_url
    );

    let app = build_admin_app(state, &assets_dir, &upload_root)
        .layer(session_layer)
        .layer(from_fn(log_requests))
        .layer(from_fn(crate::middleware::set_cache));

    let tls_config = build_tls_config(&oidc.tls_cert_path, &oidc.tls_key_path).await?;
    let bind_addr: SocketAddr = SocketAddr::from_str(listen)
        .with_context(|| format!("failed to parse bind address {}", listen))?;

    #[cfg(unix)]
    let mut sigterm =
        signal(SignalKind::terminate()).context("failed to register SIGTERM handler")?;
    #[cfg(unix)]
    let mut sigquit = signal(SignalKind::quit()).context("failed to register SIGQUIT handler")?;
    #[cfg(unix)]
    let mut sighup = signal(SignalKind::hangup()).context("failed to register SIGHUP handler")?;

    let mut reload_generation = 0_u64;
    let shutdown_reason = loop {
        #[cfg(unix)]
        {
            use tracing::warn;

            tokio::select! {
                res = axum_server::bind_rustls(bind_addr, tls_config.clone())
                    .serve(app.clone().into_make_service()) => {
                        return res .inspect_err(|err| error!("admin server error: {err}"))
                    .context("axum rustls server exited unexpectedly")
                    },
                ctrl_c = tokio::signal::ctrl_c() => {
                    ctrl_c.context("failed waiting for ctrl-c")?;
                    info!("ctrl-c received; shutting down services");
                    break "SIGINT";
                }
                maybe_term = sigterm.recv() => {
                    if maybe_term.is_none() {
                        warn!("SIGTERM signal stream closed unexpectedly");
                        continue;
                    }
                    info!("SIGTERM received; shutting down services");
                    break "SIGTERM";
                }
                maybe_quit = sigquit.recv() => {
                    if maybe_quit.is_none() {
                        warn!("SIGQUIT signal stream closed unexpectedly");
                        continue;
                    }
                    info!("SIGQUIT received; shutting down services");
                    break "SIGQUIT";
                }
                maybe_hup = sighup.recv() => {
                    if maybe_hup.is_none() {
                        warn!("SIGHUP signal stream closed unexpectedly");
                        continue;
                    }
                    reload_generation = reload_generation.wrapping_add(1);
                    info!(reload_generation, "SIGHUP received; gracefully reloading all services");
                }
            }
            continue;
        }

        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c()
                .await
                .context("failed waiting for ctrl-c")?;
            info!("ctrl-c received; shutting down services");
            break "SIGINT";
        }
    };
    info!("shutdown signal received ({shutdown_reason}); exiting");
    Ok(())
}

pub(crate) fn resolve_admin_assets_dir() -> PathBuf {
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
