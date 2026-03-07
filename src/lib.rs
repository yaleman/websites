use crate::entities::PageType;
use chrono::{DateTime, Datelike, Utc};
use quick_xml::Reader;
use quick_xml::events::Event;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;
use std::result::Result as StdResult;
use std::str::FromStr;
use tera::{Context, Tera};
use tokio::fs;
use tracing::error;
use url::Url;
use uuid::Uuid;

pub mod cli;
pub mod db;
pub mod entities;
pub mod errors;
pub mod middleware;
pub mod migration;
pub mod telemetry;
pub mod tls;
pub mod web;

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
    pub role: String,
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

/// Returns audit events, optionally filtered by site_id.
pub async fn list_audit_events(
    db: &DatabaseConnection,
    site_id: Option<Uuid>,
) -> StdResult<Vec<entities::audit_event::Model>, String> {
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

const DEFAULT_POST_TEMPLATE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>{{title}}</title></head><body><h1>{{title}}</h1><article>{{content}}</article></body></html>";
const DEFAULT_PAGE_TEMPLATE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>{{title}}</title></head><body><h1>{{title}}</h1><article>{{content}}</article></body></html>";
const DEFAULT_INDEX_TEMPLATE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>{{site}}</title></head><body><h1>{{site}}</h1><ul>{{items}}</ul></body></html>";
const DEFAULT_RSS_TEMPLATE: &str = "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>{{site}}</title>{{items}}</channel></rss>";
const DEFAULT_ATOM_TEMPLATE: &str = "<?xml version=\"1.0\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><title>{{site}}</title><updated>{{updated}}</updated>{{entries}}</feed>";
const DEFAULT_TAG_TEMPLATE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>{{tag}}</title></head><body><h1>{{tag}}</h1><ul>{{items}}</ul></body></html>";

/// Renders all published content for a site into ./rendered/<site_short_name>.
pub async fn render_site(
    db: &DatabaseConnection,
    site_id: Uuid,
    templates_dir: &str,
    rendered_dir: &str,
) -> StdResult<usize, String> {
    let site = entities::site::Entity::find_by_id(site_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "site not found".to_string())?;

    let content_items = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site.id))
        .filter(entities::content_item::Column::Draft.eq(false))
        .order_by_desc(entities::content_item::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;
    let now = Utc::now();
    let content_items = content_items
        .into_iter()
        .filter(|item| content_is_publishable_at(item, now))
        .collect::<Vec<_>>();

    let template_root = Path::new(templates_dir).join(site.template_name.clone());
    let post_template = load_template(&template_root, "post.html", DEFAULT_POST_TEMPLATE).await?;
    let page_template = load_template(&template_root, "page.html", DEFAULT_PAGE_TEMPLATE).await?;
    let index_template =
        load_template(&template_root, "index.html", DEFAULT_INDEX_TEMPLATE).await?;
    let rss_template = load_template(&template_root, "rss.xml", DEFAULT_RSS_TEMPLATE).await?;
    let atom_template = load_template(&template_root, "atom.xml", DEFAULT_ATOM_TEMPLATE).await?;
    let tag_template = load_template(&template_root, "tag.html", DEFAULT_TAG_TEMPLATE).await?;

    let mut tera = Tera::default();
    tera.autoescape_on(vec![]);
    tera.add_raw_template("post.html", &post_template)
        .map_err(|error| error.to_string())?;
    tera.add_raw_template("page.html", &page_template)
        .map_err(|error| error.to_string())?;
    tera.add_raw_template("index.html", &index_template)
        .map_err(|error| error.to_string())?;
    tera.add_raw_template("rss.xml", &rss_template)
        .map_err(|error| error.to_string())?;
    tera.add_raw_template("atom.xml", &atom_template)
        .map_err(|error| error.to_string())?;
    tera.add_raw_template("tag.html", &tag_template)
        .map_err(|error| error.to_string())?;

    let rendered_root = Path::new(rendered_dir).join(site.short_name.clone());
    let tmp_root = Path::new(rendered_dir).join(format!("{}.tmp", site.short_name));

    let tmp_parent = tmp_root
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    fs::create_dir_all(&tmp_parent)
        .await
        .map_err(|error| error.to_string())?;

    if fs::metadata(&tmp_root).await.is_ok() {
        fs::remove_dir_all(&tmp_root)
            .await
            .map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&tmp_root)
        .await
        .map_err(|error| error.to_string())?;

    let mut files_written = 0usize;
    let mut index_rows = String::new();

    for item in &content_items {
        let mut routes = HashSet::new();
        routes.insert(content_primary_route(item));

        let aliases = entities::content_alias::Entity::find()
            .filter(entities::content_alias::Column::ContentId.eq(item.id))
            .all(db)
            .await
            .map_err(|error: DbErr| error.to_string())?;
        for alias in aliases {
            routes.insert(alias.alias_path);
        }

        let html = markdown::to_html(&item.page_content);
        let template = if item.page_type == PageType::Post {
            "post.html"
        } else {
            "page.html"
        };
        let tags = load_tag_names(db, item.id).await?;
        let tag_links = render_tag_links(&tags);
        let mut context = Context::new();
        context.insert("title", &item.title);
        context.insert("content", &html);
        context.insert("slug", &item.slug);
        context.insert("site_title", &site.full_title);
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
        let rendered = render_template(&tera, template, &context)?;

        for route in routes {
            let route = route.trim_end_matches('/').trim_start_matches('/');
            let output_dir = if route.is_empty() {
                tmp_root.join("")
            } else {
                tmp_root.join(route)
            };

            fs::create_dir_all(&output_dir)
                .await
                .map_err(|error| error.to_string())?;
            fs::write(output_dir.join("index.html"), rendered.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            files_written = files_written.saturating_add(1);
        }

        let primary_route = content_primary_route(item);
        index_rows.push_str(&format!(
            "<li><a href=\"/{}/\">{}</a></li>",
            primary_route.trim_matches('/'),
            item.title
        ));
    }

    let mut index_context = Context::new();
    index_context.insert("site", &site.full_title);
    index_context.insert("items", &index_rows);
    let rendered_index = render_template(&tera, "index.html", &index_context)?;
    fs::create_dir_all(&tmp_root)
        .await
        .map_err(|error| error.to_string())?;
    fs::write(tmp_root.join("index.html"), rendered_index)
        .await
        .map_err(|error| error.to_string())?;
    files_written = files_written.saturating_add(1);

    let post_items = content_items
        .iter()
        .filter(|item| item.page_type.is_post())
        .cloned()
        .collect::<Vec<_>>();

    let rss_items = render_rss_items_xml(&post_items);
    let mut rss_context = Context::new();
    rss_context.insert("site", &site.full_title);
    rss_context.insert("link", "/");
    rss_context.insert("updated", &Utc::now().to_rfc2822());
    rss_context.insert("items", &rss_items);
    let rendered_rss = render_template(&tera, "rss.xml", &rss_context)?;
    fs::write(tmp_root.join("rss.xml"), rendered_rss.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    files_written = files_written.saturating_add(1);

    let atom_entries = render_atom_entries_xml(&post_items);
    let updated = post_items
        .first()
        .map(|m| m.content_publish_timestamp())
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let mut atom_context = Context::new();
    atom_context.insert("site", &site.full_title);
    atom_context.insert("link", "/");
    atom_context.insert("updated", &updated);
    atom_context.insert("entries", &atom_entries);
    let rendered_atom = render_template(&tera, "atom.xml", &atom_context)?;
    fs::write(tmp_root.join("atom.xml"), rendered_atom.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    files_written = files_written.saturating_add(1);

    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(site.id))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    for tag in tags {
        let mut tag_rows = String::new();
        let links = entities::content_tag::Entity::find()
            .filter(entities::content_tag::Column::TagId.eq(tag.id))
            .all(db)
            .await
            .map_err(|error: DbErr| error.to_string())?;

        for link in links {
            if let Some(content) = entities::content_item::Entity::find_by_id(link.content_id)
                .one(db)
                .await
                .map_err(|error| error.to_string())?
            {
                if content.draft {
                    continue;
                }

                tag_rows.push_str(&format!(
                    "<li><a href=\"/{}/\">{}</a></li>",
                    content_primary_route(&content).trim_matches('/'),
                    content.title
                ));
            }
        }

        let mut tag_context = Context::new();
        tag_context.insert("tag", &tag.name);
        tag_context.insert("items", &tag_rows);
        let tag_output = render_template(&tera, "tag.html", &tag_context)?;
        let tag_slug = sanitize_tag_slug(&tag.name);
        let tag_path = tmp_root.join("tags").join(tag_slug);

        fs::create_dir_all(&tag_path)
            .await
            .map_err(|error| error.to_string())?;
        fs::write(tag_path.join("index.html"), tag_output.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        files_written = files_written.saturating_add(1);
    }

    let template_assets = template_root.join("assets");
    copy_directory_recursive(
        &template_assets,
        &tmp_root.join("assets"),
        &mut files_written,
    )
    .await?;

    let uploads_root = Path::new("uploads/media-storage");
    copy_media_variants(
        db,
        site.id,
        uploads_root,
        &tmp_root.join("media/images"),
        &mut files_written,
    )
    .await?;

    if fs::metadata(&rendered_root).await.is_ok() {
        fs::remove_dir_all(&rendered_root)
            .await
            .map_err(|error| error.to_string())?;
    }
    fs::rename(&tmp_root, &rendered_root)
        .await
        .map_err(|error| error.to_string())?;

    Ok(files_written)
}

async fn load_template(
    template_root: &Path,
    filename: &str,
    fallback: &str,
) -> StdResult<String, String> {
    let template_path = template_root.join(filename);
    match fs::read_to_string(&template_path).await {
        Ok(template) => Ok(template),
        Err(_) => Ok(fallback.to_string()),
    }
}

async fn copy_directory_recursive(
    source: &Path,
    destination: &Path,
    files_written: &mut usize,
) -> StdResult<(), String> {
    let mut dirs = vec![(source.to_path_buf(), destination.to_path_buf())];

    while let Some((source_path, destination_path)) = dirs.pop() {
        let mut source_dir = match fs::read_dir(&source_path).await {
            Ok(dir) => dir,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };

        fs::create_dir_all(&destination_path)
            .await
            .map_err(|error| error.to_string())?;

        while let Some(entry) = source_dir
            .next_entry()
            .await
            .map_err(|error| error.to_string())?
        {
            let next_source = entry.path();
            let next_destination = destination_path.join(entry.file_name());
            let metadata = entry.metadata().await.map_err(|error| error.to_string())?;

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
) -> StdResult<(), String> {
    let assets = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    let mut media_files = HashSet::new();
    for asset in assets.iter() {
        media_files.insert(asset.storage_basename.clone());
    }

    let asset_ids = assets.into_iter().map(|asset| asset.id).collect::<Vec<_>>();
    if !asset_ids.is_empty() {
        let variants = entities::asset_variant::Entity::find()
            .filter(entities::asset_variant::Column::AssetId.is_in(asset_ids))
            .all(db)
            .await
            .map_err(|error: DbErr| error.to_string())?;

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

async fn copy_file_if_exists(source: &Path, destination: &Path) -> StdResult<bool, String> {
    match fs::metadata(source).await {
        Ok(metadata) if metadata.is_file() => {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|error| error.to_string())?;
            }

            fs::copy(source, destination)
                .await
                .map_err(|error| error.to_string())?;

            Ok(true)
        }
        _ => Ok(false),
    }
}

fn render_template(tera: &Tera, name: &str, context: &Context) -> StdResult<String, String> {
    tera.render(name, context)
        .map_err(|error| error.to_string())
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
) -> StdResult<Vec<String>, String> {
    let links = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content_id))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

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
        .map_err(|error: DbErr| error.to_string())?;

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
    let timestamp = content.published_at.unwrap_or(content.created_at);
    timestamp <= now
}

/// Creates a site record and returns the persisted row.
pub async fn create_site<C: ConnectionTrait>(
    db: &C,
    short_name: String,
    full_title: String,
    template_name: String,
) -> StdResult<entities::site::Model, String> {
    let now = Utc::now();
    let model = entities::site::ActiveModel {
        id: Set(Uuid::now_v7()),
        short_name: Set(short_name),
        full_title: Set(full_title),
        template_name: Set(template_name),
        created_at: Set(now),
        updated_at: Set(None),
    };

    let model = model.insert(db).await.map_err(|error| {
        error!("Failed to insert site into db! {error}");
        error.to_string()
    })?;

    Ok(model)
}

/// Updates site settings and returns the updated row.
pub async fn update_site_settings<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    full_title: String,
    template_name: String,
) -> StdResult<entities::site::Model, String> {
    let existing = entities::site::Entity::find_by_id(site_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?;
    let Some(existing) = existing else {
        return Err(format!("site {site_id} not found"));
    };
    let mut model = existing.into_active_model();
    model.full_title = Set(full_title);
    model.template_name = Set(template_name);
    model.updated_at = Set(Some(Utc::now()));
    let updated = model.update(db).await.map_err(|error| error.to_string())?;
    Ok(updated)
}

/// Returns all sites ordered by short name.
pub async fn list_sites(db: &DatabaseConnection) -> StdResult<Vec<entities::site::Model>, String> {
    let sites = entities::site::Entity::find()
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(sites)
}

/// Creates one content record and one revision snapshot in the same operation.
pub async fn create_content<C: ConnectionTrait>(
    db: &C,
    input: NewContent,
) -> StdResult<entities::content_item::Model, String> {
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
    .await
    .map_err(|error| error.to_string())?;

    let revision = entities::content_revision::ActiveModel {
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
    .await
    .map_err(|error| error.to_string())?;

    drop(revision);

    Ok(content)
}

/// Updates a content row and appends a revision snapshot.
pub async fn update_content<C: ConnectionTrait>(
    db: &C,
    input: UpdateContent,
) -> StdResult<entities::content_item::Model, String> {
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
) -> StdResult<Vec<entities::content_item::Model>, String> {
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

/// Search content for a site by title, slug, or body substring.
pub async fn search_content(
    db: &DatabaseConnection,
    site_id: Uuid,
    query: &str,
) -> StdResult<Vec<entities::content_item::Model>, String> {
    let condition = Condition::any()
        .add(entities::content_item::Column::Title.contains(query))
        .add(entities::content_item::Column::Slug.contains(query))
        .add(entities::content_item::Column::PageContent.contains(query));

    let items = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .filter(condition)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(items)
}

#[derive(Default, Clone)]
struct WordpressItem {
    post_id: Option<String>,
    post_type: Option<String>,
    title: Option<String>,
    slug: Option<String>,
    content: Option<String>,
    status: Option<String>,
    post_date_gmt: Option<String>,
    link: Option<String>,
}

pub async fn import_wordpress<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    file_path: &str,
    creator_sub: &str,
) -> StdResult<usize, String> {
    let xml = fs::read_to_string(file_path)
        .await
        .map_err(|error| error.to_string())?;
    let items = parse_wordpress_wxr(xml.as_str())?;
    let mut imported = 0usize;

    for item in items {
        let post_type = PageType::from_str(&item.post_type.unwrap_or_else(|| "post".to_string()))
            .unwrap_or(PageType::Post);

        let title = item.title.unwrap_or_else(|| "Untitled".to_string());
        let slug = item
            .slug
            .filter(|slug| !slug.trim().is_empty())
            .unwrap_or_else(|| normalize_slug(&title));
        let content = item.content.unwrap_or_default();
        let status = item.status.unwrap_or_else(|| "draft".to_string());
        let draft = status != "publish";
        let published_at = item
            .post_date_gmt
            .as_deref()
            .and_then(wordpress_date_to_rfc3339);

        let content_model = create_content(
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

        if let Some(post_id) = item.post_id {
            let alias_path = format!("/?p={post_id}");
            create_alias(
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

        if let Some(link) = item.link
            && let Some(alias_path) = wordpress_link_to_alias(&link)
        {
            create_alias(
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

fn parse_wordpress_wxr(xml: &str) -> StdResult<Vec<WordpressItem>, String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut items = Vec::new();
    let mut current = WordpressItem::default();
    let mut current_tag = String::new();
    let mut in_item = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) => {
                let name = String::from_utf8_lossy(event.name().as_ref()).to_string();
                if name == "item" {
                    in_item = true;
                    current = WordpressItem::default();
                } else if in_item {
                    current_tag = name;
                }
            }
            Ok(Event::End(event)) => {
                let name = String::from_utf8_lossy(event.name().as_ref()).to_string();
                if name == "item" && in_item {
                    in_item = false;
                    items.push(current.clone());
                    current = WordpressItem::default();
                }
                current_tag.clear();
            }
            Ok(Event::Text(event)) => {
                if !in_item {
                    buf.clear();
                    continue;
                }
                let text = String::from_utf8_lossy(event.as_ref()).to_string();
                assign_wordpress_field(&mut current, current_tag.as_str(), text.trim());
            }
            Ok(Event::CData(event)) => {
                if !in_item {
                    buf.clear();
                    continue;
                }
                let text = String::from_utf8_lossy(event.as_ref()).to_string();
                assign_wordpress_field(&mut current, current_tag.as_str(), text.as_str());
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(error.to_string()),
            _ => {}
        }
        buf.clear();
    }

    Ok(items)
}

fn assign_wordpress_field(item: &mut WordpressItem, tag: &str, value: &str) {
    match tag {
        "title" => item.title = Some(value.to_string()),
        "link" => item.link = Some(value.to_string()),
        "wp:post_id" => item.post_id = Some(value.to_string()),
        "wp:post_name" => item.slug = Some(value.to_string()),
        "wp:post_type" => item.post_type = Some(value.to_string()),
        "wp:status" => item.status = Some(value.to_string()),
        "wp:post_date_gmt" => item.post_date_gmt = Some(value.to_string()),
        "content:encoded" => item.content = Some(value.to_string()),
        _ => {}
    }
}

fn wordpress_date_to_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "0000-00-00 00:00:00" {
        return None;
    }
    let rfc3339 = format!("{}Z", trimmed.replace(' ', "T"));
    DateTime::parse_from_rfc3339(&rfc3339)
        .ok()
        .map(|value| value.with_timezone(&Utc))
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

/// Returns a single content item by id.
pub async fn get_content(
    db: &DatabaseConnection,
    content_id: Uuid,
) -> StdResult<entities::content_item::Model, String> {
    let content = entities::content_item::Entity::find_by_id(content_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "content not found".to_string())?;

    Ok(content)
}

/// Returns a single site by id.
pub async fn get_site(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> StdResult<entities::site::Model, String> {
    let model = entities::site::Entity::find_by_id(site_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "site not found".to_string())?;

    Ok(model)
}

/// Creates a content alias entry.
pub async fn create_alias<C: ConnectionTrait>(
    db: &C,
    input: NewAlias,
) -> StdResult<entities::content_alias::Model, String> {
    let model = entities::content_alias::ActiveModel {
        id: Set(Uuid::now_v7()),
        content_id: Set(input.content_id),
        site_id: Set(input.site_id),
        alias_path: Set(input.alias_path),
        kind: Set(input.kind),
    }
    .insert(db)
    .await
    .map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns all content aliases, optionally scoped to content_id.
pub async fn list_aliases(
    db: &DatabaseConnection,
    site_id: Uuid,
    content_id: Option<Uuid>,
) -> StdResult<Vec<entities::content_alias::Model>, String> {
    let query = entities::content_alias::Entity::find()
        .filter(entities::content_alias::Column::SiteId.eq(site_id));
    let query = if let Some(content_id) = content_id {
        query.filter(entities::content_alias::Column::ContentId.eq(content_id))
    } else {
        query
    };

    let aliases = query.all(db).await.map_err(|error| error.to_string())?;

    Ok(aliases)
}

/// Creates a tag record.
pub async fn create_tag<C: ConnectionTrait>(
    db: &C,
    input: NewTag,
) -> StdResult<entities::tag::Model, String> {
    let model = entities::tag::ActiveModel {
        id: Set(Uuid::now_v7()),
        site_id: Set(input.site_id),
        name: Set(input.name),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns all tags for a site.
pub async fn list_tags(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> StdResult<Vec<entities::tag::Model>, String> {
    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(site_id))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(tags)
}

/// Adds a tag to content (creates the tag if missing).
pub async fn add_content_tag<C: ConnectionTrait>(
    db: &C,
    input: NewContentTag,
) -> StdResult<entities::content_tag::Model, String> {
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
) -> StdResult<Vec<entities::tag::Model>, String> {
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
) -> StdResult<Vec<entities::content_revision_alias::Model>, String> {
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
) -> StdResult<Vec<entities::tag::Model>, String> {
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
) -> StdResult<Vec<entities::content_revision::Model>, String> {
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
) -> StdResult<entities::content_revision::Model, String> {
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
) -> StdResult<Option<entities::content_revision::Model>, String> {
    let revision = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(content_id))
        .filter(entities::content_revision::Column::RevisionNumber.eq(revision_number))
        .one(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(revision)
}

pub fn content_primary_route(content: &entities::content_item::Model) -> String {
    let slug = content.slug.trim_matches('/').to_string();
    if content.page_type.is_post() {
        return slug;
    }

    let date_source = content.published_at.unwrap_or(content.created_at);

    format!("{}/{}", content_date_path(&date_source), slug)
}

fn content_date_path(date: &DateTime<Utc>) -> String {
    format!("{:04}/{:02}/{:02}", date.year(), date.month(), date.day())
}

/// Creates a user record and returns the persisted row.
pub async fn create_user<C: ConnectionTrait>(
    db: &C,
    input: NewUser,
) -> StdResult<entities::user::Model, String> {
    let model = entities::user::ActiveModel {
        id: Set(Uuid::now_v7()),
        subject: Set(input.subject),
        created_at: Set(Utc::now()),
        last_login_at: Set(None),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns all users.
pub async fn list_users(db: &DatabaseConnection) -> StdResult<Vec<entities::user::Model>, String> {
    let users = entities::user::Entity::find()
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(users)
}

/// Ensures a user exists and updates last_login_at.
pub async fn upsert_user_login<C: ConnectionTrait>(
    db: &C,
    subject: &str,
) -> StdResult<entities::user::Model, String> {
    let existing = entities::user::Entity::find()
        .filter(entities::user::Column::Subject.eq(subject.to_string()))
        .one(db)
        .await
        .map_err(|error| error.to_string())?;

    let user = if let Some(existing) = existing {
        let mut active = existing.into_active_model();
        active.last_login_at = Set(Some(Utc::now()));
        active.update(db).await.map_err(|error| error.to_string())?
    } else {
        entities::user::ActiveModel {
            id: Set(Uuid::now_v7()),
            subject: Set(subject.to_string()),
            created_at: Set(Utc::now()),
            last_login_at: Set(Some(Utc::now())),
        }
        .insert(db)
        .await
        .map_err(|error| error.to_string())?
    };

    Ok(user)
}

/// Creates one site membership record.
pub async fn create_membership<C: ConnectionTrait>(
    db: &C,
    input: NewMembership,
) -> StdResult<entities::site_membership::Model, String> {
    let model = entities::site_membership::ActiveModel {
        id: Set(Uuid::now_v7()),
        site_id: Set(input.site_id),
        user_id: Set(input.user_id),
        role: Set(input.role),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns memberships for a site.
pub async fn list_memberships(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> StdResult<Vec<entities::site_membership::Model>, String> {
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
) -> StdResult<Option<entities::site_membership::Model>, String> {
    entities::site_membership::Entity::find_by_id(membership_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())
}

/// Returns a user by subject.
pub async fn get_user_by_subject<C: ConnectionTrait>(
    db: &C,
    subject: &str,
) -> StdResult<Option<entities::user::Model>, String> {
    entities::user::Entity::find()
        .filter(entities::user::Column::Subject.eq(subject.to_string()))
        .one(db)
        .await
        .map_err(|error| error.to_string())
}

/// Returns users by id.
pub async fn list_users_by_ids<C: ConnectionTrait>(
    db: &C,
    user_ids: Vec<Uuid>,
) -> StdResult<Vec<entities::user::Model>, String> {
    if user_ids.is_empty() {
        return Ok(vec![]);
    }

    entities::user::Entity::find()
        .filter(entities::user::Column::Id.is_in(user_ids))
        .all(db)
        .await
        .map_err(|error| error.to_string())
}

/// Returns a membership for a site and user subject.
pub async fn get_membership_for_subject<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    subject: &str,
) -> StdResult<Option<entities::site_membership::Model>, String> {
    let Some(user) = get_user_by_subject(db, subject).await? else {
        return Ok(None);
    };

    entities::site_membership::Entity::find()
        .filter(entities::site_membership::Column::SiteId.eq(site_id))
        .filter(entities::site_membership::Column::UserId.eq(user.id))
        .one(db)
        .await
        .map_err(|error| error.to_string())
}

/// Returns all sites where the subject has a membership.
pub async fn list_sites_for_subject(
    db: &DatabaseConnection,
    subject: &str,
) -> StdResult<Vec<entities::site::Model>, String> {
    let Some(user) = get_user_by_subject(db, subject).await? else {
        return Ok(vec![]);
    };

    let memberships = entities::site_membership::Entity::find()
        .filter(entities::site_membership::Column::UserId.eq(user.id))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

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
        .map_err(|error| error.to_string())?;

    Ok(sites)
}

/// Updates the membership role.
pub async fn update_membership_role<C: ConnectionTrait>(
    db: &C,
    membership_id: Uuid,
    role: String,
) -> StdResult<entities::site_membership::Model, String> {
    let Some(membership) = get_membership_by_id(db, membership_id).await? else {
        return Err("membership not found".to_string());
    };
    let mut active = membership.into_active_model();
    active.role = Set(role);
    active.update(db).await.map_err(|error| error.to_string())
}

/// Deletes the membership by id.
pub async fn delete_membership<C: ConnectionTrait>(
    db: &C,
    membership_id: Uuid,
) -> StdResult<(), String> {
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
) -> StdResult<(), String> {
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
            .map_err(|error| error.to_string())?;
        let tag = match existing_tag {
            Some(tag) => tag,
            None => {
                create_tag(
                    db,
                    NewTag {
                        site_id,
                        name: normalized.clone(),
                    },
                )
                .await?
            }
        };

        let existing_content_tag = entities::content_tag::Entity::find()
            .filter(entities::content_tag::Column::ContentId.eq(content_id))
            .filter(entities::content_tag::Column::TagId.eq(tag.id))
            .one(db)
            .await
            .map_err(|error| error.to_string())?;
        if existing_content_tag.is_none() {
            let content_tag = entities::content_tag::ActiveModel {
                id: Set(Uuid::now_v7()),
                content_id: Set(content_id),
                tag_id: Set(tag.id),
            };
            content_tag
                .insert(db)
                .await
                .map_err(|error| error.to_string())?;
        }

        let existing_revision_tag = entities::content_revision_tag::Entity::find()
            .filter(entities::content_revision_tag::Column::RevisionId.eq(revision_id))
            .filter(entities::content_revision_tag::Column::TagId.eq(tag.id))
            .one(db)
            .await
            .map_err(|error| error.to_string())?;
        if existing_revision_tag.is_none() {
            let revision_tag = entities::content_revision_tag::ActiveModel {
                id: Set(Uuid::now_v7()),
                revision_id: Set(revision_id),
                tag_id: Set(tag.id),
            };
            revision_tag
                .insert(db)
                .await
                .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

/// Creates an asset record.
pub async fn create_asset<C: ConnectionTrait>(
    db: &C,
    input: NewAsset,
) -> StdResult<entities::asset::Model, String> {
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
) -> StdResult<Vec<entities::asset::Model>, String> {
    let assets = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(assets)
}

/// Creates an asset variant entry.
pub async fn create_asset_variant<C: ConnectionTrait>(
    db: &C,
    input: NewAssetVariant,
) -> StdResult<entities::asset_variant::Model, String> {
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
pub async fn list_asset_variants(
    db: &DatabaseConnection,
    asset_id: Uuid,
) -> StdResult<Vec<entities::asset_variant::Model>, String> {
    let variants = entities::asset_variant::Entity::find()
        .filter(entities::asset_variant::Column::AssetId.eq(asset_id))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(variants)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_db_start;
    use crate::entities::audit_event::log_audit_event;
    use sea_orm::{DatabaseConnection, TransactionTrait};
    use tempfile::TempDir;
    use tokio::fs;

    async fn setup_db() -> DatabaseConnection {
        test_db_start().await.as_ref().clone()
    }

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
        let db = setup_db().await;
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
        let db = setup_db().await;
        let site = create_site_fixture(&db).await;

        let sites = list_sites(&db).await.expect("failed to list sites");
        assert!(sites.iter().any(|model| model.id == site.id));

        let fetched = get_site(&db, site.id).await.expect("failed to get site");
        assert_eq!(fetched.id, site.id);
    }

    #[tokio::test]
    async fn update_site_settings_updates_values() {
        let db = setup_db().await;
        let site = create_site_fixture(&db).await;

        let updated = update_site_settings(
            &db,
            site.id,
            "Updated Title".to_string(),
            "new-template".to_string(),
        )
        .await
        .expect("failed to update site settings");

        assert_eq!(updated.full_title, "Updated Title");
        assert_eq!(updated.template_name, "new-template");
        assert!(updated.updated_at.is_some());
    }

    #[tokio::test]
    async fn transaction_rolls_back_on_error() {
        let db = setup_db().await;

        let result = db
            .transaction::<_, _, String>(|txn| {
                Box::pin(async move {
                    create_site(
                        txn,
                        "dupe".to_string(),
                        "Dupe Site".to_string(),
                        "default".to_string(),
                    )
                    .await?;
                    create_site(
                        txn,
                        "dupe".to_string(),
                        "Dupe Site 2".to_string(),
                        "default".to_string(),
                    )
                    .await?;
                    Ok(())
                })
            })
            .await;

        assert!(result.is_err(), "expected transaction error");
        let sites = list_sites(&db).await.expect("failed to list sites");
        assert!(sites.is_empty(), "expected no sites after rollback");
    }

    #[tokio::test]
    async fn content_lifecycle_and_revisions() {
        let db = setup_db().await;
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

        let search = search_content(&db, site.id, "Hello")
            .await
            .expect("failed to search content");
        assert_eq!(search.len(), 1);

        let fetched = get_content(&db, content.id)
            .await
            .expect("failed to get content");
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

        let revision_tags = list_revision_tags(&db, latest.id)
            .await
            .expect("failed to list revision tags");
        assert_eq!(revision_tags.len(), 1);
    }

    #[tokio::test]
    async fn create_and_list_tags() {
        let db = setup_db().await;
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
    async fn users_and_memberships() {
        let db = setup_db().await;
        let site = create_site_fixture(&db).await;

        let user = create_user(
            &db,
            NewUser {
                subject: "alice".to_string(),
            },
        )
        .await
        .expect("failed to create user");

        let updated = upsert_user_login(&db, "alice")
            .await
            .expect("failed to upsert user login");
        assert_eq!(updated.id, user.id);
        assert!(updated.last_login_at.is_some());

        let _ = upsert_user_login(&db, "bob")
            .await
            .expect("failed to insert user login");

        let users = list_users(&db).await.expect("failed to list users");
        assert_eq!(users.len(), 2);

        let membership = create_membership(
            &db,
            NewMembership {
                site_id: site.id,
                user_id: user.id,
                role: "owner".to_string(),
            },
        )
        .await
        .expect("failed to create membership");

        let memberships = list_memberships(&db, site.id)
            .await
            .expect("failed to list memberships");
        assert!(memberships.iter().any(|model| model.id == membership.id));
    }

    #[tokio::test]
    async fn assets_variants_and_copy_media() {
        let db = setup_db().await;
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
        let db = setup_db().await;
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
      <wp:post_date_gmt>2025-01-02 03:04:05</wp:post_date_gmt>
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

        let aliases = list_aliases(&db, site.id, None)
            .await
            .expect("failed to list aliases");
        assert_eq!(aliases.len(), 2);
    }

    #[tokio::test]
    async fn render_site_outputs_files() {
        let db = setup_db().await;
        let site = create_site_fixture(&db).await;
        let templates_dir = TempDir::new().expect("failed to create templates dir");
        let rendered_dir = TempDir::new().expect("failed to create rendered dir");

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

        let files_written = render_site(
            &db,
            site.id,
            templates_dir
                .path()
                .to_str()
                .expect("invalid templates path"),
            rendered_dir.path().to_str().expect("invalid rendered path"),
        )
        .await
        .expect("failed to render site");
        assert!(files_written >= 4);

        let rendered_root = rendered_dir.path().join(site.short_name);
        assert!(fs::metadata(rendered_root.join("index.html")).await.is_ok());
        assert!(fs::metadata(rendered_root.join("rss.xml")).await.is_ok());
        assert!(fs::metadata(rendered_root.join("atom.xml")).await.is_ok());
        assert!(
            fs::metadata(rendered_root.join("hello").join("index.html"))
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
}
