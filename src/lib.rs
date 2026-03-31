#![cfg_attr(not(test), forbid(unsafe_code))]
#![deny(warnings)]
#![deny(deprecated)]
#![recursion_limit = "512"]
#![warn(unused_extern_crates)]
// Enable some groups of clippy lints.
#![deny(clippy::suspicious)]
#![deny(clippy::perf)]
// Specific lints to enforce.
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::await_holding_lock)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::trivially_copy_pass_by_ref)]
#![deny(clippy::disallowed_types)]
#![deny(clippy::manual_let_else)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::unreachable)]

use crate::constants::{
    CUSTOMIZABLE_TEMPLATE_FILES, LOG_PATH, REQUIRED_TEMPLATES, SITE_TEMPLATES_DIR,
};
use crate::content_scan::expand_asset_shortcodes;
use crate::images::{generate_thumbnail, mime_from_extension};
use crate::web::SiteRole;
use crate::{entities::PageType, errors::SiteError};
use chrono::{DateTime, Datelike, Utc};
use markdown::{CompileOptions, Options};
use quick_xml::Reader;
use quick_xml::events::Event;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use serde::Serialize;
use std::collections::HashSet;
use std::env;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tera::{Context, Tera};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error};
use url::Url;
use uuid::Uuid;

pub mod api;
pub mod api_docs;
pub mod cli;
pub mod constants;
pub mod content_scan;
pub mod csrf;
pub mod db;
pub mod entities;
pub mod errors;
pub mod images;
pub mod middleware;
pub mod migration;
pub mod oidc;
pub mod publish;
pub mod site_export;
pub mod telemetry;
pub mod theme_registry;
pub mod tls;
pub mod token_auth;
pub mod web;

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Arc, OnceLock};
    use tokio::sync::Mutex;

    pub fn env_lock() -> Arc<Mutex<()>> {
        static LOCK: OnceLock<Arc<Mutex<()>>> = OnceLock::new();
        LOCK.get_or_init(|| Arc::new(Mutex::new(()))).clone()
    }
}

pub use publish::{
    PublishOutcome, RsyncPublishConfig, S3CompatiblePublishConfig, delete_site_publish_config,
    get_rsync_publish_config, get_s3_publish_config, list_site_publish_runs, queue_site_publish,
    save_rsync_publish_config, save_s3_publish_config,
};
pub use site_export::{
    SITE_EXPORT_FORMAT_VERSION, SiteExport, SiteImportResult, deserialize_site_export, export_site,
    export_site_with_roots, import_site_export, import_site_json, serialize_site_export_pretty,
};

pub struct NewContent {
    pub site_id: Uuid,
    pub page_type: PageType,
    pub title: String,
    pub slug: String,
    pub page_content: String,
    pub draft: bool,
    pub creator_sub: String,
    pub published_at: Option<DateTime<Utc>>,
}

pub struct NewAlias {
    pub content_id: Uuid,
    pub site_id: Uuid,
    pub alias_path: String,
    pub kind: String,
}

pub struct NewTag {
    pub site_id: Uuid,
    pub name: String,
}

pub struct NewContentTag {
    pub content_id: Uuid,
    pub site_id: Uuid,
    pub tag_name: String,
}

pub struct NewUser {
    pub subject: String,
}

pub struct NewMembership {
    pub site_id: Uuid,
    pub user_id: Uuid,
    pub role: SiteRole,
}

pub struct NewAsset {
    pub site_id: Uuid,
    pub uploader_sub: String,
    pub original_filename: String,
    pub storage_basename: String,
    pub mime_type: String,
    pub byte_length: i32,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

pub struct NewAssetVariant {
    pub asset_id: Uuid,
    pub variant_kind: String,
    pub filename: String,
    pub mime_type: String,
    pub byte_length: i32,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

pub struct UpdateContent {
    pub content_id: Uuid,
    pub page_type: Option<PageType>,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub page_content: Option<String>,
    pub draft: Option<bool>,
    pub published_at: Option<DateTime<Utc>>,
    pub editor_sub: String,
}

#[derive(Clone, Serialize)]
struct SiteListItem {
    title: String,
    url: String,
}

pub fn resolve_upload_root() -> PathBuf {
    if let Ok(value) = env::var("WEBSITES_UPLOAD_ROOT") {
        return PathBuf::from(value);
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut candidates = vec![cwd.join("uploads/media-storage")];

    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("uploads/media-storage"));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join("uploads/media-storage"));
        }
    }

    for candidate in candidates {
        if candidate.exists() {
            return candidate;
        }
    }

    cwd.join("uploads/media-storage")
}

pub fn resolve_log_path() -> PathBuf {
    if let Ok(value) = env::var("WEBSITES_LOG_PATH") {
        return PathBuf::from(value);
    }

    PathBuf::from(LOG_PATH)
}

pub fn resolve_site_template_override_root_with_upload_root(
    upload_root: &Path,
    site_id: Uuid,
) -> PathBuf {
    upload_root
        .join(".site-template-overrides")
        .join(site_id.to_string())
}

pub fn resolve_site_template_override_root(site_id: Uuid) -> PathBuf {
    resolve_site_template_override_root_with_upload_root(&resolve_upload_root(), site_id)
}

pub fn resolve_site_templates_root() -> PathBuf {
    if let Ok(value) = env::var("WEBSITES_SITE_TEMPLATES_DIR") {
        return PathBuf::from(value);
    }

    for candidate in bundled_site_templates_roots() {
        if candidate.exists() {
            return candidate;
        }
    }

    PathBuf::from(SITE_TEMPLATES_DIR)
}

pub fn is_customizable_template_file(filename: &str) -> bool {
    CUSTOMIZABLE_TEMPLATE_FILES.contains(&filename)
}

/// Returns audit events, optionally filtered by site_id.
pub async fn list_audit_events(
    db: &DatabaseConnection,
    site_id: Option<Uuid>,
) -> Result<Vec<entities::audit_event::Model>, String> {
    let query = entities::audit_event::Entity::find();
    let query = if let Some(site_id) = site_id {
        query.filter(entities::audit_event::Column::SiteId.eq(site_id))
    } else {
        query
    };
    let events = query
        .order_by_desc(entities::audit_event::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(events)
}

/// Builds the default "site_context" which is used by everything
fn default_context(site: &entities::site::Model, page_title: &str) -> tera::Context {
    let mut context = Context::new();
    context.insert("site_title", &site.full_title);
    context.insert("page_title", page_title);
    context
}

async fn write_atom(
    tmp_root: &Path,
    tera: &Tera,
    site: &entities::site::Model,
    post_items: &[entities::content_item::Model],
) -> Result<(), SiteError> {
    let atom_entries = render_atom_entries_xml(post_items);
    let updated = post_items
        .first()
        .map(|m| m.content_publish_timestamp())
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let mut atom_context = default_context(site, &site.full_title);
    atom_context.insert("link", "/");
    atom_context.insert("updated", &updated);
    atom_context.insert("entries", &atom_entries);
    let rendered_atom = tera.render("atom.xml", &atom_context)?;
    fs::write(tmp_root.join("atom.xml"), rendered_atom.as_bytes()).await?;
    Ok(())
}

async fn write_rss(
    tmp_root: &Path,
    tera: &Tera,
    site: &entities::site::Model,
    post_items: &[entities::content_item::Model],
) -> Result<(), SiteError> {
    let rss_items = render_rss_items_xml(post_items);
    let mut rss_context = default_context(site, &site.full_title);
    rss_context.insert("link", "/");
    rss_context.insert("updated", &Utc::now().to_rfc2822());
    rss_context.insert("items", &rss_items);
    let rendered_rss = tera.render("rss.xml", &rss_context)?;
    fs::write(tmp_root.join("rss.xml"), rendered_rss.as_bytes()).await?;
    Ok(())
}

async fn write_index(
    tmp_root: &Path,
    tera: &Tera,
    site: &entities::site::Model,
    items: &[SiteListItem],
) -> Result<(), SiteError> {
    let mut index_context = default_context(site, &site.full_title);
    index_context.insert("items", items);
    let rendered_index = tera.render("index.html", &index_context)?;
    fs::create_dir_all(&tmp_root).await?;
    fs::write(tmp_root.join("index.html"), rendered_index).await?;
    Ok(())
}

/// Renders all published content for a site into ./rendered/<site_short_name>.
pub async fn render_site(
    db: &DatabaseConnection,
    site_id: Uuid,
    templates_dir: &Path,
    rendered_dir: &Path,
    upload_root: &Path,
) -> Result<usize, SiteError> {
    let override_root = resolve_site_template_override_root_with_upload_root(upload_root, site_id);
    fs::create_dir_all(rendered_dir).await?;
    let tmp_root = tempfile::tempdir_in(rendered_dir).map_err(|err| {
        SiteError::internal(format!("failed to create temporary directory: {err}"))
    })?;
    let files_written = render_site_into_dir(
        db,
        site_id,
        templates_dir,
        tmp_root.path(),
        upload_root,
        &override_root,
    )
    .await?;

    let site = entities::site::Entity::find_by_id(site_id)
        .one(db)
        .await?
        .ok_or_else(|| SiteError::internal("site not found"))?;
    let rendered_root = rendered_dir.join(site.short_name);
    if fs::metadata(&rendered_root).await.is_ok() {
        fs::remove_dir_all(&rendered_root).await?;
    }
    fs::rename(tmp_root.path(), &rendered_root).await?;

    Ok(files_written)
}

pub(crate) async fn render_site_into_dir(
    db: &DatabaseConnection,
    site_id: Uuid,
    templates_dir: &Path,
    output_root: &Path,
    upload_root: &Path,
    override_root: &Path,
) -> Result<usize, SiteError> {
    let site = entities::site::Entity::find_by_id(site_id)
        .one(db)
        .await?
        .ok_or_else(|| SiteError::internal("site not found"))?;

    let content_items = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site.id))
        .filter(entities::content_item::Column::Draft.eq(false))
        .order_by_desc(entities::content_item::Column::CreatedAt)
        .all(db)
        .await?;
    let now = Utc::now();
    let content_items = content_items
        .into_iter()
        .filter(|item| content_is_publishable_at(item, now))
        .collect::<Vec<_>>();

    let template_root = templates_dir.join(site.template_name.clone());

    let tera = load_site_templates(&template_root, override_root).await?;

    fs::create_dir_all(output_root).await?;

    let mut files_written = 0usize;
    let mut index_items = Vec::new();

    for item in &content_items {
        let mut routes = HashSet::new();
        routes.insert(content_primary_route(item));

        let aliases = entities::content_alias::Entity::find()
            .filter(entities::content_alias::Column::ContentId.eq(item.id))
            .all(db)
            .await?;
        for alias in aliases {
            routes.insert(alias.alias_path);
        }

        let html = render_content_html(db, item.site_id, &item.page_content).await?;

        let tags = load_tag_names(db, item.id).await?;
        let tag_links = render_tag_links(&tags);
        let mut context = default_context(&site, &item.title);
        context.insert("title", &item.title);
        context.insert("content", &html);
        context.insert("slug", &item.slug);
        context.insert("page_type", &item.page_type);
        context.insert("created_at", &item.created_at);
        context.insert("published_at", &item.published_at);
        context.insert("content_id", &item.id);
        context.insert(
            "primary_url",
            &format!("/{}", content_primary_route(item).trim_matches('/')),
        );
        context.insert("tags", &tags);
        context.insert("tag_links", &tag_links);
        let rendered = tera.render(item.page_type.template(), &context)?;

        for route in routes {
            let route = route.trim_end_matches('/').trim_start_matches('/');
            let output_dir = if route.is_empty() {
                output_root.to_path_buf()
            } else {
                output_root.join(route)
            };

            fs::create_dir_all(&output_dir).await?;
            fs::write(output_dir.join("index.html"), rendered.as_bytes()).await?;
            files_written = files_written.saturating_add(1);
        }

        let primary_route = content_primary_route(item);
        index_items.push(SiteListItem {
            title: item.title.clone(),
            url: format!("/{}", primary_route.trim_matches('/')),
        });
    }

    write_index(output_root, &tera, &site, &index_items).await?;
    files_written = files_written.saturating_add(1);

    let post_items = content_items
        .iter()
        .filter(|item| item.page_type.is_post())
        .cloned()
        .collect::<Vec<_>>();

    write_rss(output_root, &tera, &site, &post_items).await?;
    files_written = files_written.saturating_add(1);

    write_atom(output_root, &tera, &site, &post_items).await?;
    files_written = files_written.saturating_add(1);

    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(site.id))
        .all(db)
        .await?;

    for tag in tags {
        let mut tag_items = Vec::new();
        let links = entities::content_tag::Entity::find()
            .filter(entities::content_tag::Column::TagId.eq(tag.id))
            .all(db)
            .await?;

        for link in links {
            if let Some(content) = entities::content_item::Entity::find_by_id(link.content_id)
                .filter(entities::content_item::Column::Draft.eq(false))
                .one(db)
                .await?
                .filter(|content| content_is_publishable_at(content, now))
            {
                let route = content_primary_route(&content);
                tag_items.push(SiteListItem {
                    title: content.title,
                    url: format!("/{}", route.trim_matches('/')),
                });
            }
        }

        let mut tag_context = default_context(&site, &format!("Tag: {}", tag.name));
        tag_context.insert("tag", &tag.name);
        tag_context.insert("items", &tag_items);
        let tag_output = tera.render("tag.html", &tag_context)?;
        let tag_slug = sanitize_tag_slug(&tag.name);
        let tag_path = output_root.join("tags").join(tag_slug);

        fs::create_dir_all(&tag_path).await?;
        fs::write(tag_path.join("index.html"), tag_output.as_bytes()).await?;
        files_written = files_written.saturating_add(1);
    }

    let template_assets = template_root.join("assets");
    copy_directory_recursive(
        &template_assets,
        &output_root.join("assets"),
        &mut files_written,
    )
    .await?;

    copy_media_variants(
        db,
        site.id,
        upload_root,
        &output_root.join("media/images"),
        &mut files_written,
    )
    .await?;

    Ok(files_written)
}

pub async fn render_content_preview(
    db: &DatabaseConnection,
    site_id: Uuid,
    content_id: Uuid,
    templates_dir: PathBuf,
    upload_root: &Path,
) -> Result<String, SiteError> {
    let override_root = resolve_site_template_override_root_with_upload_root(upload_root, site_id);
    render_content_preview_with_overrides(db, site_id, content_id, templates_dir, &override_root)
        .await
}

async fn render_content_preview_with_overrides(
    db: &DatabaseConnection,
    site_id: Uuid,
    content_id: Uuid,
    templates_dir: PathBuf,
    override_root: &Path,
) -> Result<String, SiteError> {
    let site = entities::site::Entity::find_by_id(site_id)
        .one(db)
        .await?
        .ok_or_else(|| SiteError::SiteNotFound(site_id.to_string()))?;

    let content = entities::content_item::Entity::find_by_id(content_id)
        .filter(entities::content_item::Column::SiteId.eq(site.id))
        .one(db)
        .await?
        .ok_or_else(|| SiteError::ContentNotFound(content_id))?;

    let template_root = templates_dir.join(site.template_name.clone());
    let tera = load_site_templates(&template_root, override_root).await?;

    let html = render_content_html(db, content.site_id, &content.page_content).await?;

    let tags = load_tag_names(db, content.id).await?;
    let tag_links = render_tag_links(&tags);
    let mut context = Context::new();
    context.insert("title", &content.title);
    context.insert("content", &html);
    context.insert("slug", &content.slug);
    context.insert("site_title", &site.full_title);
    context.insert("page_title", &content.title);
    context.insert("page_type", &content.page_type);
    context.insert("created_at", &content.created_at);
    context.insert("published_at", &content.published_at);
    context.insert("content_id", &content.id);
    context.insert(
        "primary_url",
        &format!("/{}", content_primary_route(&content).trim_matches('/')),
    );
    context.insert("tags", &tags);
    context.insert("tag_links", &tag_links);

    tera.render(content.page_type.template(), &context)
        .map_err(SiteError::from)
}

async fn render_content_html(
    db: &DatabaseConnection,
    site_id: Uuid,
    value: &str,
) -> Result<String, SiteError> {
    let expanded = expand_asset_shortcodes(db, site_id, value).await?;
    markdown::to_html_with_options(
        expanded.as_str(),
        &Options {
            compile: CompileOptions {
                allow_dangerous_html: true,
                ..CompileOptions::default()
            },
            ..Options::default()
        },
    )
    .map_err(|err| SiteError::internal(format!("Failed to render markdown: {err:?}")))
}

fn absolute_log_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn bundled_site_templates_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    push_unique_path(&mut roots, PathBuf::from(SITE_TEMPLATES_DIR));
    push_unique_path(&mut roots, PathBuf::from("/site_templates"));

    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        push_unique_path(&mut roots, dir.join("site_templates"));
        if let Some(parent) = dir.parent() {
            push_unique_path(&mut roots, parent.join("site_templates"));
        }
    }

    push_unique_path(
        &mut roots,
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("site_templates"),
    );

    roots
}

async fn load_template(
    template_root: &Path,
    override_root: &Path,
    filename: &str,
) -> Result<String, SiteError> {
    let override_path = override_root.join(filename);
    if let Ok(template) = fs::read_to_string(&override_path).await {
        return Ok(template);
    }

    let template_path = template_root.join(filename);
    match fs::read_to_string(&template_path).await {
        Ok(template) => Ok(template),
        Err(err) => {
            debug!(
                error=?err,
                path=%absolute_log_path(&template_path).display(),
                "Failed to load template, using fallback",
            );
            let configured_default_path = template_root
                .parent()
                .unwrap_or(template_root)
                .join("default")
                .join(filename);
            let mut fallback_paths = Vec::new();
            let bundled_roots = bundled_site_templates_roots();

            if let Some(template_name) = template_root.file_name() {
                for bundled_root in &bundled_roots {
                    push_unique_path(
                        &mut fallback_paths,
                        bundled_root.join(template_name).join(filename),
                    );
                }
            }

            push_unique_path(&mut fallback_paths, configured_default_path);
            for bundled_root in bundled_roots {
                push_unique_path(
                    &mut fallback_paths,
                    bundled_root.join("default").join(filename),
                );
            }

            let mut last_error = None;
            for fallback_path in fallback_paths {
                match fs::read_to_string(&fallback_path).await {
                    Ok(fallback) => return Ok(fallback),
                    Err(err) => {
                        last_error = Some((fallback_path, err));
                    }
                }
            }

            let Some((fallback_path, fallback_error)) = last_error else {
                error!(
                    error=?err,
                    path=%absolute_log_path(&template_path).display(),
                    "Failed to load template and no fallback paths were generated"
                );
                return Err(SiteError::from(err));
            };
            error!(
                error=?fallback_error,
                path=%absolute_log_path(&fallback_path).display(),
                "Failed to load fallback template"
            );
            Err(SiteError::from(fallback_error))
        }
    }
}

async fn add_raw_template(
    tera: &mut Tera,
    template_root: &Path,
    override_root: &Path,
    name: &str,
    contents: &str,
) -> Result<(), SiteError> {
    if let Err(err) = tera.add_raw_template(name, contents) {
        if let tera::ErrorKind::MissingParent { parent, .. } = &err.kind {
            if tera
                .get_template_names()
                .collect::<Vec<_>>()
                .contains(&parent.as_str())
            {
                error!(
                    "Parent template {} is reportedly missing but already exists in tera, cannot resolve!",
                    parent
                );
                return Err(SiteError::from(err));
            }
            debug!("Need to load a parent first!");
            let parent_contents = load_template(template_root, override_root, parent).await?;
            Box::pin(add_raw_template(
                tera,
                template_root,
                override_root,
                parent,
                &parent_contents,
            ))
            .await?;
            tera.add_raw_template(name, contents)
                .map_err(SiteError::from)
        } else {
            Err(SiteError::from(err))
        }
    } else {
        Ok(())
    }
}

async fn load_site_templates(
    template_root: &Path,
    override_root: &Path,
) -> Result<Tera, SiteError> {
    let mut tera = Tera::default();
    tera.autoescape_on(vec![]);

    let mut loaded_templates = HashSet::new();

    for required_file in REQUIRED_TEMPLATES.iter() {
        let contents = load_template(template_root, override_root, required_file).await?;
        add_raw_template(
            &mut tera,
            template_root,
            override_root,
            required_file,
            &contents,
        )
        .await
        .inspect_err(|error| error!(error=?error, "failed to load required template"))?;
        loaded_templates.insert(required_file.to_string());
        debug!("Loaded required template {}", required_file);
    }

    let mut template_dir_glob = glob::glob(
        template_root
            .join("**/*.html")
            .to_str()
            .ok_or_else(|| SiteError::internal("invalid template directory path"))?,
    )
    .map_err(|error| SiteError::internal(format!("failed to read template directory: {error}")))?;

    while let Some(Ok(path)) = template_dir_glob.next() {
        if path.is_file()
            && let Some(filename) = path.file_name().and_then(|name| name.to_str())
            && !loaded_templates.contains(filename)
            && (filename.ends_with(".html") || filename.ends_with(".xml"))
        {
            let contents = fs::read_to_string(&path).await.inspect_err(
                |err| error!(error=?err, path=?path.display(), "Failed to read template!"),
            )?;
            add_raw_template(&mut tera, template_root, override_root, filename, &contents).await?;

            loaded_templates.insert(filename.to_string());
            debug!("Loaded template {}", filename);
        }
    }

    Ok(tera)
}

async fn copy_directory_recursive(
    source: &Path,
    destination: &Path,
    files_written: &mut usize,
) -> Result<(), SiteError> {
    let mut dirs = vec![(source.to_path_buf(), destination.to_path_buf())];

    while let Some((source_path, destination_path)) = dirs.pop() {
        let mut source_dir = match fs::read_dir(&source_path).await {
            Ok(dir) => dir,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                continue;
            }

            Err(error) => return Err(SiteError::internal(error.to_string())),
        };

        fs::create_dir_all(&destination_path).await?;

        while let Some(entry) = source_dir.next_entry().await? {
            let next_source = entry.path();
            let next_destination = destination_path.join(entry.file_name());
            let metadata = entry.metadata().await?;

            if metadata.is_dir() {
                dirs.push((next_source, next_destination));
            } else if metadata.is_file() {
                copy_file_if_exists(&next_source, &next_destination).await?;
                *files_written = files_written.saturating_add(1);
            }
        }
    }

    Ok(())
}

async fn copy_media_variants(
    db: &sea_orm::DatabaseConnection,
    site_id: Uuid,
    source_root: &Path,
    destination_root: &Path,
    files_written: &mut usize,
) -> Result<(), SiteError> {
    let assets = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .all(db)
        .await?;

    let mut media_files = HashSet::new();
    for asset in assets.iter() {
        media_files.insert(asset.storage_basename.clone());
    }

    let asset_ids = assets.into_iter().map(|asset| asset.id).collect::<Vec<_>>();
    if !asset_ids.is_empty() {
        let variants = entities::asset_variant::Entity::find()
            .filter(entities::asset_variant::Column::AssetId.is_in(asset_ids))
            .all(db)
            .await?;

        for variant in variants {
            media_files.insert(variant.filename);
        }
    }

    for filename in media_files {
        let source = source_root.join(&filename);
        let destination = destination_root.join(filename);
        if copy_file_if_exists(&source, &destination).await? {
            *files_written = files_written.saturating_add(1);
        }
    }

    Ok(())
}

async fn copy_file_if_exists(source: &Path, destination: &Path) -> Result<bool, SiteError> {
    match fs::metadata(source).await {
        Ok(metadata) if metadata.is_file() => {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).await?;
            }

            fs::copy(source, destination).await?;

            Ok(true)
        }
        _ => Ok(false),
    }
}

fn sanitize_tag_slug(tag_name: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;

    for c in tag_name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }

    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "tag".to_string()
    } else {
        slug
    }
}

async fn load_tag_names(
    db: &DatabaseConnection,
    content_id: Uuid,
) -> Result<Vec<String>, SiteError> {
    let links = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content_id))
        .all(db)
        .await?;

    if links.is_empty() {
        return Ok(Vec::new());
    }

    let tag_ids = links
        .into_iter()
        .map(|link| link.tag_id)
        .collect::<Vec<_>>();
    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::Id.is_in(tag_ids))
        .all(db)
        .await?;

    Ok(tags.into_iter().map(|tag| tag.name).collect())
}

fn render_tag_links(tags: &[String]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let links = tags
        .iter()
        .map(|tag| format!("<a href=\"/tags/{}/\">{}</a>", sanitize_tag_slug(tag), tag))
        .collect::<Vec<_>>();
    links.join(", ")
}

fn render_rss_items_xml(content_items: &[entities::content_item::Model]) -> String {
    let mut rows = String::new();
    for item in content_items {
        let pub_date = item.content_publish_timestamp_rfc2822();
        let route = content_primary_route(item);
        let route = route.trim_matches('/');
        let link = format!("/{route}/");
        let summary = escape_xml_text(item.page_content.as_str());
        let title = escape_xml_text(item.title.as_str());
        rows.push_str(&format!(
            "<item><title>{}</title><link>{}</link><guid isPermaLink=\"false\">{}</guid><pubDate>{}</pubDate><description>{}</description></item>",
            title, link, item.id, pub_date, summary
        ));
    }
    rows
}

fn render_atom_entries_xml(content_items: &[entities::content_item::Model]) -> String {
    let mut rows = String::new();
    for item in content_items {
        let route = content_primary_route(item);
        let route = route.trim_matches('/');
        let link = format!("/{route}/");
        let updated = item.content_publish_timestamp();
        let title = escape_xml_text(item.title.as_str());
        let published = item.content_publish_timestamp_rfc2822();
        let summary = escape_xml_text(item.page_content.as_str());
        rows.push_str(&format!(
            "<entry><title>{}</title><link href=\"{}\"/><id>{}</id><published>{}</published><updated>{}</updated><summary>{}</summary></entry>",
            title, link, item.id, published, updated, summary
        ));
    }
    rows
}

fn escape_xml_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn content_is_publishable_at(content: &entities::content_item::Model, now: DateTime<Utc>) -> bool {
    content.published_at.unwrap_or(content.created_at) <= now
}

/// Creates a site record and returns the persisted row.
pub async fn create_site<C: ConnectionTrait>(
    db: &C,
    short_name: String,
    full_title: String,
    template_name: String,
) -> Result<entities::site::Model, SiteError> {
    let now = Utc::now();
    let model = entities::site::ActiveModel {
        id: Set(Uuid::now_v7()),
        short_name: Set(short_name),
        full_title: Set(full_title),
        template_name: Set(template_name),
        publish_on_render: Set(false),
        created_at: Set(now),
        updated_at: Set(None),
    };

    model.insert(db).await.map_err(|error| {
        error!("Failed to insert site into db! {error}");
        SiteError::from(error)
    })
}

/// Updates site settings and returns the updated row.
pub async fn update_site_settings<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    full_title: String,
    template_name: String,
    publish_on_render: bool,
) -> Result<entities::site::Model, SiteError> {
    let existing = entities::site::Entity::find_by_id(site_id).one(db).await?;
    let Some(existing) = existing else {
        return Err(SiteError::SiteNotFound(site_id.to_string()));
    };
    let mut model = existing.into_active_model();
    model.full_title = Set(full_title);
    model.template_name = Set(template_name);
    model.publish_on_render = Set(publish_on_render);
    model.updated_at = Set(Some(Utc::now()));
    model.update(db).await.map_err(SiteError::from)
}

/// Deletes a site row by id.
pub async fn delete_site<C: ConnectionTrait>(db: &C, site_id: Uuid) -> Result<(), SiteError> {
    let existing = entities::site::Entity::find_by_id(site_id).one(db).await?;
    let Some(_) = existing else {
        return Err(SiteError::SiteNotFound(site_id.to_string()));
    };

    entities::site::Entity::delete_by_id(site_id)
        .exec(db)
        .await
        .map_err(SiteError::from)?;
    Ok(())
}

/// Returns all sites ordered by short name.
pub async fn list_sites(db: &DatabaseConnection) -> Result<Vec<entities::site::Model>, SiteError> {
    entities::site::Entity::find()
        .all(db)
        .await
        .map_err(SiteError::from)
}

/// Creates one content record and one revision snapshot in the same operation.
pub async fn create_content<C: ConnectionTrait>(
    db: &C,
    input: NewContent,
) -> Result<entities::content_item::Model, SiteError> {
    let now = Utc::now();
    let content_id = Uuid::now_v7();
    let revision_id = Uuid::now_v7();
    let NewContent {
        site_id,
        page_type,
        title,
        slug,
        page_content,
        draft,
        creator_sub,
        published_at,
    } = input;

    let published_at = if !draft {
        Some(published_at.unwrap_or(Utc::now()))
    } else {
        published_at
    };

    let content = entities::content_item::ActiveModel {
        id: Set(content_id),
        site_id: Set(site_id),
        page_type: Set(page_type),
        title: Set(title.clone()),
        slug: Set(slug.clone()),
        page_content: Set(page_content.clone()),
        draft: Set(draft),
        creator_sub: Set(creator_sub),
        created_at: Set(now),
        last_updated: Set(None),
        published_at: Set(published_at),
    }
    .insert(db)
    .await?;

    entities::content_revision::ActiveModel {
        id: Set(revision_id),
        content_id: Set(content.id),
        site_id: Set(site_id),
        revision_number: Set(1),
        title: Set(title),
        slug: Set(slug),
        page_content: Set(page_content),
        draft: Set(draft),
        page_type: Set(page_type),
        editor_sub: Set(content.creator_sub.clone()),
        created_at: Set(now),
    }
    .insert(db)
    .await?;

    Ok(content)
}

/// Updates a content row and appends a revision snapshot.
pub async fn update_content<C: ConnectionTrait>(
    db: &C,
    input: UpdateContent,
) -> Result<entities::content_item::Model, String> {
    let now = Utc::now();
    let existing = entities::content_item::Entity::find_by_id(input.content_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "content not found".to_string())?;

    let publish_now = input.draft == Some(false) && existing.draft;
    let published_at = if let Some(published_at) = input.published_at {
        Some(published_at)
    } else if publish_now {
        Some(now)
    } else {
        existing.published_at
    };

    let mut active = existing.clone().into_active_model();
    if let Some(page_type) = input.page_type {
        active.page_type = Set(page_type);
    }
    if let Some(title) = input.title.clone() {
        active.title = Set(title);
    }
    if let Some(slug) = input.slug {
        active.slug = Set(slug);
    }
    if let Some(page_content) = input.page_content {
        active.page_content = Set(page_content);
    }
    if let Some(draft) = input.draft {
        active.draft = Set(draft);
    }
    active.published_at = Set(published_at);
    active.last_updated = Set(Some(now));

    let content: entities::content_item::Model = active
        .update(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    let revisions = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(input.content_id))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;
    let revision_number = i32::try_from(revisions.len())
        .map_err(|error: std::num::TryFromIntError| error.to_string())?
        .saturating_add(1);

    let revision = entities::content_revision::ActiveModel {
        id: Set(Uuid::now_v7()),
        content_id: Set(content.id),
        site_id: Set(content.site_id),
        revision_number: Set(revision_number),
        title: Set(content.title.clone()),
        slug: Set(content.slug.clone()),
        page_content: Set(content.page_content.clone()),
        draft: Set(content.draft),
        page_type: Set(content.page_type),
        editor_sub: Set(input.editor_sub),
        created_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|error| error.to_string())?;

    let content_tags = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content.id))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    for content_tag in content_tags {
        entities::content_revision_tag::ActiveModel {
            id: Set(Uuid::now_v7()),
            revision_id: Set(revision.id),
            content_id: Set(content.id),
            tag_id: Set(content_tag.tag_id),
        }
        .insert(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;
    }

    let aliases = entities::content_alias::Entity::find()
        .filter(entities::content_alias::Column::ContentId.eq(content.id))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    for alias in aliases {
        let _revision_alias = entities::content_revision_alias::ActiveModel {
            id: Set(Uuid::now_v7()),
            revision_id: Set(revision.id),
            content_id: Set(content.id),
            alias_path: Set(alias.alias_path),
            kind: Set(alias.kind),
        }
        .insert(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;
    }

    Ok(content)
}

/// Returns all content records for a site, optionally filtered by page_type.
pub async fn list_content(
    db: &DatabaseConnection,
    site_id: Uuid,
    page_type: Option<PageType>,
) -> Result<Vec<entities::content_item::Model>, String> {
    let query = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id));
    let query = if let Some(filter) = page_type {
        query.filter(entities::content_item::Column::PageType.eq(filter))
    } else {
        query
    };

    let content = query.all(db).await.map_err(|error| error.to_string())?;

    Ok(content)
}

/// Returns a single content record scoped to a site.
pub async fn get_content_for_site(
    db: &DatabaseConnection,
    site_id: Uuid,
    content_id: Uuid,
) -> Result<Option<entities::content_item::Model>, SiteError> {
    entities::content_item::Entity::find_by_id(content_id)
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .one(db)
        .await
        .map_err(SiteError::from)
}

/// Deletes a content record scoped to a site.
pub async fn delete_content<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    content_id: Uuid,
) -> Result<(), SiteError> {
    let existing = entities::content_item::Entity::find_by_id(content_id)
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .one(db)
        .await
        .map_err(SiteError::from)?;
    if existing.is_none() {
        return Err(SiteError::ContentNotFound(content_id));
    }

    entities::content_item::Entity::delete_by_id(content_id)
        .exec(db)
        .await
        .map_err(SiteError::from)?;
    Ok(())
}

/// Search content for a site by title, slug, or body substring.
pub async fn search_content(
    db: &DatabaseConnection,
    site_id: Uuid,
    query: &str,
) -> Result<Vec<entities::content_item::Model>, SiteError> {
    let condition = content_search_condition(query);

    entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .filter(condition)
        .order_by_desc(entities::content_item::Column::CreatedAt)
        .all(db)
        .await
        .map_err(SiteError::from)
}

pub async fn search_all_content(
    db: &DatabaseConnection,
    query: &str,
) -> Result<Vec<entities::content_item::Model>, SiteError> {
    entities::content_item::Entity::find()
        .filter(content_search_condition(query))
        .order_by_desc(entities::content_item::Column::CreatedAt)
        .all(db)
        .await
        .map_err(SiteError::from)
}

fn content_search_condition(query: &str) -> Condition {
    let query_like = format!("%{}%", query.replace(" ", "%"));

    Condition::any()
        .add(
            entities::content_item::Column::Title
                .into_expr()
                .like(&query_like),
        )
        .add(
            entities::content_item::Column::Slug
                .into_expr()
                .like(&query_like),
        )
        .add(
            entities::content_item::Column::PageContent
                .into_expr()
                .like(&query_like),
        )
}

#[derive(Default, Clone)]
struct WordpressItem {
    post_id: Option<String>,
    post_type: Option<String>,
    title: Option<String>,
    slug: Option<String>,
    content: Option<String>,
    status: Option<String>,
    post_date: Option<String>,
    post_date_gmt: Option<String>,
    post_modified: Option<String>,
    post_modified_gmt: Option<String>,
    link: Option<String>,
    tags: Vec<String>,
}

pub async fn import_wordpress<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    file_path: &str,
    creator_sub: &str,
) -> Result<usize, SiteError> {
    let xml = fs::read_to_string(file_path).await?;
    import_wordpress_xml(db, site_id, &xml, creator_sub).await
}

pub async fn import_wordpress_xml<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    xml: &str,
    creator_sub: &str,
) -> Result<usize, SiteError> {
    let items = parse_wordpress_wxr(xml)?;
    let mut imported = 0usize;

    for item in items {
        let Some(post_type) = item
            .post_type
            .as_deref()
            .map(str::trim)
            .filter(|post_type| !post_type.is_empty())
            .and_then(|post_type| PageType::from_str(post_type).ok())
        else {
            continue;
        };

        let title = item.title.unwrap_or_else(|| "Untitled".to_string());
        let slug = item
            .slug
            .filter(|slug| !slug.trim().is_empty())
            .unwrap_or_else(|| normalize_slug(&title));
        let content = item.content.unwrap_or_default();
        let status = item.status.unwrap_or_else(|| "draft".to_string());
        let draft = status != "publish";
        let tag_names = item.tags;
        let created_at =
            resolve_wordpress_timestamp(item.post_date_gmt.as_deref(), item.post_date.as_deref());
        let updated_at = resolve_wordpress_timestamp(
            item.post_modified_gmt.as_deref(),
            item.post_modified.as_deref(),
        );
        let published_at = if draft { None } else { created_at };
        let mut known_aliases = Vec::new();
        if let Some(post_id) = item.post_id.as_deref().map(str::trim)
            && !post_id.is_empty()
        {
            known_aliases.push(format!("/?p={post_id}"));
        }
        if let Some(link) = item.link.as_deref()
            && let Some(alias_path) = wordpress_link_to_alias(link)
        {
            known_aliases.push(alias_path);
        }

        if wordpress_item_exists(
            db,
            site_id,
            post_type,
            slug.as_str(),
            known_aliases.as_slice(),
        )
        .await?
        {
            continue;
        }

        let imported_content = create_content(
            db,
            NewContent {
                site_id,
                page_type: post_type,
                title,
                slug,
                page_content: content,
                draft,
                creator_sub: creator_sub.to_string(),
                published_at,
            },
        )
        .await?;
        let content_model =
            apply_imported_content_timestamps(db, imported_content, created_at, updated_at).await?;
        if !tag_names.is_empty() {
            let revision = get_revision_by_number(db, content_model.id, 1)
                .await
                .map_err(|error| SiteError::internal(format!("failed to get revision: {error}")))?
                .ok_or_else(|| {
                    SiteError::internal(format!(
                        "missing initial revision for imported content {}",
                        content_model.id
                    ))
                })?;
            assign_tags_to_content(db, site_id, content_model.id, revision.id, tag_names)
                .await
                .map_err(|err| SiteError::internal(err.to_string()))?;
        }

        let mut alias_paths = HashSet::new();
        for alias_path in known_aliases {
            if !alias_paths.insert(alias_path.clone()) {
                continue;
            }
            create_alias_if_missing(
                db,
                NewAlias {
                    content_id: content_model.id,
                    site_id,
                    alias_path,
                    kind: "alias".to_string(),
                },
            )
            .await?;
        }

        imported = imported.saturating_add(1);
    }

    Ok(imported)
}

async fn apply_imported_content_timestamps<C: ConnectionTrait>(
    db: &C,
    content: entities::content_item::Model,
    created_at: Option<DateTime<Utc>>,
    last_updated: Option<DateTime<Utc>>,
) -> Result<entities::content_item::Model, SiteError> {
    if created_at.is_none() && last_updated.is_none() {
        return Ok(content);
    }

    let mut active = content.into_active_model();
    if let Some(created_at) = created_at {
        active.created_at = Set(created_at);
    }
    active.last_updated = Set(last_updated);

    let content = active.update(db).await?;

    if let Some(created_at) = created_at
        && let Some(revision) = entities::content_revision::Entity::find()
            .filter(entities::content_revision::Column::ContentId.eq(content.id))
            .filter(entities::content_revision::Column::RevisionNumber.eq(1))
            .one(db)
            .await?
    {
        let mut revision = revision.into_active_model();
        revision.created_at = Set(created_at);
        revision.update(db).await?;
    }

    Ok(content)
}

async fn wordpress_item_exists<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    page_type: PageType,
    slug: &str,
    alias_paths: &[String],
) -> Result<bool, SiteError> {
    for alias_path in alias_paths {
        let existing_alias = entities::content_alias::Entity::find()
            .filter(entities::content_alias::Column::SiteId.eq(site_id))
            .filter(entities::content_alias::Column::AliasPath.eq(alias_path.as_str()))
            .one(db)
            .await?;
        if existing_alias.is_some() {
            return Ok(true);
        }
    }

    let existing_content = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .filter(entities::content_item::Column::PageType.eq(page_type))
        .filter(entities::content_item::Column::Slug.eq(slug))
        .one(db)
        .await?;

    Ok(existing_content.is_some())
}

async fn create_alias_if_missing<C: ConnectionTrait>(
    db: &C,
    input: NewAlias,
) -> Result<Option<entities::content_alias::Model>, SiteError> {
    let existing_alias = entities::content_alias::Entity::find()
        .filter(entities::content_alias::Column::SiteId.eq(input.site_id))
        .filter(entities::content_alias::Column::AliasPath.eq(input.alias_path.as_str()))
        .one(db)
        .await?;
    if existing_alias.is_some() {
        return Ok(None);
    }

    create_alias(db, input).await.map(Some)
}

fn parse_wordpress_wxr(xml: &str) -> Result<Vec<WordpressItem>, SiteError> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut items = Vec::new();
    let mut current = WordpressItem::default();
    let mut current_tag = String::new();
    let mut current_category_domain = None;
    let mut in_item = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) => {
                let name = String::from_utf8_lossy(event.name().as_ref()).to_string();
                if name == "item" {
                    in_item = true;
                    current = WordpressItem::default();
                    current_category_domain = None;
                } else if in_item {
                    current_category_domain = if name == "category" {
                        wordpress_category_domain(&event)
                    } else {
                        None
                    };
                    current_tag = name;
                }
            }
            Ok(Event::End(event)) => {
                let name = String::from_utf8_lossy(event.name().as_ref()).to_string();
                if name == "item" && in_item {
                    in_item = false;
                    items.push(current.clone());
                    current = WordpressItem::default();
                    current_category_domain = None;
                } else if name == "category" {
                    current_category_domain = None;
                }
                current_tag.clear();
            }
            Ok(Event::Text(event)) => {
                if !in_item {
                    buf.clear();
                    continue;
                }
                let text = String::from_utf8_lossy(event.as_ref()).to_string();
                assign_wordpress_field(
                    &mut current,
                    current_tag.as_str(),
                    current_category_domain.as_deref(),
                    text.trim(),
                );
            }
            Ok(Event::CData(event)) => {
                if !in_item {
                    buf.clear();
                    continue;
                }
                let text = String::from_utf8_lossy(event.as_ref()).to_string();
                assign_wordpress_field(
                    &mut current,
                    current_tag.as_str(),
                    current_category_domain.as_deref(),
                    text.as_str(),
                );
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(SiteError::from(error)),
            _ => {}
        }
        buf.clear();
    }

    Ok(items)
}

fn wordpress_category_domain(event: &quick_xml::events::BytesStart<'_>) -> Option<String> {
    event
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.as_ref() == b"domain")
        .map(|attribute| String::from_utf8_lossy(attribute.value.as_ref()).to_string())
}

fn assign_wordpress_field(
    item: &mut WordpressItem,
    tag: &str,
    category_domain: Option<&str>,
    value: &str,
) {
    match tag {
        "title" => item.title = Some(value.to_string()),
        "link" => item.link = Some(value.to_string()),
        "wp:post_id" => item.post_id = Some(value.to_string()),
        "wp:post_name" => item.slug = Some(value.to_string()),
        "wp:post_type" => item.post_type = Some(value.to_string()),
        "wp:status" => item.status = Some(value.to_string()),
        "wp:post_date" => item.post_date = Some(value.to_string()),
        "wp:post_date_gmt" => item.post_date_gmt = Some(value.to_string()),
        "wp:post_modified" => item.post_modified = Some(value.to_string()),
        "wp:post_modified_gmt" => item.post_modified_gmt = Some(value.to_string()),
        "content:encoded" => item.content = Some(value.to_string()),
        "category" if category_domain == Some("post_tag") => item.tags.push(value.to_string()),
        _ => {}
    }
}

fn wordpress_date_to_utc(value: &str) -> Option<DateTime<Utc>> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "0000-00-00 00:00:00" {
        return None;
    }
    let rfc3339 = format!("{}Z", trimmed.replace(' ', "T"));
    DateTime::parse_from_rfc3339(&rfc3339)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn resolve_wordpress_timestamp(
    gmt_value: Option<&str>,
    local_value: Option<&str>,
) -> Option<DateTime<Utc>> {
    gmt_value
        .and_then(wordpress_date_to_utc)
        .or_else(|| local_value.and_then(wordpress_date_to_utc))
}

fn wordpress_link_to_alias(link: &str) -> Option<String> {
    if let Ok(url) = Url::parse(link) {
        let mut path = url.path().to_string();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        if path.is_empty() { None } else { Some(path) }
    } else if link.starts_with('/') {
        Some(link.to_string())
    } else {
        None
    }
}

fn normalize_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for c in value.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "post".to_string()
    } else {
        slug
    }
}

/// Creates a content alias entry.
pub async fn create_alias<C: ConnectionTrait>(
    db: &C,
    input: NewAlias,
) -> Result<entities::content_alias::Model, SiteError> {
    entities::content_alias::ActiveModel {
        id: Set(Uuid::now_v7()),
        content_id: Set(input.content_id),
        site_id: Set(input.site_id),
        alias_path: Set(input.alias_path),
        kind: Set(input.kind),
    }
    .insert(db)
    .await
    .map_err(SiteError::from)
}

/// Returns all content aliases, optionally scoped to content_id.
pub async fn list_aliases(
    db: &DatabaseConnection,
    site_id: Uuid,
    content_id: Option<Uuid>,
) -> Result<Vec<entities::content_alias::Model>, SiteError> {
    let query = entities::content_alias::Entity::find()
        .filter(entities::content_alias::Column::SiteId.eq(site_id));
    let query = if let Some(content_id) = content_id {
        query.filter(entities::content_alias::Column::ContentId.eq(content_id))
    } else {
        query
    };

    query.all(db).await.map_err(SiteError::from)
}

/// Creates a tag record.
pub async fn create_tag<C: ConnectionTrait>(
    db: &C,
    input: NewTag,
) -> Result<entities::tag::Model, SiteError> {
    let model = entities::tag::ActiveModel {
        id: Set(Uuid::now_v7()),
        site_id: Set(input.site_id),
        name: Set(input.name),
    };

    model.insert(db).await.map_err(SiteError::from)
}

/// Deletes a tag for a specific site.
pub async fn delete_tag<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    tag_id: Uuid,
) -> Result<(), SiteError> {
    let tag = entities::tag::Entity::find_by_id(tag_id)
        .one(db)
        .await
        .map_err(SiteError::from)?
        .ok_or_else(|| SiteError::internal("tag not found"))?;
    if tag.site_id != site_id {
        return Err(SiteError::UnAuthorized(
            "tag does not belong to site".to_string(),
        ));
    }

    entities::tag::Entity::delete_by_id(tag_id)
        .exec(db)
        .await
        .map_err(SiteError::from)?;
    Ok(())
}

/// Returns all tags for a site.
pub async fn list_tags(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> Result<Vec<entities::tag::Model>, SiteError> {
    entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(site_id))
        .all(db)
        .await
        .map_err(SiteError::from)
}

/// Adds a tag to content (creates the tag if missing).
pub async fn add_content_tag<C: ConnectionTrait>(
    db: &C,
    input: NewContentTag,
) -> Result<entities::content_tag::Model, String> {
    let content = entities::content_item::Entity::find_by_id(input.content_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "content not found".to_string())?;

    if content.site_id != input.site_id {
        return Err("content does not belong to provided site".to_string());
    }

    let tag = if let Some(tag) = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(input.site_id))
        .filter(entities::tag::Column::Name.eq(input.tag_name.clone()))
        .one(db)
        .await
        .map_err(|error| error.to_string())?
    {
        tag
    } else {
        entities::tag::ActiveModel {
            id: Set(Uuid::now_v7()),
            site_id: Set(input.site_id),
            name: Set(input.tag_name),
        }
        .insert(db)
        .await
        .map_err(|error| error.to_string())?
    };

    let model = entities::content_tag::ActiveModel {
        id: Set(Uuid::now_v7()),
        content_id: Set(input.content_id),
        tag_id: Set(tag.id),
    }
    .insert(db)
    .await
    .map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns tags currently attached to a content item.
pub async fn list_content_tags(
    db: &DatabaseConnection,
    content_id: Uuid,
) -> Result<Vec<entities::tag::Model>, String> {
    let links = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content_id))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    if links.is_empty() {
        return Ok(Vec::new());
    }

    let tag_ids = links
        .into_iter()
        .map(|link| link.tag_id)
        .collect::<Vec<_>>();
    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::Id.is_in(tag_ids))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(tags)
}

/// Returns all aliases for a specific content revision.
pub async fn list_revision_aliases(
    db: &DatabaseConnection,
    revision_id: Uuid,
) -> Result<Vec<entities::content_revision_alias::Model>, String> {
    let revision_aliases = entities::content_revision_alias::Entity::find()
        .filter(entities::content_revision_alias::Column::RevisionId.eq(revision_id))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(revision_aliases)
}

/// Returns all tags captured for a specific content revision.
pub async fn list_revision_tags(
    db: &DatabaseConnection,
    revision_id: Uuid,
) -> Result<Vec<entities::tag::Model>, String> {
    let links = entities::content_revision_tag::Entity::find()
        .filter(entities::content_revision_tag::Column::RevisionId.eq(revision_id))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    if links.is_empty() {
        return Ok(Vec::new());
    }

    let tag_ids = links
        .into_iter()
        .map(|link| link.tag_id)
        .collect::<Vec<_>>();
    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::Id.is_in(tag_ids))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(tags)
}

/// Returns all revisions for a content item sorted by revision number.
pub async fn list_revisions(
    db: &DatabaseConnection,
    content_id: Uuid,
) -> Result<Vec<entities::content_revision::Model>, String> {
    let revisions = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(content_id))
        .order_by_desc(entities::content_revision::Column::RevisionNumber)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(revisions)
}

/// Returns a single revision by id.
pub async fn get_revision(
    db: &DatabaseConnection,
    revision_id: Uuid,
) -> Result<entities::content_revision::Model, String> {
    let revision = entities::content_revision::Entity::find_by_id(revision_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "revision not found".to_string())?;

    Ok(revision)
}

/// Returns a revision by number, if present.
pub async fn get_revision_by_number<C: ConnectionTrait>(
    db: &C,
    content_id: Uuid,
    revision_number: i32,
) -> Result<Option<entities::content_revision::Model>, String> {
    let revision = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(content_id))
        .filter(entities::content_revision::Column::RevisionNumber.eq(revision_number))
        .one(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(revision)
}

/// Gets the primary path for this content item (based on page type, slug, and publish date).
pub fn content_primary_route(content: &entities::content_item::Model) -> String {
    let slug = content.slug.trim_matches('/').to_string();
    if content.page_type.is_page() {
        return slug;
    }

    let date_source = content.published_at.unwrap_or(content.created_at);

    format!("{}/{}", content_date_path(&date_source), slug)
}

/// Returns a path segment based on the date, in the format "YYYY/MM/DD".
fn content_date_path(date: &DateTime<Utc>) -> String {
    format!("{:04}/{:02}/{:02}", date.year(), date.month(), date.day())
}

/// Returns all users.
pub async fn list_users(db: &DatabaseConnection) -> Result<Vec<entities::user::Model>, String> {
    let users = entities::user::Entity::find()
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(users)
}

/// Creates one site membership record.
pub async fn create_membership<C: ConnectionTrait>(
    db: &C,
    input: NewMembership,
) -> Result<entities::site_membership::Model, SiteError> {
    if input.role.is_admin() {
        return Err(SiteError::BadRequest(
            "cannot assign admin role to a site".to_string(),
        ));
    }

    let model = entities::site_membership::ActiveModel {
        id: Set(Uuid::now_v7()),
        site_id: Set(input.site_id),
        user_id: Set(input.user_id),
        role: Set(input.role),
    };

    let model = model.insert(db).await.map_err(SiteError::from)?;

    Ok(model)
}

/// Returns memberships for a site.
pub async fn list_memberships(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> Result<Vec<entities::site_membership::Model>, String> {
    let memberships = entities::site_membership::Entity::find()
        .filter(entities::site_membership::Column::SiteId.eq(site_id))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(memberships)
}

/// Returns a membership by id.
pub async fn get_membership_by_id<C: ConnectionTrait>(
    db: &C,
    membership_id: Uuid,
) -> Result<Option<entities::site_membership::Model>, SiteError> {
    entities::site_membership::Entity::find_by_id(membership_id)
        .one(db)
        .await
        .map_err(SiteError::from)
}

/// Returns a user by subject.
pub async fn get_user_by_subject<C: ConnectionTrait>(
    db: &C,
    subject: &str,
) -> Result<Option<entities::user::Model>, SiteError> {
    entities::user::Entity::find()
        .filter(entities::user::Column::Subject.eq(subject.to_string()))
        .one(db)
        .await
        .map_err(SiteError::from)
}

/// Returns a user by id.
pub async fn get_user_by_id<C: ConnectionTrait>(
    db: &C,
    user_id: Uuid,
) -> Result<Option<entities::user::Model>, SiteError> {
    entities::user::Entity::find_by_id(user_id)
        .one(db)
        .await
        .map_err(SiteError::from)
}

/// Returns users by id.
pub async fn list_users_by_ids<C: ConnectionTrait>(
    db: &C,
    user_ids: Vec<Uuid>,
) -> Result<Vec<entities::user::Model>, SiteError> {
    if user_ids.is_empty() {
        return Ok(vec![]);
    }

    entities::user::Entity::find()
        .filter(entities::user::Column::Id.is_in(user_ids))
        .all(db)
        .await
        .map_err(SiteError::from)
}

/// Returns memberships for a user.
pub async fn list_memberships_for_user_id<C: ConnectionTrait>(
    db: &C,
    user_id: Uuid,
) -> Result<Vec<entities::site_membership::Model>, SiteError> {
    entities::site_membership::Entity::find()
        .filter(entities::site_membership::Column::UserId.eq(user_id))
        .all(db)
        .await
        .map_err(SiteError::from)
}

/// Returns a membership for a site and user subject.
pub async fn get_membership_for_subject<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    subject: &str,
) -> Result<Option<entities::site_membership::Model>, SiteError> {
    let Some(user) = get_user_by_subject(db, subject).await? else {
        return Ok(None);
    };

    entities::site_membership::Entity::find()
        .filter(entities::site_membership::Column::SiteId.eq(site_id))
        .filter(entities::site_membership::Column::UserId.eq(user.id))
        .one(db)
        .await
        .map_err(SiteError::from)
}

/// Returns all sites where the subject has a membership.
pub async fn list_sites_for_subject(
    db: &DatabaseConnection,
    subject: &str,
) -> Result<Vec<entities::site::Model>, SiteError> {
    let Some(user) = get_user_by_subject(db, subject).await? else {
        return Ok(vec![]);
    };

    let memberships = entities::site_membership::Entity::find()
        .filter(entities::site_membership::Column::UserId.eq(user.id))
        .all(db)
        .await?;

    if memberships.is_empty() {
        return Ok(vec![]);
    }

    let site_ids = memberships
        .iter()
        .map(|membership| membership.site_id)
        .collect::<Vec<_>>();
    let sites = entities::site::Entity::find()
        .filter(entities::site::Column::Id.is_in(site_ids))
        .order_by_asc(entities::site::Column::ShortName)
        .all(db)
        .await
        .map_err(SiteError::from)?;

    Ok(sites)
}

/// Updates the membership role.
pub async fn update_membership_role<C: ConnectionTrait>(
    db: &C,
    membership_id: Uuid,
    role: SiteRole,
) -> Result<entities::site_membership::Model, SiteError> {
    if role.is_admin() {
        return Err(SiteError::BadRequest(
            "cannot assign admin role to a site".to_string(),
        ));
    }

    let Some(membership) = get_membership_by_id(db, membership_id).await? else {
        return Err(SiteError::MembershipNotFound(membership_id));
    };
    let mut active = membership.into_active_model();
    active.role = Set(role);
    active.update(db).await.map_err(SiteError::from)
}

/// Deletes the membership by id.
pub async fn delete_membership<C: ConnectionTrait>(
    db: &C,
    membership_id: Uuid,
) -> Result<(), String> {
    entities::site_membership::Entity::delete_by_id(membership_id)
        .exec(db)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Assigns tags to content and its revision, creating tags as needed.
pub async fn assign_tags_to_content<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    content_id: Uuid,
    revision_id: Uuid,
    tag_names: Vec<String>,
) -> Result<(), SiteError> {
    let mut unique = HashSet::new();

    for raw in tag_names {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_string();
        if !unique.insert(normalized.clone()) {
            continue;
        }

        let existing_tag = entities::tag::Entity::find()
            .filter(entities::tag::Column::SiteId.eq(site_id))
            .filter(entities::tag::Column::Name.eq(normalized.clone()))
            .one(db)
            .await
            .map_err(|error| SiteError::internal(format!("failed to query tag: {error}")))?;
        let tag = match existing_tag {
            Some(tag) => tag,
            None => create_tag(
                db,
                NewTag {
                    site_id,
                    name: normalized.clone(),
                },
            )
            .await
            .map_err(|err| SiteError::internal(format!("Failed to create tag: {}", err)))?,
        };

        let existing_content_tag = entities::content_tag::Entity::find()
            .filter(entities::content_tag::Column::ContentId.eq(content_id))
            .filter(entities::content_tag::Column::TagId.eq(tag.id))
            .one(db)
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to query content tag: {error}"))
            })?;
        if existing_content_tag.is_none() {
            let content_tag = entities::content_tag::ActiveModel {
                id: Set(Uuid::now_v7()),
                content_id: Set(content_id),
                tag_id: Set(tag.id),
            };
            content_tag.insert(db).await.map_err(|error| {
                SiteError::internal(format!("failed to insert content tag: {error}"))
            })?;
        }

        let existing_revision_tag = entities::content_revision_tag::Entity::find()
            .filter(entities::content_revision_tag::Column::RevisionId.eq(revision_id))
            .filter(entities::content_revision_tag::Column::TagId.eq(tag.id))
            .one(db)
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to query revision tag: {error}"))
            })?;
        if existing_revision_tag.is_none() {
            let revision_tag = entities::content_revision_tag::ActiveModel {
                id: Set(Uuid::now_v7()),
                revision_id: Set(revision_id),
                content_id: Set(content_id),
                tag_id: Set(tag.id),
            };
            revision_tag.insert(db).await.map_err(|error| {
                SiteError::internal(format!("failed to insert revision tag: {error}"))
            })?;
        }
    }

    Ok(())
}

/// Replaces the current tag set for content and the specified revision.
pub async fn sync_tags_to_content<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    content_id: Uuid,
    revision_id: Uuid,
    tag_names: Vec<String>,
) -> Result<(), SiteError> {
    let mut unique = HashSet::new();
    let mut desired_tag_ids = Vec::new();

    for raw in tag_names {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_string();
        if !unique.insert(normalized.clone()) {
            continue;
        }

        let existing_tag = entities::tag::Entity::find()
            .filter(entities::tag::Column::SiteId.eq(site_id))
            .filter(entities::tag::Column::Name.eq(normalized.clone()))
            .one(db)
            .await
            .map_err(|error| SiteError::internal(format!("failed to query tag: {error}")))?;
        let tag = match existing_tag {
            Some(tag) => tag,
            None => create_tag(
                db,
                NewTag {
                    site_id,
                    name: normalized,
                },
            )
            .await
            .map_err(|error| SiteError::internal(format!("failed to create tag: {error}")))?,
        };
        desired_tag_ids.push(tag.id);
    }

    let existing_content_tags = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content_id))
        .all(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to query content tags: {error}")))?;
    for existing in existing_content_tags {
        if !desired_tag_ids.contains(&existing.tag_id) {
            entities::content_tag::Entity::delete_by_id(existing.id)
                .exec(db)
                .await
                .map_err(|error| {
                    SiteError::internal(format!("failed to delete content tag: {error}"))
                })?;
        }
    }

    let existing_revision_tags = entities::content_revision_tag::Entity::find()
        .filter(entities::content_revision_tag::Column::RevisionId.eq(revision_id))
        .all(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to query revision tags: {error}")))?;
    for existing in existing_revision_tags {
        if !desired_tag_ids.contains(&existing.tag_id) {
            entities::content_revision_tag::Entity::delete_by_id(existing.id)
                .exec(db)
                .await
                .map_err(|error| {
                    SiteError::internal(format!("failed to delete revision tag: {error}"))
                })?;
        }
    }

    assign_tags_to_content(
        db,
        site_id,
        content_id,
        revision_id,
        unique.into_iter().collect(),
    )
    .await
}

/// Creates an asset record.
pub async fn create_asset<C: ConnectionTrait>(
    db: &C,
    input: NewAsset,
) -> Result<entities::asset::Model, String> {
    let model = entities::asset::ActiveModel {
        id: Set(Uuid::now_v7()),
        site_id: Set(input.site_id),
        uploader_sub: Set(input.uploader_sub),
        original_filename: Set(input.original_filename),
        storage_basename: Set(input.storage_basename),
        mime_type: Set(input.mime_type),
        byte_length: Set(input.byte_length),
        width: Set(input.width),
        height: Set(input.height),
        created_at: Set(Utc::now()),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns all assets for a site.
pub async fn list_assets(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> Result<Vec<entities::asset::Model>, SiteError> {
    entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .all(db)
        .await
        .map_err(SiteError::from)
}

/// Returns a single asset scoped to a site.
pub async fn get_asset_for_site(
    db: &DatabaseConnection,
    site_id: Uuid,
    asset_id: Uuid,
) -> Result<Option<entities::asset::Model>, SiteError> {
    entities::asset::Entity::find_by_id(asset_id)
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .one(db)
        .await
        .map_err(SiteError::from)
}

/// Creates an uploaded image asset and derivative thumbnail files under the provided upload root.
pub(crate) struct PersistedAssetFiles {
    pub byte_length: i32,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub mime_type: String,
    pub thumbnail_filename: Option<String>,
}

pub(crate) async fn persist_asset_files(
    upload_root: &Path,
    storage_basename: &str,
    bytes: Vec<u8>,
    original_filename: &str,
    mime_type: Option<String>,
) -> Result<PersistedAssetFiles, SiteError> {
    let extension = Path::new(original_filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin")
        .to_lowercase();
    let storage_path = upload_root.join(storage_basename);

    fs::create_dir_all(upload_root).await.map_err(|error| {
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

    let thumbnail_filename = if let Some(thumbnail) = thumbnail {
        let stem = Path::new(storage_basename)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("asset");
        let filename = format!("{stem}_thumb.{}", thumbnail.extension);
        let thumb_path = upload_root.join(&filename);
        fs::write(&thumb_path, &thumbnail.bytes)
            .await
            .map_err(|error| SiteError::internal(format!("failed to write thumbnail: {error}")))?;
        Some(filename)
    } else {
        None
    };

    Ok(PersistedAssetFiles {
        byte_length,
        width: width_i32,
        height: height_i32,
        mime_type,
        thumbnail_filename,
    })
}

pub async fn store_uploaded_asset<C: ConnectionTrait>(
    db: &C,
    upload_root: &Path,
    site_id: Uuid,
    uploader_sub: &str,
    bytes: Vec<u8>,
    original_filename: String,
    mime_type: Option<String>,
) -> Result<entities::asset::Model, SiteError> {
    let extension = Path::new(&original_filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin")
        .to_lowercase();
    let storage_basename = format!("{}.{}", Uuid::now_v7(), extension);
    let file_details = persist_asset_files(
        upload_root,
        &storage_basename,
        bytes,
        &original_filename,
        mime_type,
    )
    .await?;

    let asset = create_asset(
        db,
        NewAsset {
            site_id,
            uploader_sub: uploader_sub.to_string(),
            original_filename,
            storage_basename: storage_basename.clone(),
            mime_type: file_details.mime_type.clone(),
            byte_length: file_details.byte_length,
            width: file_details.width,
            height: file_details.height,
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
            mime_type: file_details.mime_type.clone(),
            byte_length: file_details.byte_length,
            width: file_details.width,
            height: file_details.height,
        },
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to create asset variant: {error}")))?;

    if let Some(filename) = file_details.thumbnail_filename {
        create_asset_variant(
            db,
            NewAssetVariant {
                asset_id: asset.id,
                variant_kind: "thumbnail".to_string(),
                filename,
                mime_type: file_details.mime_type.clone(),
                byte_length: file_details.byte_length,
                width: file_details.width,
                height: file_details.height,
            },
        )
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to create asset thumbnail: {error}"))
        })?;
    }

    Ok(asset)
}

/// Returns every stored filename used by an asset and its variants.
pub async fn collect_asset_filenames<C: ConnectionTrait>(
    db: &C,
    asset_id: Uuid,
) -> Result<Vec<String>, SiteError> {
    let asset = entities::asset::Entity::find_by_id(asset_id)
        .one(db)
        .await
        .map_err(SiteError::from)?
        .ok_or_else(|| SiteError::NotFound)?;
    let variants = list_asset_variants(db, asset_id)
        .await
        .map_err(SiteError::internal)?;

    let mut filenames = vec![asset.storage_basename];
    filenames.extend(variants.into_iter().map(|variant| variant.filename));
    filenames.sort();
    filenames.dedup();
    Ok(filenames)
}

/// Creates an asset variant entry.
pub async fn create_asset_variant<C: ConnectionTrait>(
    db: &C,
    input: NewAssetVariant,
) -> Result<entities::asset_variant::Model, String> {
    let model = entities::asset_variant::ActiveModel {
        id: Set(Uuid::now_v7()),
        asset_id: Set(input.asset_id),
        variant_kind: Set(input.variant_kind),
        filename: Set(input.filename),
        mime_type: Set(input.mime_type),
        byte_length: Set(input.byte_length),
        width: Set(input.width),
        height: Set(input.height),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns all variants for an asset.
pub async fn list_asset_variants<C: ConnectionTrait>(
    db: &C,
    asset_id: Uuid,
) -> Result<Vec<entities::asset_variant::Model>, String> {
    let variants = entities::asset_variant::Entity::find()
        .filter(entities::asset_variant::Column::AssetId.eq(asset_id))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(variants)
}

/// Deletes an asset record scoped to a site.
pub async fn delete_asset<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    asset_id: Uuid,
) -> Result<(), SiteError> {
    let existing = entities::asset::Entity::find_by_id(asset_id)
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .one(db)
        .await
        .map_err(SiteError::from)?;
    if existing.is_none() {
        return Err(SiteError::NotFound);
    }

    entities::asset::Entity::delete_by_id(asset_id)
        .exec(db)
        .await
        .map_err(SiteError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_db_start;
    use crate::entities::audit_event::log_audit_event;
    use crate::entities::site::get_by_id;
    use crate::entities::user::upsert_user_login;
    use chrono::TimeZone;
    use sea_orm::{DatabaseConnection, TransactionTrait};
    use tempfile::TempDir;
    use tokio::fs;

    /// test function to create a site with default values for testing
    async fn create_site_fixture(db: &DatabaseConnection) -> entities::site::Model {
        create_site(
            db,
            "alpha".to_string(),
            "Alpha Site".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create site")
    }

    async fn create_site_with_template_fixture(
        db: &DatabaseConnection,
        short_name: &str,
        template_name: &str,
    ) -> entities::site::Model {
        create_site(
            db,
            short_name.to_string(),
            format!("{short_name} site"),
            template_name.to_string(),
        )
        .await
        .expect("failed to create site")
    }

    /// test function to create content with default values for testing
    async fn create_content_fixture(
        db: &DatabaseConnection,
        site_id: Uuid,
        page_type: PageType,
        slug: &str,
        draft: bool,
    ) -> entities::content_item::Model {
        create_content(
            db,
            NewContent {
                site_id,
                page_type,
                title: "Hello World".to_string(),
                slug: slug.to_string(),
                page_content: "Body".to_string(),
                draft,
                creator_sub: "creator".to_string(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create content")
    }

    #[tokio::test]
    async fn list_audit_events_filters_by_site() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;

        log_audit_event(&db, "actor", "create", "site", "1", Some(site.id), None)
            .await
            .expect("failed to log site audit event");
        log_audit_event(&db, "actor", "login", "user", "2", None, None)
            .await
            .expect("failed to log global audit event");

        let scoped = list_audit_events(&db, Some(site.id))
            .await
            .expect("failed to list scoped audit events");
        assert_eq!(scoped.len(), 1);

        let all = list_audit_events(&db, None)
            .await
            .expect("failed to list all audit events");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn create_list_and_get_sites() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;

        let sites = list_sites(&db).await.expect("failed to list sites");
        assert!(sites.iter().any(|model| model.id == site.id));

        let fetched = get_by_id(&db, site.id).await.expect("failed to get site");
        assert_eq!(fetched.id, site.id);
    }

    #[tokio::test]
    async fn update_site_settings_updates_values() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;

        let updated = update_site_settings(
            &db,
            site.id,
            "Updated Title".to_string(),
            "new-template".to_string(),
            true,
        )
        .await
        .expect("failed to update site settings");

        assert_eq!(updated.full_title, "Updated Title");
        assert_eq!(updated.template_name, "new-template");
        assert!(updated.publish_on_render);
        assert!(updated.updated_at.is_some());
    }

    #[tokio::test]
    async fn delete_site_removes_row() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;

        delete_site(&db, site.id)
            .await
            .expect("failed to delete site");

        let deleted = entities::site::Entity::find_by_id(site.id)
            .one(&db)
            .await
            .expect("failed to reload deleted site");
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn transaction_rolls_back_on_error() {
        let db = test_db_start().await;
        let txn = db.begin().await.expect("failed to start transaction");

        let site_1 = create_site(
            &txn,
            "dupe".to_string(),
            "Dupe Site".to_string(),
            "default".to_string(),
        )
        .await;
        let site_2 = create_site(
            &txn,
            "dupe".to_string(),
            "Dupe Site 2".to_string(),
            "default".to_string(),
        )
        .await;
        assert!(site_1.is_ok(), "expected first site creation to succeed");
        assert!(
            site_2.is_err(),
            "expected second site creation to fail with duplicate short name"
        );
        drop(txn);

        let sites = list_sites(&db).await.expect("failed to list sites");
        assert!(sites.is_empty(), "expected no sites after rollback");
    }

    #[tokio::test]
    async fn content_lifecycle_and_revisions() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;

        let content = create_content_fixture(&db, site.id, PageType::Post, "hello", true).await;

        let all_content = list_content(&db, site.id, None)
            .await
            .expect("failed to list content");
        assert_eq!(all_content.len(), 1);

        let post_content = list_content(&db, site.id, Some(PageType::Post))
            .await
            .expect("failed to list post content");
        assert_eq!(post_content.len(), 1);

        let page_content = list_content(&db, site.id, Some(PageType::Page))
            .await
            .expect("failed to list page content");
        assert_eq!(page_content.len(), 0);

        // using case-insensitive search to verify that the search is normalized for consistent results
        let search = search_content(&db, site.id, "Hello")
            .await
            .expect("failed to search content");
        assert_eq!(search.len(), 1);

        let search = search_content(&db, site.id, "hello")
            .await
            .expect("failed to search content");
        assert_eq!(search.len(), 1);

        let fetched = entities::content_item::Entity::find_by_id(content.id)
            .filter(entities::content_item::Column::SiteId.eq(site.id))
            .one(&db)
            .await
            .expect("failed to fetch content by id")
            .expect("content not found");
        assert_eq!(fetched.id, content.id);

        let alias = create_alias(
            &db,
            NewAlias {
                content_id: content.id,
                site_id: site.id,
                alias_path: "/legacy/hello".to_string(),
                kind: "alias".to_string(),
            },
        )
        .await
        .expect("failed to create alias");

        let tag_link = add_content_tag(
            &db,
            NewContentTag {
                content_id: content.id,
                site_id: site.id,
                tag_name: "News".to_string(),
            },
        )
        .await
        .expect("failed to add content tag");

        let tag_names = load_tag_names(&db, content.id)
            .await
            .expect("failed to load tag names");
        assert_eq!(tag_names, vec!["News".to_string()]);

        let updated = update_content(
            &db,
            UpdateContent {
                content_id: content.id,
                page_type: None,
                title: Some("Hello Updated".to_string()),
                slug: Some("hello-updated".to_string()),
                page_content: Some("Updated".to_string()),
                draft: Some(false),
                published_at: None,
                editor_sub: "editor".to_string(),
            },
        )
        .await
        .expect("failed to update content");
        assert_eq!(updated.title, "Hello Updated");

        let aliases = list_aliases(&db, site.id, Some(content.id))
            .await
            .expect("failed to list aliases");
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].id, alias.id);

        let content_tags = list_content_tags(&db, content.id)
            .await
            .expect("failed to list content tags");
        assert_eq!(content_tags.len(), 1);
        assert_eq!(content_tags[0].id, tag_link.tag_id);

        let revisions = list_revisions(&db, content.id)
            .await
            .expect("failed to list revisions");
        assert_eq!(revisions.len(), 2);

        let latest = revisions.first().expect("missing revision entry").clone();
        let fetched_revision = get_revision(&db, latest.id)
            .await
            .expect("failed to get revision");
        assert_eq!(fetched_revision.id, latest.id);

        let rev1 = get_revision_by_number(&db, content.id, 1)
            .await
            .expect("failed to fetch revision 1");
        assert!(rev1.is_some());
        let rev2 = get_revision_by_number(&db, content.id, 2)
            .await
            .expect("failed to fetch revision 2");
        assert!(rev2.is_some());

        let revision_aliases = list_revision_aliases(&db, latest.id)
            .await
            .expect("failed to list revision aliases");
        assert_eq!(revision_aliases.len(), 1);
        assert_eq!(revision_aliases[0].content_id, content.id);

        let revision_tags = list_revision_tags(&db, latest.id)
            .await
            .expect("failed to list revision tags");
        assert_eq!(revision_tags.len(), 1);
        let revision_tag_links = entities::content_revision_tag::Entity::find()
            .filter(entities::content_revision_tag::Column::RevisionId.eq(latest.id))
            .all(&db)
            .await
            .expect("failed to load revision tag links");
        assert_eq!(revision_tag_links.len(), 1);
        assert_eq!(revision_tag_links[0].content_id, content.id);
    }

    #[tokio::test]
    async fn search_all_content_returns_matches_from_multiple_sites() {
        let db = test_db_start().await;
        let site_one = create_site_fixture(&db).await;
        let site_two = create_site_with_template_fixture(&db, "beta", "default").await;

        create_content(
            &db,
            NewContent {
                site_id: site_one.id,
                page_type: PageType::Page,
                title: "Alpha Match".to_string(),
                slug: "alpha-match".to_string(),
                page_content: "shared needle".to_string(),
                draft: true,
                creator_sub: "creator".to_string(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create site one content");
        create_content(
            &db,
            NewContent {
                site_id: site_two.id,
                page_type: PageType::Page,
                title: "Beta Match".to_string(),
                slug: "beta-match".to_string(),
                page_content: "shared needle".to_string(),
                draft: true,
                creator_sub: "creator".to_string(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create site two content");

        let results = search_all_content(&db, "shared needle")
            .await
            .expect("failed to search across all content");

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|item| item.site_id == site_one.id));
        assert!(results.iter().any(|item| item.site_id == site_two.id));
    }

    #[test]
    fn content_primary_route_uses_slug_for_pages_and_date_prefix_for_posts() {
        let published_at = Utc
            .with_ymd_and_hms(2026, 3, 8, 12, 0, 0)
            .single()
            .expect("invalid published date");
        let created_at = Utc
            .with_ymd_and_hms(2026, 3, 7, 9, 30, 0)
            .single()
            .expect("invalid created date");

        let page = entities::content_item::Model {
            id: Uuid::now_v7(),
            site_id: Uuid::now_v7(),
            page_type: PageType::Page,
            title: "Page".to_string(),
            slug: "collegia-notes".to_string(),
            page_content: "Body".to_string(),
            draft: false,
            creator_sub: "creator".to_string(),
            created_at,
            last_updated: None,
            published_at: Some(published_at),
        };
        let post = entities::content_item::Model {
            id: Uuid::now_v7(),
            site_id: Uuid::now_v7(),
            page_type: PageType::Post,
            title: "Post".to_string(),
            slug: "collegia-notes".to_string(),
            page_content: "Body".to_string(),
            draft: false,
            creator_sub: "creator".to_string(),
            created_at,
            last_updated: None,
            published_at: Some(published_at),
        };

        assert_eq!(content_primary_route(&page), "collegia-notes");
        assert_eq!(content_primary_route(&post), "2026/03/08/collegia-notes");
    }

    #[tokio::test]
    async fn create_and_list_tags() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;

        let tag = create_tag(
            &db,
            NewTag {
                site_id: site.id,
                name: "Docs".to_string(),
            },
        )
        .await
        .expect("failed to create tag");

        let tags = list_tags(&db, site.id).await.expect("failed to list tags");
        assert!(tags.iter().any(|model| model.id == tag.id));

        let content = create_content_fixture(&db, site.id, PageType::Post, "docs", true).await;
        let _ = add_content_tag(
            &db,
            NewContentTag {
                content_id: content.id,
                site_id: site.id,
                tag_name: "Docs".to_string(),
            },
        )
        .await
        .expect("failed to add tag to content");

        let content_tags = list_content_tags(&db, content.id)
            .await
            .expect("failed to list content tags");
        assert_eq!(content_tags.len(), 1);
    }

    #[tokio::test]
    async fn sync_tags_to_content_replaces_current_tag_set_for_latest_revision() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let content = create_content_fixture(&db, site.id, PageType::Post, "tag-sync", true).await;
        let rev1 = get_revision_by_number(&db, content.id, 1)
            .await
            .expect("failed to fetch revision 1")
            .expect("missing revision 1");

        assign_tags_to_content(
            &db,
            site.id,
            content.id,
            rev1.id,
            vec!["Docs".to_string(), "News".to_string()],
        )
        .await
        .expect("failed to assign initial tags");
        let revision_tag_links = entities::content_revision_tag::Entity::find()
            .filter(entities::content_revision_tag::Column::RevisionId.eq(rev1.id))
            .all(&db)
            .await
            .expect("failed to load initial revision tag links");
        assert_eq!(revision_tag_links.len(), 2);
        assert!(
            revision_tag_links
                .iter()
                .all(|link| link.content_id == content.id)
        );

        update_content(
            &db,
            UpdateContent {
                content_id: content.id,
                page_type: None,
                title: Some("Tag Sync Updated".to_string()),
                slug: Some("tag-sync-updated".to_string()),
                page_content: Some("Updated body".to_string()),
                draft: Some(false),
                published_at: None,
                editor_sub: "editor".to_string(),
            },
        )
        .await
        .expect("failed to update content");

        let rev2 = get_revision_by_number(&db, content.id, 2)
            .await
            .expect("failed to fetch revision 2")
            .expect("missing revision 2");

        sync_tags_to_content(
            &db,
            site.id,
            content.id,
            rev2.id,
            vec!["News".to_string(), "Guides".to_string()],
        )
        .await
        .expect("failed to sync tags");

        let mut current_tag_names = list_content_tags(&db, content.id)
            .await
            .expect("failed to list content tags")
            .into_iter()
            .map(|tag| tag.name)
            .collect::<Vec<_>>();
        current_tag_names.sort();
        assert_eq!(
            current_tag_names,
            vec!["Guides".to_string(), "News".to_string()]
        );

        let mut rev2_tag_names = list_revision_tags(&db, rev2.id)
            .await
            .expect("failed to list revision 2 tags")
            .into_iter()
            .map(|tag| tag.name)
            .collect::<Vec<_>>();
        rev2_tag_names.sort();
        assert_eq!(
            rev2_tag_names,
            vec!["Guides".to_string(), "News".to_string()]
        );

        let mut rev1_tag_names = list_revision_tags(&db, rev1.id)
            .await
            .expect("failed to list revision 1 tags")
            .into_iter()
            .map(|tag| tag.name)
            .collect::<Vec<_>>();
        rev1_tag_names.sort();
        assert_eq!(rev1_tag_names, vec!["Docs".to_string(), "News".to_string()]);
    }

    #[tokio::test]
    async fn users_and_memberships() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;

        let user = upsert_user_login(&db, "alice", None, Some("Alice"))
            .await
            .expect("failed to create user");

        let updated = upsert_user_login(
            &db,
            "alice",
            Some("alice@example.com"),
            Some("Alice Example"),
        )
        .await
        .expect("failed to upsert user login");
        assert_eq!(updated.id, user.id);
        assert!(updated.admin);

        assert!(updated.last_login_at.is_some());
        assert_eq!(updated.display_name.as_deref(), Some("Alice Example"));

        let bob = upsert_user_login(&db, "bob", None, None)
            .await
            .expect("failed to insert user login");

        assert!(!bob.admin);

        let users = list_users(&db).await.expect("failed to list users");
        assert_eq!(users.len(), 2);

        let membership = create_membership(
            &db,
            NewMembership {
                site_id: site.id,
                user_id: user.id,
                role: SiteRole::Owner,
            },
        )
        .await
        .expect("failed to create membership");

        let memberships = list_memberships(&db, site.id)
            .await
            .expect("failed to list memberships");
        assert!(memberships.iter().any(|model| model.id == membership.id));

        let fetched_user = get_user_by_id(&db, user.id)
            .await
            .expect("failed to fetch user by id");
        assert_eq!(fetched_user.map(|value| value.id), Some(user.id));

        let user_memberships = list_memberships_for_user_id(&db, user.id)
            .await
            .expect("failed to list memberships by user id");
        assert_eq!(user_memberships.len(), 1);
        assert_eq!(user_memberships[0].id, membership.id);

        let admin_membership = create_membership(
            &db,
            NewMembership {
                site_id: site.id,
                user_id: bob.id,
                role: SiteRole::Admin,
            },
        )
        .await;
        assert!(admin_membership.is_err());
    }

    #[tokio::test]
    async fn assets_variants_and_copy_media() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;

        let asset = create_asset(
            &db,
            NewAsset {
                site_id: site.id,
                uploader_sub: "uploader".to_string(),
                original_filename: "photo.jpg".to_string(),
                storage_basename: "asset.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                byte_length: 10,
                width: Some(100),
                height: Some(200),
            },
        )
        .await
        .expect("failed to create asset");

        let variant = create_asset_variant(
            &db,
            NewAssetVariant {
                asset_id: asset.id,
                variant_kind: "thumbnail".to_string(),
                filename: "asset-thumb.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                byte_length: 5,
                width: Some(50),
                height: Some(50),
            },
        )
        .await
        .expect("failed to create asset variant");

        let assets = list_assets(&db, site.id)
            .await
            .expect("failed to list assets");
        assert_eq!(assets.len(), 1);

        let variants = list_asset_variants(&db, asset.id)
            .await
            .expect("failed to list asset variants");
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].id, variant.id);

        let source_dir = TempDir::new().expect("failed to create temp source dir");
        let dest_dir = TempDir::new().expect("failed to create temp dest dir");
        let source_root = source_dir.path();
        let dest_root = dest_dir.path();

        fs::create_dir_all(source_root)
            .await
            .expect("failed to create source root");
        fs::write(source_root.join("asset.jpg"), b"asset data")
            .await
            .expect("failed to write asset file");
        fs::write(source_root.join("asset-thumb.jpg"), b"thumb data")
            .await
            .expect("failed to write variant file");

        let mut files_written = 0usize;
        copy_media_variants(&db, site.id, source_root, dest_root, &mut files_written)
            .await
            .expect("failed to copy media variants");

        assert_eq!(files_written, 2);
        assert!(fs::metadata(dest_root.join("asset.jpg")).await.is_ok());
        assert!(
            fs::metadata(dest_root.join("asset-thumb.jpg"))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn import_wordpress_creates_content_and_aliases() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let file_path = temp_dir.path().join("import.xml");
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Imported Post</title>
      <link>https://example.com/2020/01/imported-post/?p=123</link>
      <wp:post_id>123</wp:post_id>
      <wp:post_name>imported-post</wp:post_name>
      <wp:post_type>post</wp:post_type>
      <wp:status>publish</wp:status>
      <wp:post_date>2025-01-02 13:04:05</wp:post_date>
      <wp:post_date_gmt>2025-01-02 03:04:05</wp:post_date_gmt>
      <wp:post_modified>2025-01-03 14:05:06</wp:post_modified>
      <wp:post_modified_gmt>2025-01-03 04:05:06</wp:post_modified_gmt>
      <content:encoded><![CDATA[Hello world]]></content:encoded>
    </item>
  </channel>
</rss>
"#;

        fs::write(&file_path, xml.as_bytes())
            .await
            .expect("failed to write import file");

        let imported = import_wordpress(
            &db,
            site.id,
            file_path.to_str().expect("invalid path"),
            "creator",
        )
        .await
        .expect("failed to import wordpress data");
        assert_eq!(imported, 1);

        let content = list_content(&db, site.id, None)
            .await
            .expect("failed to list content");
        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0].created_at,
            Utc.with_ymd_and_hms(2025, 1, 2, 3, 4, 5)
                .single()
                .expect("invalid expected created_at")
        );
        assert_eq!(
            content[0].last_updated,
            Some(
                Utc.with_ymd_and_hms(2025, 1, 3, 4, 5, 6)
                    .single()
                    .expect("invalid expected last_updated")
            )
        );
        assert_eq!(content[0].published_at, Some(content[0].created_at));

        let revisions = list_revisions(&db, content[0].id)
            .await
            .expect("failed to list revisions");
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].created_at, content[0].created_at);

        let aliases = list_aliases(&db, site.id, None)
            .await
            .expect("failed to list aliases");
        assert_eq!(aliases.len(), 2);
    }

    #[tokio::test]
    async fn import_wordpress_falls_back_to_non_gmt_timestamps() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let file_path = temp_dir.path().join("import.xml");
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Imported Page</title>
      <link>https://example.com/imported-page/</link>
      <wp:post_id>124</wp:post_id>
      <wp:post_name>imported-page</wp:post_name>
      <wp:post_type>page</wp:post_type>
      <wp:status>publish</wp:status>
      <wp:post_date>2025-02-03 04:05:06</wp:post_date>
      <wp:post_modified>2025-02-04 05:06:07</wp:post_modified>
      <content:encoded><![CDATA[Hello page]]></content:encoded>
    </item>
  </channel>
</rss>
"#;

        fs::write(&file_path, xml.as_bytes())
            .await
            .expect("failed to write import file");

        let imported = import_wordpress(
            &db,
            site.id,
            file_path.to_str().expect("invalid path"),
            "creator",
        )
        .await
        .expect("failed to import wordpress data");
        assert_eq!(imported, 1);

        let content = list_content(&db, site.id, Some(PageType::Page))
            .await
            .expect("failed to list imported page content");
        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0].created_at,
            Utc.with_ymd_and_hms(2025, 2, 3, 4, 5, 6)
                .single()
                .expect("invalid expected created_at")
        );
        assert_eq!(
            content[0].last_updated,
            Some(
                Utc.with_ymd_and_hms(2025, 2, 4, 5, 6, 7)
                    .single()
                    .expect("invalid expected last_updated")
            )
        );
        assert_eq!(content[0].published_at, Some(content[0].created_at));
    }

    #[tokio::test]
    async fn import_wordpress_skips_unsupported_item_types() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let file_path = temp_dir.path().join("import.xml");
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Imported Post</title>
      <link>https://example.com/2020/01/imported-post/?p=123</link>
      <wp:post_id>123</wp:post_id>
      <wp:post_name>imported-post</wp:post_name>
      <wp:post_type>post</wp:post_type>
      <wp:status>publish</wp:status>
      <content:encoded><![CDATA[Hello world]]></content:encoded>
    </item>
    <item>
      <title>Imported Page</title>
      <link>https://example.com/about/</link>
      <wp:post_id>456</wp:post_id>
      <wp:post_name>about</wp:post_name>
      <wp:post_type>page</wp:post_type>
      <wp:status>publish</wp:status>
      <content:encoded><![CDATA[About page]]></content:encoded>
    </item>
    <item>
      <title>Header Image</title>
      <link>https://example.com/wp-content/uploads/header.jpg</link>
      <wp:post_id>789</wp:post_id>
      <wp:post_name>header-jpg</wp:post_name>
      <wp:post_type>attachment</wp:post_type>
      <wp:status>inherit</wp:status>
      <content:encoded><![CDATA[]]></content:encoded>
    </item>
    <item>
      <title></title>
      <link>https://example.com/?p=999</link>
      <wp:post_id>999</wp:post_id>
      <wp:status>publish</wp:status>
      <content:encoded><![CDATA[]]></content:encoded>
    </item>
  </channel>
</rss>
"#;

        fs::write(&file_path, xml.as_bytes())
            .await
            .expect("failed to write import file");

        let imported = import_wordpress(
            &db,
            site.id,
            file_path.to_str().expect("invalid path"),
            "creator",
        )
        .await
        .expect("failed to import wordpress data");
        assert_eq!(imported, 2);

        let content = list_content(&db, site.id, None)
            .await
            .expect("failed to list content");
        assert_eq!(content.len(), 2);
        assert!(content.iter().any(|item| item.title == "Imported Post"));
        assert!(content.iter().any(|item| item.title == "Imported Page"));
    }

    #[tokio::test]
    async fn import_wordpress_skips_duplicates_on_repeat_imports() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let file_path = temp_dir.path().join("import.xml");
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Imported Post</title>
      <link>https://example.com/2020/01/imported-post/?p=123</link>
      <wp:post_id>123</wp:post_id>
      <wp:post_name>imported-post</wp:post_name>
      <wp:post_type>post</wp:post_type>
      <wp:status>publish</wp:status>
      <content:encoded><![CDATA[Hello world]]></content:encoded>
    </item>
  </channel>
</rss>
"#;

        fs::write(&file_path, xml.as_bytes())
            .await
            .expect("failed to write import file");

        let imported_first = import_wordpress(
            &db,
            site.id,
            file_path.to_str().expect("invalid path"),
            "creator",
        )
        .await
        .expect("failed to import wordpress data the first time");
        let imported_second = import_wordpress(
            &db,
            site.id,
            file_path.to_str().expect("invalid path"),
            "creator",
        )
        .await
        .expect("failed to import wordpress data the second time");

        assert_eq!(imported_first, 1);
        assert_eq!(imported_second, 0);

        let content = list_content(&db, site.id, None)
            .await
            .expect("failed to list content");
        assert_eq!(content.len(), 1);

        let aliases = list_aliases(&db, site.id, None)
            .await
            .expect("failed to list aliases");
        assert_eq!(aliases.len(), 2);
    }

    #[tokio::test]
    async fn import_wordpress_creates_tags_from_post_tag_categories() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let file_path = temp_dir.path().join("import.xml");
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Tagged Post</title>
      <link>https://example.com/tagged-post/</link>
      <wp:post_id>125</wp:post_id>
      <wp:post_name>tagged-post</wp:post_name>
      <wp:post_type>post</wp:post_type>
      <wp:status>publish</wp:status>
      <category domain="category" nicename="news"><![CDATA[News]]></category>
      <category domain="post_tag" nicename="ivf"><![CDATA[ivf]]></category>
      <category domain="post_tag" nicename="fertility"><![CDATA[Fertility]]></category>
      <category domain="post_tag" nicename="ivf"><![CDATA[ivf]]></category>
      <content:encoded><![CDATA[Tagged content]]></content:encoded>
    </item>
  </channel>
</rss>
"#;

        fs::write(&file_path, xml.as_bytes())
            .await
            .expect("failed to write import file");

        let imported = import_wordpress(
            &db,
            site.id,
            file_path.to_str().expect("invalid path"),
            "creator",
        )
        .await
        .expect("failed to import wordpress data");
        assert_eq!(imported, 1);

        let content = list_content(&db, site.id, Some(PageType::Post))
            .await
            .expect("failed to list imported content");
        assert_eq!(content.len(), 1);

        let mut content_tag_names = list_content_tags(&db, content[0].id)
            .await
            .expect("failed to list content tags")
            .into_iter()
            .map(|tag| tag.name)
            .collect::<Vec<_>>();
        content_tag_names.sort();
        assert_eq!(
            content_tag_names,
            vec!["Fertility".to_string(), "ivf".to_string()]
        );

        let revision = get_revision_by_number(&db, content[0].id, 1)
            .await
            .expect("failed to fetch initial revision")
            .expect("missing initial revision");
        let mut revision_tag_names = list_revision_tags(&db, revision.id)
            .await
            .expect("failed to list revision tags")
            .into_iter()
            .map(|tag| tag.name)
            .collect::<Vec<_>>();
        revision_tag_names.sort();
        assert_eq!(
            revision_tag_names,
            vec!["Fertility".to_string(), "ivf".to_string()]
        );
    }

    #[tokio::test]
    async fn import_wordpress_deduplicates_equivalent_aliases_per_item() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let file_path = temp_dir.path().join("import.xml");
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Imported Post</title>
      <link>https://example.com/?p=123</link>
      <wp:post_id>123</wp:post_id>
      <wp:post_name>imported-post</wp:post_name>
      <wp:post_type>post</wp:post_type>
      <wp:status>publish</wp:status>
      <content:encoded><![CDATA[Hello world]]></content:encoded>
    </item>
  </channel>
</rss>
"#;

        fs::write(&file_path, xml.as_bytes())
            .await
            .expect("failed to write import file");

        let imported = import_wordpress(
            &db,
            site.id,
            file_path.to_str().expect("invalid path"),
            "creator",
        )
        .await
        .expect("failed to import wordpress data");
        assert_eq!(imported, 1);

        let aliases = list_aliases(&db, site.id, None)
            .await
            .expect("failed to list aliases");
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].alias_path, "/?p=123");
    }

    #[tokio::test]
    async fn render_site_outputs_files() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let rendered_dir = TempDir::new().expect("failed to create rendered dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");

        let content = create_content_fixture(&db, site.id, PageType::Post, "hello", false).await;
        let _ = add_content_tag(
            &db,
            NewContentTag {
                content_id: content.id,
                site_id: site.id,
                tag_name: "Alpha Tag".to_string(),
            },
        )
        .await
        .expect("failed to tag content");

        let content_route = content_primary_route(&content);

        let files_written = render_site(
            &db,
            site.id,
            templates_dir.path(),
            rendered_dir.path(),
            upload_root.path(),
        )
        .await
        .expect("failed to render site");
        assert!(files_written >= 4);

        let rendered_root = rendered_dir.path().join(site.short_name);
        assert!(fs::metadata(rendered_root.join("index.html")).await.is_ok());
        assert!(fs::metadata(rendered_root.join("rss.xml")).await.is_ok());
        assert!(fs::metadata(rendered_root.join("atom.xml")).await.is_ok());
        assert!(
            fs::metadata(rendered_root.join(content_route).join("index.html"))
                .await
                .is_ok()
        );
        assert!(
            fs::metadata(
                rendered_root
                    .join("tags")
                    .join("alpha-tag")
                    .join("index.html")
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn render_content_preview_uses_site_template_files() {
        let db = test_db_start().await;
        let site = create_site_with_template_fixture(&db, "preview-site", "custom-preview").await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");
        let template_root = templates_dir.path().join("custom-preview");
        fs::create_dir_all(&template_root)
            .await
            .expect("failed to create template root");
        fs::write(
            template_root.join("base_template.html"),
            r#"<!doctype html><html><body><div class="site-shell">{% block content %}{% endblock %}</div></body></html>"#,
        )
        .await
        .expect("failed to write base template");
        fs::write(
            template_root.join("page.html"),
            r#"{% extends "base_template.html" %}{% block content %}<article data-template="custom-preview">{{title}}::{{content}}</article>{% endblock %}"#,
        )
        .await
        .expect("failed to write page template");

        let content =
            create_content_fixture(&db, site.id, PageType::Page, "preview-page", true).await;
        let rendered = render_content_preview(
            &db,
            site.id,
            content.id,
            templates_dir.path().into(),
            upload_root.path(),
        )
        .await
        .expect("failed to render preview");

        assert!(rendered.contains("data-template=\"custom-preview\""));
        assert!(rendered.contains("site-shell"));
    }

    #[tokio::test]
    async fn render_content_preview_prefers_site_template_overrides() {
        let db = test_db_start().await;
        let site =
            create_site_with_template_fixture(&db, "override-preview", "custom-preview").await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let override_root = TempDir::new().expect("failed to create override dir");
        let template_root = templates_dir.path().join("custom-preview");
        fs::create_dir_all(&template_root)
            .await
            .expect("failed to create template root");
        fs::write(
            template_root.join("base_template.html"),
            r#"<!doctype html><html><body>{% block content %}{% endblock %}</body></html>"#,
        )
        .await
        .expect("failed to write base template");
        fs::write(
            template_root.join("page.html"),
            r#"{% extends "base_template.html" %}{% block content %}<article data-template="shared">{{title}}</article>{% endblock %}"#,
        )
        .await
        .expect("failed to write shared page template");
        fs::write(
            override_root.path().join("page.html"),
            r#"{% extends "base_template.html" %}{% block content %}<article data-template="override">{{title}}</article>{% endblock %}"#,
        )
        .await
        .expect("failed to write override page template");

        let content =
            create_content_fixture(&db, site.id, PageType::Page, "override-page", true).await;
        let rendered = render_content_preview_with_overrides(
            &db,
            site.id,
            content.id,
            templates_dir.path().into(),
            override_root.path(),
        )
        .await
        .expect("failed to render preview");

        assert!(rendered.contains("data-template=\"override\""));
        assert!(!rendered.contains("data-template=\"shared\""));
    }

    #[tokio::test]
    async fn render_content_preview_uses_bundled_default_template_when_configured_root_is_empty() {
        let _env_lock = crate::test_support::env_lock().lock_owned().await;
        let original = env::var_os("WEBSITES_SITE_TEMPLATES_DIR");
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");

        unsafe {
            env::set_var("WEBSITES_SITE_TEMPLATES_DIR", templates_dir.path());
        }

        let content =
            create_content_fixture(&db, site.id, PageType::Page, "bundled-default-page", true)
                .await;
        let rendered = render_content_preview(
            &db,
            site.id,
            content.id,
            templates_dir.path().into(),
            upload_root.path(),
        )
        .await
        .expect("failed to render preview");

        assert!(rendered.contains(r#"<link rel="stylesheet" href="/assets/style.css" />"#));
        assert!(rendered.contains(r#"<section class="content"><p>Body</p></section>"#));

        match original {
            Some(value) => unsafe {
                env::set_var("WEBSITES_SITE_TEMPLATES_DIR", value);
            },
            None => unsafe {
                env::remove_var("WEBSITES_SITE_TEMPLATES_DIR");
            },
        }
    }

    #[tokio::test]
    async fn render_content_preview_uses_bundled_default_template_outside_repo_cwd() {
        let _env_lock = crate::test_support::env_lock().lock_owned().await;
        let original_cwd = env::current_dir().expect("failed to capture current dir");
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");
        let working_dir = TempDir::new().expect("failed to create working dir");

        env::set_current_dir(working_dir.path()).expect("failed to change current dir");

        let content = create_content_fixture(
            &db,
            site.id,
            PageType::Page,
            "bundled-default-off-cwd",
            true,
        )
        .await;
        let render_result = render_content_preview(
            &db,
            site.id,
            content.id,
            templates_dir.path().into(),
            upload_root.path(),
        )
        .await;

        env::set_current_dir(&original_cwd).expect("failed to restore current dir");

        let rendered = render_result.expect("failed to render preview");
        assert!(rendered.contains(r#"<link rel="stylesheet" href="/assets/style.css" />"#));
        assert!(rendered.contains(r#"<section class="content"><p>Body</p></section>"#));
    }

    #[tokio::test]
    async fn render_content_preview_renders_raw_html_from_markdown() {
        let db = test_db_start().await;
        let site = create_site_with_template_fixture(&db, "html-preview", "html-preview").await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");
        let template_root = templates_dir.path().join("html-preview");
        fs::create_dir_all(&template_root)
            .await
            .expect("failed to create template root");
        fs::write(
            template_root.join("base_template.html"),
            r#"<!doctype html><html><body>{% block content %}{% endblock %}</body></html>"#,
        )
        .await
        .expect("failed to write base template");
        fs::write(
            template_root.join("page.html"),
            r#"{% extends "base_template.html" %}{% block content %}<article>{{content}}</article>{% endblock %}"#,
        )
        .await
        .expect("failed to write page template");

        let content = create_content(
            &db,
            NewContent {
                site_id: site.id,
                page_type: PageType::Page,
                title: "HTML Preview".to_string(),
                slug: "html-preview".to_string(),
                page_content: r#"Before <span class="inline-html">inline html</span> after"#
                    .to_string(),
                draft: true,
                creator_sub: "creator".to_string(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create content");

        let rendered = render_content_preview(
            &db,
            site.id,
            content.id,
            templates_dir.path().into(),
            upload_root.path(),
        )
        .await
        .expect("failed to render preview");

        assert!(rendered.contains(r#"<span class="inline-html">inline html</span>"#));
        assert!(!rendered.contains("&lt;span"));
    }

    #[tokio::test]
    async fn render_content_preview_expands_asset_shortcode_into_figure() {
        let db = test_db_start().await;
        let site = create_site_with_template_fixture(&db, "asset-preview", "asset-preview").await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");
        let template_root = templates_dir.path().join("asset-preview");
        fs::create_dir_all(&template_root)
            .await
            .expect("failed to create template root");
        fs::write(
            template_root.join("base_template.html"),
            r#"<!doctype html><html><body>{% block content %}{% endblock %}</body></html>"#,
        )
        .await
        .expect("failed to write base template");
        fs::write(
            template_root.join("page.html"),
            r#"{% extends "base_template.html" %}{% block content %}<article>{{content}}</article>{% endblock %}"#,
        )
        .await
        .expect("failed to write page template");

        let asset = create_asset(
            &db,
            NewAsset {
                site_id: site.id,
                uploader_sub: "creator".to_string(),
                original_filename: "preview-image.png".to_string(),
                storage_basename: "preview-image.png".to_string(),
                mime_type: "image/png".to_string(),
                byte_length: 10,
                width: Some(640),
                height: Some(480),
            },
        )
        .await
        .expect("failed to create asset");
        create_asset_variant(
            &db,
            NewAssetVariant {
                asset_id: asset.id,
                variant_kind: "thumbnail".to_string(),
                filename: "preview-image_thumb.png".to_string(),
                mime_type: "image/png".to_string(),
                byte_length: 5,
                width: Some(320),
                height: Some(240),
            },
        )
        .await
        .expect("failed to create thumbnail variant");

        let content = create_content(
            &db,
            NewContent {
                site_id: site.id,
                page_type: PageType::Page,
                title: "Asset Preview".to_string(),
                slug: "asset-preview".to_string(),
                page_content: format!(
                    "Before [[asset id=\"{}\" variant=\"thumbnail\" alt=\"Preview alt\" title=\"Preview caption\"]] after",
                    asset.id
                ),
                draft: true,
                creator_sub: "creator".to_string(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create content");

        let rendered = render_content_preview(
            &db,
            site.id,
            content.id,
            templates_dir.path().into(),
            upload_root.path(),
        )
        .await
        .expect("failed to render preview");

        assert!(rendered.contains("<figure><img"));
        assert!(rendered.contains("src=\"/media/images/preview-image_thumb.png\""));
        assert!(rendered.contains("<figcaption>Preview caption</figcaption>"));
    }

    #[tokio::test]
    async fn render_site_supports_template_inheritance() {
        let db = test_db_start().await;
        let site = create_site_with_template_fixture(&db, "render-site", "custom-render").await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let rendered_dir = TempDir::new().expect("failed to create rendered dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");
        let template_root = templates_dir.path().join("custom-render");
        fs::create_dir_all(&template_root)
            .await
            .expect("failed to create template root");
        fs::write(
            template_root.join("base_template.html"),
            r#"<!doctype html><html><body><div class="shell">{% block content %}{% endblock %}</div></body></html>"#,
        )
        .await
        .expect("failed to write base template");
        fs::write(
            template_root.join("post.html"),
            r#"{% extends "base_template.html" %}{% block content %}<article class="post-template">{{page_title}}</article>{% endblock %}"#,
        )
        .await
        .expect("failed to write post template");
        fs::write(
            template_root.join("index.html"),
            r#"{% extends "base_template.html" %}{% block content %}<section class="index-template">{% for item in items %}<a href="{{ item.url }}">{{ item.title }}</a>{% endfor %}</section>{% endblock %}"#,
        )
        .await
        .expect("failed to write index template");

        let content = create_content_fixture(&db, site.id, PageType::Post, "hello", false).await;
        let content_route = content_primary_route(&content);

        render_site(
            &db,
            site.id,
            templates_dir.path(),
            rendered_dir.path(),
            upload_root.path(),
        )
        .await
        .expect("failed to render site");

        let rendered_root = rendered_dir.path().join(site.short_name);
        let page_output = fs::read_to_string(rendered_root.join(content_route).join("index.html"))
            .await
            .expect("failed to read page output");
        let index_output = fs::read_to_string(rendered_root.join("index.html"))
            .await
            .expect("failed to read index output");

        assert!(page_output.contains("post-template"));
        assert!(page_output.contains("shell"));
        assert!(index_output.contains("index-template"));
        assert!(index_output.contains("hello"));
        assert!(index_output.contains("shell"));
    }

    #[tokio::test]
    async fn render_site_renders_raw_html_from_markdown() {
        let db = test_db_start().await;
        let site = create_site_with_template_fixture(&db, "html-site", "html-site").await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let rendered_dir = TempDir::new().expect("failed to create rendered dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");
        let template_root = templates_dir.path().join("html-site");
        fs::create_dir_all(&template_root)
            .await
            .expect("failed to create template root");
        fs::write(
            template_root.join("base_template.html"),
            r#"<!doctype html><html><body>{% block content %}{% endblock %}</body></html>"#,
        )
        .await
        .expect("failed to write base template");
        fs::write(
            template_root.join("post.html"),
            r#"{% extends "base_template.html" %}{% block content %}<article>{{content}}</article>{% endblock %}"#,
        )
        .await
        .expect("failed to write post template");

        let content = create_content(
            &db,
            NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: "HTML Post".to_string(),
                slug: "html-post".to_string(),
                page_content: r#"Before <span class="inline-html">inline html</span> after"#
                    .to_string(),
                draft: false,
                creator_sub: "creator".to_string(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create content");
        let content_route = content_primary_route(&content);

        render_site(
            &db,
            site.id,
            templates_dir.path(),
            rendered_dir.path(),
            upload_root.path(),
        )
        .await
        .expect("failed to render site");

        let rendered_root = rendered_dir.path().join(site.short_name);
        let page_output = fs::read_to_string(rendered_root.join(content_route).join("index.html"))
            .await
            .expect("failed to read page output");

        assert!(page_output.contains(r#"<span class="inline-html">inline html</span>"#));
        assert!(!page_output.contains("&lt;span"));
    }

    #[tokio::test]
    async fn render_site_expands_asset_shortcode_into_figure() {
        let db = test_db_start().await;
        let site = create_site_with_template_fixture(&db, "asset-site", "asset-site").await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let rendered_dir = TempDir::new().expect("failed to create rendered dir");
        let upload_root = TempDir::new().expect("failed to create upload root");
        let template_root = templates_dir.path().join("asset-site");
        fs::create_dir_all(&template_root)
            .await
            .expect("failed to create template root");
        fs::write(
            template_root.join("base_template.html"),
            r#"<!doctype html><html><body>{% block content %}{% endblock %}</body></html>"#,
        )
        .await
        .expect("failed to write base template");
        fs::write(
            template_root.join("page.html"),
            r#"{% extends "base_template.html" %}{% block content %}<article>{{content}}</article>{% endblock %}"#,
        )
        .await
        .expect("failed to write page template");

        let asset = create_asset(
            &db,
            NewAsset {
                site_id: site.id,
                uploader_sub: "creator".to_string(),
                original_filename: "render-image.png".to_string(),
                storage_basename: "render-image.png".to_string(),
                mime_type: "image/png".to_string(),
                byte_length: 10,
                width: Some(1024),
                height: Some(768),
            },
        )
        .await
        .expect("failed to create asset");
        create_asset_variant(
            &db,
            NewAssetVariant {
                asset_id: asset.id,
                variant_kind: "thumbnail".to_string(),
                filename: "render-image_thumb.png".to_string(),
                mime_type: "image/png".to_string(),
                byte_length: 5,
                width: Some(320),
                height: Some(240),
            },
        )
        .await
        .expect("failed to create thumbnail variant");
        fs::write(upload_root.path().join("render-image.png"), b"image")
            .await
            .expect("failed to write asset file");
        fs::write(upload_root.path().join("render-image_thumb.png"), b"thumb")
            .await
            .expect("failed to write thumbnail file");

        let content = create_content(
            &db,
            NewContent {
                site_id: site.id,
                page_type: PageType::Page,
                title: "Asset Render".to_string(),
                slug: "asset-render".to_string(),
                page_content: format!(
                    "[[asset id=\"{}\" variant=\"thumbnail\" alt=\"Render alt\" title=\"Render caption\"]]",
                    asset.id
                ),
                draft: false,
                creator_sub: "creator".to_string(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create content");

        render_site(
            &db,
            site.id,
            templates_dir.path(),
            rendered_dir.path(),
            upload_root.path(),
        )
        .await
        .expect("failed to render site");

        let rendered_root = rendered_dir.path().join(site.short_name);
        let page_output = fs::read_to_string(
            rendered_root
                .join(content_primary_route(&content))
                .join("index.html"),
        )
        .await
        .expect("failed to read page output");

        assert!(page_output.contains("<figure><img"));
        assert!(page_output.contains("src=\"/media/images/render-image_thumb.png\""));
        assert!(page_output.contains("<figcaption>Render caption</figcaption>"));
    }

    #[tokio::test]
    async fn render_site_tag_pages_skip_future_scheduled_content() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let rendered_dir = TempDir::new().expect("failed to create rendered dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");
        let now = Utc::now();

        let current = create_content(
            &db,
            NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: "Current Post".to_string(),
                slug: "current-post".to_string(),
                page_content: "Body".to_string(),
                draft: false,
                creator_sub: "creator".to_string(),
                published_at: Some(now - chrono::Duration::days(1)),
            },
        )
        .await
        .expect("failed to create current content");
        let future = create_content(
            &db,
            NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: "Future Post".to_string(),
                slug: "future-post".to_string(),
                page_content: "Body".to_string(),
                draft: false,
                creator_sub: "creator".to_string(),
                published_at: Some(now + chrono::Duration::days(7)),
            },
        )
        .await
        .expect("failed to create future content");
        add_content_tag(
            &db,
            NewContentTag {
                content_id: current.id,
                site_id: site.id,
                tag_name: "Schedule".to_string(),
            },
        )
        .await
        .expect("failed to tag current content");
        add_content_tag(
            &db,
            NewContentTag {
                content_id: future.id,
                site_id: site.id,
                tag_name: "Schedule".to_string(),
            },
        )
        .await
        .expect("failed to tag future content");

        render_site(
            &db,
            site.id,
            templates_dir.path(),
            rendered_dir.path(),
            upload_root.path(),
        )
        .await
        .expect("failed to render site");

        let rendered_root = rendered_dir.path().join(site.short_name);
        let tag_output = fs::read_to_string(
            rendered_root
                .join("tags")
                .join("schedule")
                .join("index.html"),
        )
        .await
        .expect("failed to read tag output");

        assert!(tag_output.contains("Current Post"));
        assert!(!tag_output.contains("Future Post"));
    }

    #[tokio::test]
    async fn render_site_copies_media_from_upload_root() {
        let db = test_db_start().await;
        let site = create_site_fixture(&db).await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let rendered_dir = TempDir::new().expect("failed to create rendered dir");
        let upload_root = TempDir::new().expect("failed to create upload dir");

        let _content = create_content_fixture(&db, site.id, PageType::Post, "hello", false).await;
        let asset = create_asset(
            &db,
            NewAsset {
                site_id: site.id,
                uploader_sub: "uploader".to_string(),
                original_filename: "photo.jpg".to_string(),
                storage_basename: "asset.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                byte_length: 10,
                width: Some(100),
                height: Some(200),
            },
        )
        .await
        .expect("failed to create asset");
        create_asset_variant(
            &db,
            NewAssetVariant {
                asset_id: asset.id,
                variant_kind: "thumbnail".to_string(),
                filename: "asset-thumb.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                byte_length: 5,
                width: Some(50),
                height: Some(50),
            },
        )
        .await
        .expect("failed to create asset variant");

        fs::write(upload_root.path().join("asset.jpg"), b"asset data")
            .await
            .expect("failed to write asset file");
        fs::write(upload_root.path().join("asset-thumb.jpg"), b"thumb data")
            .await
            .expect("failed to write variant file");

        render_site(
            &db,
            site.id,
            templates_dir.path(),
            rendered_dir.path(),
            upload_root.path(),
        )
        .await
        .expect("failed to render site");

        let rendered_root = rendered_dir.path().join(site.short_name);
        assert!(
            fs::metadata(rendered_root.join("media").join("images").join("asset.jpg"))
                .await
                .is_ok()
        );
        assert!(
            fs::metadata(
                rendered_root
                    .join("media")
                    .join("images")
                    .join("asset-thumb.jpg")
            )
            .await
            .is_ok()
        );
    }
}
