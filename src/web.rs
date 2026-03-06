use askama::Template;
use axum::http::StatusCode;
use axum::{
    Router,
    extract::{Path, State},
    response::{IntoResponse, Response},
    routing::get,
};
use websites::{
    content_primary_route, get_content, get_site, list_aliases, list_assets, list_content,
    list_content_tags, list_revisions, list_sites, list_tags, render_site,
};

use crate::OidcConfig;

#[derive(Clone)]
struct AdminState {
    database_url: String,
    oidc_client_id: Option<String>,
    oidc_frontend_url: Option<String>,
    oidc_discovery_url: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/page.html")]
struct AdminPageTemplate {
    title: String,
    heading: String,
    message: String,
    rows: Vec<AdminRow>,
    links: Vec<AdminLink>,
    pre_body: String,
}

#[derive(Debug)]
struct AdminRow {
    label: String,
    value: String,
}

#[derive(Debug)]
struct AdminLink {
    href: String,
    label: String,
}

const ADMIN_STYLE: &str = include_str!("../templates/admin/assets/style.css");

pub async fn run_admin_server(
    database_url: &str,
    listen: &str,
    oidc: &OidcConfig,
) -> Result<(), String> {
    let state = AdminState {
        database_url: database_url.to_string(),
        oidc_client_id: oidc.oidc_client_id.clone(),
        oidc_frontend_url: oidc.frontend_url.clone(),
        oidc_discovery_url: oidc.oidc_discovery_url.clone(),
    };

    let app = Router::new()
        .route("/admin", get(admin_index))
        .route("/admin/sites", get(admin_sites))
        .route("/admin/sites/new", get(admin_sites_new))
        .route("/admin/login", get(admin_login))
        .route("/admin/logout", get(admin_logout))
        .route("/admin/site/:site_id/content", get(admin_site_content_list))
        .route(
            "/admin/site/:site_id/content/:content_id",
            get(admin_site_content_detail),
        )
        .route(
            "/admin/site/:site_id/content/:content_id/source",
            get(admin_site_content_source),
        )
        .route(
            "/admin/site/:site_id/content/:content_id/advanced",
            get(admin_site_content_advanced),
        )
        .route(
            "/admin/site/:site_id/content/:content_id/revisions",
            get(admin_site_content_revisions),
        )
        .route("/admin/site/:site_id/tags", get(admin_site_tags))
        .route("/admin/site/:site_id/assets", get(admin_site_assets))
        .route("/admin/site/:site_id/settings", get(admin_site_settings))
        .route("/admin/site/:site_id/render", get(admin_site_render))
        .route("/admin/assets/style.css", get(admin_style_css))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|error| error.to_string())?;
    let local = listener.local_addr().map_err(|error| error.to_string())?;

    println!("admin server listening on http://{local}");
    axum::serve(listener, app)
        .await
        .map_err(|error| error.to_string())
}

async fn admin_style_css() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        ADMIN_STYLE,
    )
}

async fn admin_index(State(state): State<AdminState>) -> Response {
    let mut links = vec![
        link("/admin/sites", "Sites"),
        link("/admin/login", "Login"),
        link("/admin/logout", "Logout"),
    ];
    if admin_oidc_is_configured(&state) {
        links.push(link("/admin/login", "OIDC configured"));
    }

    AdminPageTemplate {
        title: "Admin".to_string(),
        heading: "Administration".to_string(),
        message: "Use the route set below to browse admin surfaces. Authentication is wired next."
            .to_string(),
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
        pre_body: String::new(),
    }
    .into_response()
}

async fn admin_sites(State(state): State<AdminState>) -> Response {
    match list_sites(&state.database_url).await {
        Ok(sites) => {
            let rows = sites
                .into_iter()
                .map(|site| AdminRow {
                    label: site.short_name,
                    value: format!(
                        "{0} ({1}) [content: /admin/site/{0}/content, tags: /admin/site/{0}/tags, assets: /admin/site/{0}/assets, settings: /admin/site/{0}/settings, render: /admin/site/{0}/render]",
                        site.id, site.full_title
                    ),
                })
                .collect();

            AdminPageTemplate {
                title: "Sites".to_string(),
                heading: "Managed Sites".to_string(),
                message: "Add or inspect site-specific admin routes.".to_string(),
                rows,
                links: vec![link("/admin/sites/new", "New site")],
                pre_body: String::new(),
            }
            .into_response()
        }
        Err(error) => internal_error(format!("failed to load sites: {error}")),
    }
}

async fn admin_sites_new() -> Response {
    AdminPageTemplate {
        title: "New Site".to_string(),
        heading: "Create Site".to_string(),
        message: "POST-backed create flow will be added next; use CLI create for now.".to_string(),
        rows: vec![],
        links: vec![link("/admin/sites", "Back to sites")],
        pre_body: String::new(),
    }
    .into_response()
}

async fn admin_login() -> Response {
    AdminPageTemplate {
        title: "Login".to_string(),
        heading: "Admin Login".to_string(),
        message: "OIDC login flow is part of the next phase; this page is a placeholder."
            .to_string(),
        rows: vec![],
        links: vec![link("/admin/logout", "Logout")],
        pre_body: String::new(),
    }
    .into_response()
}

async fn admin_logout() -> Response {
    AdminPageTemplate {
        title: "Logout".to_string(),
        heading: "Admin Logout".to_string(),
        message:
            "OIDC logout endpoint will terminate the session once authentication middleware is active."
                .to_string(),
        rows: vec![],
        links: vec![link("/admin/login", "Login")],
        pre_body: String::new(),
    }
    .into_response()
}

async fn admin_site_content_list(
    State(state): State<AdminState>,
    Path(site_id): Path<String>,
) -> Response {
    match list_content(&state.database_url, &site_id, None).await {
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

            AdminPageTemplate {
                title: "Content".to_string(),
                heading: format!("Site Content {site_id}"),
                message: "Browse linked content routes for each item.".to_string(),
                rows,
                links: vec![
                    link(&format!("/admin/site/{site_id}/settings"), "Site settings"),
                    link(&format!("/admin/site/{site_id}/tags"), "Tags"),
                    link(&format!("/admin/site/{site_id}/assets"), "Assets"),
                    link(&format!("/admin/site/{site_id}/render"), "Render"),
                ],
                pre_body: String::new(),
            }
            .into_response()
        }
        Err(error) => internal_error(format!(
            "failed to load content for site {site_id}: {error}"
        )),
    }
}

async fn admin_site_content_detail(
    State(state): State<AdminState>,
    Path((_site_id, content_id)): Path<(String, String)>,
) -> Response {
    match get_content(&state.database_url, &content_id).await {
        Ok(content) => {
            let content_id = content.id.clone();
            let content_site_id = content.site_id.clone();
            let published_at = content
                .published_at
                .clone()
                .unwrap_or_else(|| "n/a".to_string());
            let page_type = content.page_type.clone();
            let slug = content.slug.clone();
            let title = content.title.clone();
            let rows = vec![
                AdminRow {
                    label: "id".to_string(),
                    value: content_id.clone(),
                },
                AdminRow {
                    label: "site_id".to_string(),
                    value: content_site_id.clone(),
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
            AdminPageTemplate {
                title: "Content Detail".to_string(),
                heading: format!("Content: /{route}"),
                message: format!("Creator: {}", content.creator_sub),
                rows,
                links: vec![
                    link(
                        &format!(
                            "/admin/site/{}/content/{}/source",
                            content_site_id, content_id
                        ),
                        "Source",
                    ),
                    link(
                        &format!(
                            "/admin/site/{}/content/{}/advanced",
                            content_site_id, content_id
                        ),
                        "Advanced",
                    ),
                    link(
                        &format!(
                            "/admin/site/{}/content/{}/revisions",
                            content_site_id, content_id
                        ),
                        "Revisions",
                    ),
                    link(
                        &format!("/admin/site/{}/content", content_site_id),
                        "Back to content",
                    ),
                ],
                pre_body: String::new(),
            }
            .into_response()
        }
        Err(error) => internal_error(format!("failed to load content {content_id}: {error}")),
    }
}

async fn admin_site_content_source(
    State(state): State<AdminState>,
    Path((_site_id, content_id)): Path<(String, String)>,
) -> Response {
    match get_content(&state.database_url, &content_id).await {
        Ok(content) => AdminPageTemplate {
            title: "Content Source".to_string(),
            heading: format!("Source: {}", content.title),
            message: "Raw markdown source for this content item.".to_string(),
            rows: vec![],
            links: vec![link(
                &format!("/admin/site/{}/content/{}", content.site_id, content.id),
                "Back to content",
            )],
            pre_body: content.page_content,
        }
        .into_response(),
        Err(error) => internal_error(format!("failed to load source for {content_id}: {error}")),
    }
}

async fn admin_site_content_advanced(
    State(state): State<AdminState>,
    Path((_site_id, content_id)): Path<(String, String)>,
) -> Response {
    let content = match get_content(&state.database_url, &content_id).await {
        Ok(content) => content,
        Err(error) => {
            return internal_error(format!("failed to load content {content_id}: {error}"));
        }
    };

    let aliases = match list_aliases(&state.database_url, &content.site_id, Some(&content.id)).await
    {
        Ok(value) => value,
        Err(error) => {
            return internal_error(format!(
                "failed to load aliases for content {content_id}: {error}"
            ));
        }
    };
    let tags = match list_content_tags(&state.database_url, &content.id).await {
        Ok(value) => value,
        Err(error) => {
            return internal_error(format!(
                "failed to load tags for content {content_id}: {error}"
            ));
        }
    };

    AdminPageTemplate {
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
                value: content.created_at,
            },
            AdminRow {
                label: "updated_at".to_string(),
                value: content.last_updated,
            },
        ],
        links: vec![link(
            &format!("/admin/site/{}/content/{}", content.site_id, content.id),
            "Back to content",
        )],
        pre_body: String::new(),
    }
    .into_response()
}

async fn admin_site_content_revisions(
    State(state): State<AdminState>,
    Path((site_id, content_id)): Path<(String, String)>,
) -> Response {
    match list_revisions(&state.database_url, &content_id).await {
        Ok(revisions) => {
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

            AdminPageTemplate {
                title: "Content Revisions".to_string(),
                heading: format!("Revisions for {content_id}"),
                message: "Latest revision is first in list order by revision number.".to_string(),
                rows,
                links: vec![link(
                    &format!("/admin/site/{site_id}/content/{content_id}"),
                    "Back to content",
                )],
                pre_body: String::new(),
            }
            .into_response()
        }
        Err(error) => internal_error(format!(
            "failed to load revisions for {content_id}: {error}"
        )),
    }
}

async fn admin_site_tags(State(state): State<AdminState>, Path(site_id): Path<String>) -> Response {
    match list_tags(&state.database_url, &site_id).await {
        Ok(tags) => {
            let rows = tags
                .into_iter()
                .map(|tag| AdminRow {
                    label: tag.name,
                    value: tag.id,
                })
                .collect();

            AdminPageTemplate {
                title: "Tags".to_string(),
                heading: format!("Site Tags ({site_id})"),
                message: "Tag definitions for this site.".to_string(),
                rows,
                links: vec![link(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to content",
                )],
                pre_body: String::new(),
            }
            .into_response()
        }
        Err(error) => internal_error(format!("failed to load tags for site {site_id}: {error}")),
    }
}

async fn admin_site_assets(
    State(state): State<AdminState>,
    Path(site_id): Path<String>,
) -> Response {
    match list_assets(&state.database_url, &site_id).await {
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

            AdminPageTemplate {
                title: "Assets".to_string(),
                heading: format!("Site Assets ({site_id})"),
                message: "Variant metadata will appear once upload pipeline is active.".to_string(),
                rows,
                links: vec![link(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to content",
                )],
                pre_body: String::new(),
            }
            .into_response()
        }
        Err(error) => internal_error(format!("failed to load assets for site {site_id}: {error}")),
    }
}

async fn admin_site_settings(
    State(state): State<AdminState>,
    Path(site_id): Path<String>,
) -> Response {
    match get_site(&state.database_url, &site_id).await {
        Ok(site) => {
            let rows = vec![
                AdminRow {
                    label: "id".to_string(),
                    value: site.id,
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
                pre_body: String::new(),
            }
            .into_response()
        }
        Err(error) => internal_error(format!("failed to load site {site_id}: {error}")),
    }
}

async fn admin_site_render(
    State(state): State<AdminState>,
    Path(site_id): Path<String>,
) -> Response {
    match render_site(&state.database_url, &site_id, "templates", "./rendered").await {
        Ok(files_written) => AdminPageTemplate {
            title: "Render".to_string(),
            heading: format!("Rendered site {site_id}"),
            message: format!("Rendered {files_written} file(s)."),
            rows: vec![],
            links: vec![link(
                &format!("/admin/site/{site_id}/content"),
                "Back to content",
            )],
            pre_body: String::new(),
        }
        .into_response(),
        Err(error) => internal_error(format!("failed to render site {site_id}: {error}")),
    }
}

fn admin_oidc_is_configured(state: &AdminState) -> bool {
    state.oidc_client_id.is_some()
        || state.oidc_frontend_url.is_some()
        || state.oidc_discovery_url.is_some()
}

fn link(href: &str, label: &str) -> AdminLink {
    AdminLink {
        href: href.to_string(),
        label: label.to_string(),
    }
}

fn internal_error(error: String) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, error).into_response()
}

impl IntoResponse for AdminPageTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(rendered) => rendered.into_response(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    }
}
