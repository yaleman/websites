use crate::api;
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
use crate::entities::site_publish_config::PublishMethod;
use crate::entities::user::upsert_user_login;
use crate::entities::{self, PageType};
use crate::errors::SiteError;
use crate::middleware::log_requests;
use crate::oidc::{admin_login_callback, build_http_client, build_oidc_client};
use crate::publish::{
    RsyncPublishConfig, S3CompatiblePublishConfig, delete_site_publish_config,
    get_s3_publish_config, get_site_publish_config, get_site_publish_run, list_site_publish_runs,
    publish_rendered_site, queue_site_publish, save_rsync_publish_config, save_s3_publish_config,
};
use crate::theme_registry::{
    ThemeAdminRow, ThemeInstallRequest, available_template_names, delete_theme, install_theme,
    theme_admin_rows, update_theme,
};
use crate::tls::build_tls_config;
use crate::token_auth::{
    self, TokenGrantSet, TokenSiteGrant, deserialize_grants_json, ensure_jwt_hs256_secret,
    issue_user_api_token, revoke_user_api_token, signer_from_secret, summarize_grants,
};
use crate::{
    NewAssetVariant, NewContent, cli::OidcConfig, content_primary_route, create_content,
    create_membership, create_site, create_tag, delete_membership, delete_site, delete_tag,
    deserialize_site_export, export_site, get_asset_for_site, get_membership_by_id,
    get_membership_for_subject, get_revision, get_revision_by_number, get_user_by_id,
    import_site_export, import_wordpress_xml, list_aliases, list_assets, list_content,
    list_content_tags, list_memberships, list_memberships_for_user_id, list_revisions, list_sites,
    list_sites_for_subject, list_tags, list_users, list_users_by_ids, persist_asset_files,
    render_content_preview, render_site, resolve_log_path, resolve_site_template_override_root,
    resolve_upload_root, search_all_content, search_content, serialize_site_export_pretty,
    store_uploaded_asset, sync_tags_to_content, update_content, update_membership_role,
    update_site_settings,
};
use anyhow::Context;
use askama::Template;
use askama_web::WebTemplate;
use axum::middleware::from_fn;
use axum::{
    Json, Router,
    body::Body,
    extract::{Form, Multipart, OriginalUri, Path, Query, RawForm, State},
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
    ActiveModelTrait, ColumnTrait as _, Condition, ConnectionTrait, DatabaseConnection,
    DeriveActiveEnum, EntityTrait, EnumIter, IntoActiveModel, Iterable, QueryFilter, QueryOrder,
    Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use similar::TextDiff;
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::ErrorKind;
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::path::{Component, Path as StdPath, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tower_http::services::ServeDir;
use tower_sessions::{Expiry, Session, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;
use tracing::{debug, error, info};
use url::Url;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

/// Holds all the common template-shared data for admin pages.
mod assets;
mod content;
mod dashboard;
mod router;
mod sites;
pub(crate) mod state;
mod themes;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use router::build_admin_app;
pub(crate) use router::run_admin_server;
pub(crate) use state::{AdminState, SiteRole};
