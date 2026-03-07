use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use quick_xml::Reader;
use quick_xml::events::Event;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, Database, DatabaseBackend,
    DatabaseConnection, DbErr, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
    Statement,
};
use serde_json::json;
use std::io::ErrorKind;
use std::path::Path;
use std::result::Result as StdResult;
use std::{collections::HashSet, sync::Arc};
use tera::{Context, Tera};
use tokio::fs;
use url::Url;
use uuid::Uuid;

use crate::cli::{
    AssetCommands, AuditCommands, Commands, ContentCommands, OidcConfig, ServeCommands,
    SiteCommands, UserCommands,
};

pub mod cli;
pub mod entities;
pub mod errors;
pub mod middleware;
pub mod migration;
pub mod web;

pub struct NewContent {
    pub site_id: String,
    pub page_type: String,
    pub title: String,
    pub slug: String,
    pub page_content: String,
    pub draft: bool,
    pub creator_sub: String,
    pub published_at: Option<String>,
}

pub struct NewAlias {
    pub content_id: String,
    pub site_id: String,
    pub alias_path: String,
    pub kind: String,
}

pub struct NewTag {
    pub site_id: String,
    pub name: String,
}

pub struct NewContentTag {
    pub content_id: String,
    pub site_id: String,
    pub tag_name: String,
}

pub struct NewUser {
    pub subject: String,
}

pub struct NewMembership {
    pub site_id: String,
    pub user_id: String,
    pub role: String,
}

pub struct NewAsset {
    pub site_id: String,
    pub uploader_sub: String,
    pub original_filename: String,
    pub storage_basename: String,
    pub mime_type: String,
    pub byte_length: i32,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

pub struct NewAssetVariant {
    pub asset_id: String,
    pub variant_kind: String,
    pub filename: String,
    pub mime_type: String,
    pub byte_length: i32,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

pub struct UpdateContent {
    pub content_id: String,
    pub page_type: Option<String>,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub page_content: Option<String>,
    pub draft: Option<bool>,
    pub published_at: Option<String>,
    pub editor_sub: String,
}

/// Runs all schema statements required by the current platform specification.
pub async fn ensure_schema(db: &DatabaseConnection) -> StdResult<(), String> {
    for statement in migration::SCHEMA_SQL {
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            (*statement).to_owned(),
        ))
        .await
        .map_err(|error: DbErr| error.to_string())?;
    }

    Ok(())
}

/// Records an audit event for administrative actions.
pub async fn log_audit_event(
    db: &DatabaseConnection,
    actor_sub: &str,
    event_type: &str,
    entity_type: &str,
    entity_id: &str,
    site_id: Option<&str>,
    payload_json: Option<&str>,
) -> StdResult<entities::audit_event::Model, String> {
    let model = entities::audit_event::ActiveModel {
        id: Set(uuid_v7()),
        site_id: Set(site_id.map(ToString::to_string)),
        actor_sub: Set(actor_sub.to_string()),
        event_type: Set(event_type.to_string()),
        entity_type: Set(entity_type.to_string()),
        entity_id: Set(entity_id.to_string()),
        created_at: Set(utc_now()),
        payload_json: Set(payload_json.map(ToString::to_string)),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns audit events, optionally filtered by site_id.
pub async fn list_audit_events(
    db: &DatabaseConnection,
    site_id: Option<&str>,
) -> StdResult<Vec<entities::audit_event::Model>, String> {
    let query = entities::audit_event::Entity::find();
    let query = if let Some(site_id) = site_id {
        query.filter(entities::audit_event::Column::SiteId.eq(site_id.to_owned()))
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
    site_id: &str,
    templates_dir: &str,
    rendered_dir: &str,
) -> StdResult<usize, String> {
    let site = entities::site::Entity::find_by_id(site_id.to_owned())
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "site not found".to_string())?;

    let content_items = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site.id.clone()))
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
            .filter(entities::content_alias::Column::ContentId.eq(item.id.clone()))
            .all(db)
            .await
            .map_err(|error: DbErr| error.to_string())?;
        for alias in aliases {
            routes.insert(alias.alias_path);
        }

        let html = markdown::to_html(&item.page_content);
        let template = if item.page_type == "post" {
            "post.html"
        } else {
            "page.html"
        };
        let tags = load_tag_names(db, &item.id).await?;
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
        .filter(|item| item.page_type == "post")
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
        .map(content_publish_timestamp)
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
        .filter(entities::tag::Column::SiteId.eq(site.id.clone()))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    for tag in tags {
        let mut tag_rows = String::new();
        let links = entities::content_tag::Entity::find()
            .filter(entities::content_tag::Column::TagId.eq(tag.id.clone()))
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
        &site.id,
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
    site_id: &str,
    source_root: &Path,
    destination_root: &Path,
    files_written: &mut usize,
) -> StdResult<(), String> {
    let assets = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id.to_string()))
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
    content_id: &str,
) -> StdResult<Vec<String>, String> {
    let links = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content_id.to_owned()))
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
        let route = content_primary_route(item);
        let route = route.trim_matches('/');
        let link = format!("/{route}/");
        let summary = escape_xml_text(item.page_content.as_str());
        let pub_date = content_publish_timestamp_rfc2822(item);
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
        let updated = content_publish_timestamp(item);
        let title = escape_xml_text(item.title.as_str());
        let published = content_publish_timestamp_rfc2822(item);
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

fn content_publish_timestamp(content: &entities::content_item::Model) -> String {
    content
        .published_at
        .clone()
        .unwrap_or_else(|| content.created_at.clone())
}

fn content_publish_timestamp_rfc2822(content: &entities::content_item::Model) -> String {
    DateTime::parse_from_rfc3339(content_publish_timestamp(content).as_str())
        .map(|timestamp| timestamp.to_rfc2822())
        .unwrap_or_else(|_| Utc::now().to_rfc2822())
}

fn content_is_publishable_at(content: &entities::content_item::Model, now: DateTime<Utc>) -> bool {
    let timestamp = content
        .published_at
        .as_deref()
        .unwrap_or(content.created_at.as_str());
    DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.with_timezone(&Utc) <= now)
        .unwrap_or(true)
}

/// Creates a site record and returns the persisted row.
pub async fn create_site(
    db: &DatabaseConnection,
    short_name: String,
    full_title: String,
    template_name: String,
) -> StdResult<entities::site::Model, String> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let model = entities::site::ActiveModel {
        id: Set(uuid_v7()),
        short_name: Set(short_name),
        full_title: Set(full_title),
        template_name: Set(template_name),
        created_at: Set(now.clone()),
        updated_at: Set(now),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns all sites ordered by short name.
pub async fn list_sites(db: &DatabaseConnection) -> StdResult<Vec<entities::site::Model>, String> {
    let sites = entities::site::Entity::find()
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(sites)
}

fn uuid_v7() -> String {
    Uuid::now_v7().to_string()
}

fn utc_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Creates one content record and one revision snapshot in the same operation.
pub async fn create_content(
    db: &DatabaseConnection,
    input: NewContent,
) -> StdResult<entities::content_item::Model, String> {
    let now = utc_now();
    let content_id = uuid_v7();
    let revision_id = uuid_v7();
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
        Some(published_at.unwrap_or_else(utc_now))
    } else {
        published_at
    };

    let content = entities::content_item::ActiveModel {
        id: Set(content_id.clone()),
        site_id: Set(site_id.clone()),
        page_type: Set(page_type.clone()),
        title: Set(title.clone()),
        slug: Set(slug.clone()),
        page_content: Set(page_content.clone()),
        draft: Set(draft),
        creator_sub: Set(creator_sub),
        created_at: Set(now.clone()),
        last_updated: Set(now.clone()),
        published_at: Set(published_at),
    }
    .insert(db)
    .await
    .map_err(|error| error.to_string())?;

    let revision = entities::content_revision::ActiveModel {
        id: Set(revision_id),
        content_id: Set(content.id.clone()),
        site_id: Set(site_id),
        revision_number: Set(1),
        title: Set(title),
        slug: Set(slug),
        page_content: Set(page_content),
        draft: Set(draft),
        page_type: Set(page_type),
        editor_sub: Set(content.creator_sub.clone()),
        created_at: Set(now.clone()),
    }
    .insert(db)
    .await
    .map_err(|error| error.to_string())?;

    drop(revision);

    Ok(content)
}

/// Updates a content row and appends a revision snapshot.
pub async fn update_content(
    db: &DatabaseConnection,
    input: UpdateContent,
) -> StdResult<entities::content_item::Model, String> {
    let now = utc_now();
    let existing = entities::content_item::Entity::find_by_id(&input.content_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "content not found".to_string())?;

    let publish_now = input.draft == Some(false) && existing.draft;
    let published_at = if let Some(published_at) = input.published_at {
        Some(published_at)
    } else if publish_now {
        Some(now.clone())
    } else {
        existing.published_at.clone()
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
    active.last_updated = Set(now.clone());

    let content: entities::content_item::Model = active
        .update(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    let revisions = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(input.content_id.as_str()))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;
    let revision_number = i32::try_from(revisions.len())
        .map_err(|error: std::num::TryFromIntError| error.to_string())?
        .saturating_add(1);

    let revision = entities::content_revision::ActiveModel {
        id: Set(uuid_v7()),
        content_id: Set(content.id.clone()),
        site_id: Set(content.site_id.clone()),
        revision_number: Set(revision_number),
        title: Set(content.title.clone()),
        slug: Set(content.slug.clone()),
        page_content: Set(content.page_content.clone()),
        draft: Set(content.draft),
        page_type: Set(content.page_type.clone()),
        editor_sub: Set(input.editor_sub),
        created_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|error| error.to_string())?;

    let content_tags = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content.id.clone()))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    for content_tag in content_tags {
        entities::content_revision_tag::ActiveModel {
            id: Set(uuid_v7()),
            revision_id: Set(revision.id.clone()),
            tag_id: Set(content_tag.tag_id),
        }
        .insert(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;
    }

    let aliases = entities::content_alias::Entity::find()
        .filter(entities::content_alias::Column::ContentId.eq(content.id.clone()))
        .all(db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    for alias in aliases {
        let _revision_alias = entities::content_revision_alias::ActiveModel {
            id: Set(uuid_v7()),
            revision_id: Set(revision.id.clone()),
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
    site_id: &str,
    page_type: Option<&str>,
) -> StdResult<Vec<entities::content_item::Model>, String> {
    let query = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id.to_owned()));
    let query = if let Some(filter) = page_type {
        query.filter(entities::content_item::Column::PageType.eq(filter.to_owned()))
    } else {
        query
    };

    let content = query.all(db).await.map_err(|error| error.to_string())?;

    Ok(content)
}

/// Search content for a site by title, slug, or body substring.
pub async fn search_content(
    db: &DatabaseConnection,
    site_id: &str,
    query: &str,
) -> StdResult<Vec<entities::content_item::Model>, String> {
    let condition = Condition::any()
        .add(entities::content_item::Column::Title.contains(query))
        .add(entities::content_item::Column::Slug.contains(query))
        .add(entities::content_item::Column::PageContent.contains(query));

    let items = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id.to_owned()))
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

pub async fn import_wordpress(
    db: &DatabaseConnection,
    site_id: &str,
    file_path: &str,
    creator_sub: &str,
) -> StdResult<usize, String> {
    let xml = fs::read_to_string(file_path)
        .await
        .map_err(|error| error.to_string())?;
    let items = parse_wordpress_wxr(xml.as_str())?;
    let mut imported = 0usize;

    for item in items {
        let post_type = item.post_type.unwrap_or_else(|| "post".to_string());
        if post_type != "post" && post_type != "page" {
            continue;
        }

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
                site_id: site_id.to_string(),
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
            let _ = create_alias(
                db,
                NewAlias {
                    content_id: content_model.id.clone(),
                    site_id: site_id.to_string(),
                    alias_path,
                    kind: "alias".to_string(),
                },
            )
            .await;
        }

        if let Some(link) = item.link
            && let Some(alias_path) = wordpress_link_to_alias(&link)
        {
            let _ = create_alias(
                db,
                NewAlias {
                    content_id: content_model.id.clone(),
                    site_id: site_id.to_string(),
                    alias_path,
                    kind: "alias".to_string(),
                },
            )
            .await;
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

fn wordpress_date_to_rfc3339(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "0000-00-00 00:00:00" {
        return None;
    }
    Some(format!("{}Z", trimmed.replace(' ', "T")))
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
    content_id: &str,
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
    site_id: &str,
) -> StdResult<entities::site::Model, String> {
    let model = entities::site::Entity::find_by_id(site_id.to_owned())
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "site not found".to_string())?;

    Ok(model)
}

/// Creates a content alias entry.
pub async fn create_alias(
    db: &DatabaseConnection,
    input: NewAlias,
) -> StdResult<entities::content_alias::Model, String> {
    let model = entities::content_alias::ActiveModel {
        id: Set(uuid_v7()),
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
    site_id: &str,
    content_id: Option<&str>,
) -> StdResult<Vec<entities::content_alias::Model>, String> {
    let query = entities::content_alias::Entity::find()
        .filter(entities::content_alias::Column::SiteId.eq(site_id.to_owned()));
    let query = if let Some(content_id) = content_id {
        query.filter(entities::content_alias::Column::ContentId.eq(content_id.to_owned()))
    } else {
        query
    };

    let aliases = query.all(db).await.map_err(|error| error.to_string())?;

    Ok(aliases)
}

/// Creates a tag record.
pub async fn create_tag(
    db: &DatabaseConnection,
    input: NewTag,
) -> StdResult<entities::tag::Model, String> {
    let model = entities::tag::ActiveModel {
        id: Set(uuid_v7()),
        site_id: Set(input.site_id),
        name: Set(input.name),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns all tags for a site.
pub async fn list_tags(
    db: &DatabaseConnection,
    site_id: &str,
) -> StdResult<Vec<entities::tag::Model>, String> {
    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(site_id.to_owned()))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(tags)
}

/// Adds a tag to content (creates the tag if missing).
pub async fn add_content_tag(
    db: &DatabaseConnection,
    input: NewContentTag,
) -> StdResult<entities::content_tag::Model, String> {
    let content = entities::content_item::Entity::find_by_id(&input.content_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "content not found".to_string())?;

    if content.site_id != input.site_id {
        return Err("content does not belong to provided site".to_string());
    }

    let tag = if let Some(tag) = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(input.site_id.as_str()))
        .filter(entities::tag::Column::Name.eq(input.tag_name.clone()))
        .one(db)
        .await
        .map_err(|error| error.to_string())?
    {
        tag
    } else {
        entities::tag::ActiveModel {
            id: Set(uuid_v7()),
            site_id: Set(input.site_id.clone()),
            name: Set(input.tag_name),
        }
        .insert(db)
        .await
        .map_err(|error| error.to_string())?
    };

    let model = entities::content_tag::ActiveModel {
        id: Set(uuid_v7()),
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
    content_id: &str,
) -> StdResult<Vec<entities::tag::Model>, String> {
    let links = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content_id.to_owned()))
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
    revision_id: &str,
) -> StdResult<Vec<entities::content_revision_alias::Model>, String> {
    let revision_aliases = entities::content_revision_alias::Entity::find()
        .filter(entities::content_revision_alias::Column::RevisionId.eq(revision_id.to_owned()))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(revision_aliases)
}

/// Returns all tags captured for a specific content revision.
pub async fn list_revision_tags(
    db: &DatabaseConnection,
    revision_id: &str,
) -> StdResult<Vec<entities::tag::Model>, String> {
    let links = entities::content_revision_tag::Entity::find()
        .filter(entities::content_revision_tag::Column::RevisionId.eq(revision_id.to_owned()))
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
    content_id: &str,
) -> StdResult<Vec<entities::content_revision::Model>, String> {
    let revisions = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(content_id.to_owned()))
        .order_by_desc(entities::content_revision::Column::RevisionNumber)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(revisions)
}

/// Returns a single revision by id.
pub async fn get_revision(
    db: &DatabaseConnection,
    revision_id: &str,
) -> StdResult<entities::content_revision::Model, String> {
    let revision = entities::content_revision::Entity::find_by_id(revision_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "revision not found".to_string())?;

    Ok(revision)
}

/// Returns a revision by number, if present.
pub async fn get_revision_by_number(
    db: &DatabaseConnection,
    content_id: &str,
    revision_number: i32,
) -> StdResult<Option<entities::content_revision::Model>, String> {
    let revision = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(content_id.to_owned()))
        .filter(entities::content_revision::Column::RevisionNumber.eq(revision_number))
        .one(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(revision)
}

pub fn content_primary_route(content: &entities::content_item::Model) -> String {
    let slug = content.slug.trim_matches('/').to_string();
    if content.page_type != "post" {
        return slug;
    }

    let date_source = content
        .published_at
        .as_deref()
        .unwrap_or(content.created_at.as_str());

    format!(
        "{}/{}",
        content_date_path(date_source).unwrap_or_else(|| "0000/00/00".to_string()),
        slug
    )
}

fn content_date_path(value: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| format!("{:04}/{:02}/{:02}", date.year(), date.month(), date.day()))
}

/// Creates a user record and returns the persisted row.
pub async fn create_user(
    db: &DatabaseConnection,
    input: NewUser,
) -> StdResult<entities::user::Model, String> {
    let model = entities::user::ActiveModel {
        id: Set(uuid_v7()),
        subject: Set(input.subject),
        created_at: Set(utc_now()),
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
pub async fn upsert_user_login(
    db: &DatabaseConnection,
    subject: &str,
) -> StdResult<entities::user::Model, String> {
    let existing = entities::user::Entity::find()
        .filter(entities::user::Column::Subject.eq(subject.to_string()))
        .one(db)
        .await
        .map_err(|error| error.to_string())?;

    let user = if let Some(existing) = existing {
        let mut active = existing.into_active_model();
        active.last_login_at = Set(Some(utc_now()));
        active.update(db).await.map_err(|error| error.to_string())?
    } else {
        entities::user::ActiveModel {
            id: Set(uuid_v7()),
            subject: Set(subject.to_string()),
            created_at: Set(utc_now()),
            last_login_at: Set(Some(utc_now())),
        }
        .insert(db)
        .await
        .map_err(|error| error.to_string())?
    };

    Ok(user)
}

/// Creates one site membership record.
pub async fn create_membership(
    db: &DatabaseConnection,
    input: NewMembership,
) -> StdResult<entities::site_membership::Model, String> {
    let model = entities::site_membership::ActiveModel {
        id: Set(uuid_v7()),
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
    site_id: &str,
) -> StdResult<Vec<entities::site_membership::Model>, String> {
    let memberships = entities::site_membership::Entity::find()
        .filter(entities::site_membership::Column::SiteId.eq(site_id.to_owned()))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(memberships)
}

/// Creates an asset record.
pub async fn create_asset(
    db: &DatabaseConnection,
    input: NewAsset,
) -> StdResult<entities::asset::Model, String> {
    let model = entities::asset::ActiveModel {
        id: Set(uuid_v7()),
        site_id: Set(input.site_id),
        uploader_sub: Set(input.uploader_sub),
        original_filename: Set(input.original_filename),
        storage_basename: Set(input.storage_basename),
        mime_type: Set(input.mime_type),
        byte_length: Set(input.byte_length),
        width: Set(input.width),
        height: Set(input.height),
        created_at: Set(utc_now()),
    };

    let model = model.insert(db).await.map_err(|error| error.to_string())?;

    Ok(model)
}

/// Returns all assets for a site.
pub async fn list_assets(
    db: &DatabaseConnection,
    site_id: &str,
) -> StdResult<Vec<entities::asset::Model>, String> {
    let assets = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id.to_owned()))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(assets)
}

/// Creates an asset variant entry.
pub async fn create_asset_variant(
    db: &DatabaseConnection,
    input: NewAssetVariant,
) -> StdResult<entities::asset_variant::Model, String> {
    let model = entities::asset_variant::ActiveModel {
        id: Set(uuid_v7()),
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
    asset_id: &str,
) -> StdResult<Vec<entities::asset_variant::Model>, String> {
    let variants = entities::asset_variant::Entity::find()
        .filter(entities::asset_variant::Column::AssetId.eq(asset_id.to_owned()))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;

    Ok(variants)
}

pub async fn execute(
    command: Commands,
    database_url: &str,
    oidc: &OidcConfig,
) -> Result<(), String> {
    let db = Arc::new(
        Database::connect(database_url)
            .await
            .map_err(|error| error.to_string())?,
    );
    match command {
        Commands::Init => {
            ensure_schema(&db).await?;
            println!("database initialized: {}", database_url);
            Ok(())
        }
        Commands::ShowConfig => {
            println!("tls_cert_path={}", cli_value(&oidc.tls_cert_path));
            println!("tls_key_path={}", cli_value(&oidc.tls_key_path));
            println!("frontend_url={}", cli_value(&oidc.frontend_url));
            println!("oidc_client_id={}", cli_value(&oidc.oidc_client_id));
            println!("oidc_discovery_url={}", cli_value(&oidc.oidc_discovery_url));
            Ok(())
        }
        Commands::Site { command } => match command {
            SiteCommands::Create {
                short_name,
                full_title,
                template_name,
            } => {
                let site = create_site(&db, short_name, full_title, template_name).await?;
                let _ = log_audit_event(
                    &db,
                    "system",
                    "create_site",
                    "site",
                    &site.id,
                    Some(&site.id),
                    Some(&format!(
                        "{}",
                        json!(
                            {"short_name":&site.short_name,"full_title":&site.full_title}
                        )
                    )),
                )
                .await?;
                println!("created site: {} ({})", site.id, site.short_name);
                Ok(())
            }
            SiteCommands::List => {
                let sites = list_sites(&db).await?;
                if sites.is_empty() {
                    println!("no sites");
                    return Ok(());
                }

                println!("short_name\tfull_title\ttemplate_name");
                for site in sites {
                    println!(
                        "{}\t{}\t{}",
                        site.short_name, site.full_title, site.template_name
                    );
                }
                Ok(())
            }
            SiteCommands::MemberAdd {
                site_id,
                user_id,
                role,
            } => {
                let membership = create_membership(
                    &db,
                    NewMembership {
                        site_id,
                        user_id,
                        role,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    &db,
                    "system",
                    "create_membership",
                    "site_membership",
                    &membership.id,
                    Some(&membership.site_id),
                    Some(&format!("{}", json!(
                         {"site_id":&membership.site_id,"user_id":&membership.user_id,"role":&membership.role}
                    ))),
                )
                .await?;
                println!("created membership: {} {}", membership.id, membership.role);
                Ok(())
            }
            SiteCommands::MemberList { site_id } => {
                let memberships = list_memberships(&db, &site_id).await?;
                if memberships.is_empty() {
                    println!("no memberships");
                    return Ok(());
                }

                println!("id\tsite_id\tuser_id\trole");
                for row in memberships {
                    println!("{}\t{}\t{}\t{}", row.id, row.site_id, row.user_id, row.role);
                }
                Ok(())
            }
            SiteCommands::TagCreate { site_id, name } => {
                let tag = create_tag(&db, NewTag { site_id, name }).await?;
                let _ = log_audit_event(
                    &db,
                    "system",
                    "create_tag",
                    "tag",
                    &tag.id,
                    Some(&tag.site_id),
                    Some(&format!("{}", json!({"name":&tag.name}))),
                )
                .await?;
                println!("created tag: {} {}", tag.id, tag.name);
                Ok(())
            }
            SiteCommands::TagList { site_id } => {
                let tags = list_tags(&db, &site_id).await?;
                if tags.is_empty() {
                    println!("no tags");
                    return Ok(());
                }

                println!("id\tname");
                for row in tags {
                    println!("{}\t{}", row.id, row.name);
                }
                Ok(())
            }
            SiteCommands::Render {
                site_id,
                templates_dir,
                rendered_dir,
            } => {
                let files_written =
                    render_site(&db, &site_id, &templates_dir, &rendered_dir).await?;
                println!("rendered site {} files {}", site_id, files_written);
                Ok(())
            }
        },
        Commands::User { command } => match command {
            UserCommands::Create { subject } => {
                let user = create_user(&db, NewUser { subject }).await?;
                let _ = log_audit_event(
                    &db,
                    "system",
                    "create_user",
                    "user",
                    &user.id,
                    None,
                    Some(&format!("{}", json!({"subject":&user.subject}))),
                )
                .await?;
                println!("created user: {} {}", user.id, user.subject);
                Ok(())
            }
            UserCommands::List => {
                let users = list_users(&db).await?;
                if users.is_empty() {
                    println!("no users");
                    return Ok(());
                }

                println!("id\tsubject\tlast_login_at");
                for row in users {
                    println!("{}\t{}\t{:?}", row.id, row.subject, row.last_login_at);
                }
                Ok(())
            }
        },
        Commands::Asset { command } => match command {
            AssetCommands::Create {
                site_id,
                uploader_sub,
                original_filename,
                storage_basename,
                mime_type,
                byte_length,
                width,
                height,
            } => {
                let asset = create_asset(
                    &db,
                    NewAsset {
                        site_id,
                        uploader_sub,
                        original_filename,
                        storage_basename,
                        mime_type,
                        byte_length,
                        width,
                        height,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    &db,
                    &asset.uploader_sub,
                    "create_asset",
                    "asset",
                    &asset.id,
                    Some(&asset.site_id),
                    Some(&format!(
                        "{}",
                        json!({"original_filename":&asset.original_filename,"storage_basename":&asset.storage_basename})
                    )),
                )
                .await?;
                println!("created asset: {} {}", asset.id, asset.original_filename);
                Ok(())
            }
            AssetCommands::List { site_id } => {
                let assets = list_assets(&db, &site_id).await?;
                if assets.is_empty() {
                    println!("no assets");
                    return Ok(());
                }

                println!("id\toriginal_filename\tstorage_basename\tmime_type");
                for row in assets {
                    println!(
                        "{}\t{}\t{}\t{}",
                        row.id, row.original_filename, row.storage_basename, row.mime_type
                    );
                }
                Ok(())
            }
            AssetCommands::VariantCreate {
                asset_id,
                variant_kind,
                filename,
                mime_type,
                byte_length,
                width,
                height,
            } => {
                let variant = create_asset_variant(
                    &db,
                    NewAssetVariant {
                        asset_id,
                        variant_kind,
                        filename,
                        mime_type,
                        byte_length,
                        width,
                        height,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    &db,
                    "system",
                    "create_asset_variant",
                    "asset_variant",
                    &variant.id,
                    Some(&variant.asset_id),
                    Some(&format!(
                        "{}",
                        json!({"variant_kind":&variant.variant_kind,"filename":&variant.filename})
                    )),
                )
                .await?;
                println!("created variant: {} {}", variant.id, variant.filename);
                Ok(())
            }
            AssetCommands::VariantList { asset_id } => {
                let variants = list_asset_variants(&db, &asset_id).await?;
                if variants.is_empty() {
                    println!("no variants");
                    return Ok(());
                }

                println!("id\tasset_id\tvariant_kind\tfilename\tmime_type");
                for row in variants {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        row.id, row.asset_id, row.variant_kind, row.filename, row.mime_type
                    );
                }
                Ok(())
            }
        },
        Commands::Serve { command } => match command {
            ServeCommands::Admin { listen } => {
                web::run_admin_server(db.clone(), &listen, oidc).await
            }
        },
        Commands::Audit { command } => match command {
            AuditCommands::List { site_id } => {
                let events = list_audit_events(&db, site_id.as_deref()).await?;
                if events.is_empty() {
                    println!("no audit events");
                    return Ok(());
                }

                println!(
                    "id\tsite_id\tactor_sub\tevent_type\tentity_type\tentity_id\tcreated_at\tpayload_json"
                );
                for event in events {
                    let site_id = event.site_id.unwrap_or_else(|| "-".to_string());
                    let payload = event.payload_json.unwrap_or_else(|| "null".to_string());
                    println!(
                        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        event.id,
                        site_id,
                        event.actor_sub,
                        event.event_type,
                        event.entity_type,
                        event.entity_id,
                        event.created_at,
                        payload
                    );
                }
                Ok(())
            }
        },
        Commands::Content { command } => match command {
            ContentCommands::Create {
                site_id,
                page_type,
                title,
                slug,
                page_content,
                creator_sub,
                draft,
                published_at,
            } => {
                let content = create_content(
                    &db,
                    NewContent {
                        site_id,
                        page_type,
                        title,
                        slug,
                        page_content,
                        creator_sub,
                        draft,
                        published_at,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    &db,
                    &content.creator_sub,
                    "create_content",
                    "content_item",
                    &content.id,
                    Some(&content.site_id),
                    Some(&format!(
                        "{}",
                        json!({
                            "page_type": &content.page_type,
                            "slug": &content.slug,
                            "title": &content.title,
                            "draft": content.draft
                        })
                    )),
                )
                .await?;
                println!("created content: {} {}", content.id, content.title);
                Ok(())
            }
            ContentCommands::List { site_id, page_type } => {
                let page_filter = page_type.as_deref();
                let content = list_content(&db, &site_id, page_filter).await?;
                if content.is_empty() {
                    println!("no content");
                    return Ok(());
                }

                println!("id\ttitle\tslug\tpage_type\tdraft\tcreated_at\tpublished_at\turl");
                for row in content {
                    let published_at = row
                        .published_at
                        .as_ref()
                        .map_or_else(|| "n/a".to_string(), |value| value.clone());
                    let public_path = content_primary_route(&row);
                    let public_url = format!("/{}/", public_path.trim_matches('/'));
                    println!(
                        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        row.id,
                        row.title,
                        row.slug,
                        row.page_type,
                        row.draft,
                        row.created_at,
                        published_at,
                        public_url
                    );
                }
                Ok(())
            }
            ContentCommands::AliasCreate {
                content_id,
                site_id,
                alias_path,
                kind,
            } => {
                let alias = create_alias(
                    &db,
                    NewAlias {
                        content_id,
                        site_id,
                        alias_path,
                        kind,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    &db,
                    "system",
                    "create_alias",
                    "content_alias",
                    &alias.id,
                    Some(&alias.site_id),
                    Some(&format!(
                        "{}",
                        json!({
                            "content_id": &alias.content_id,
                            "alias_path": &alias.alias_path,
                            "kind": &alias.kind
                        })
                    )),
                )
                .await?;
                println!("created alias: {} {}", alias.id, alias.alias_path);
                Ok(())
            }
            ContentCommands::AliasList {
                site_id,
                content_id,
            } => {
                let content_filter = content_id.as_deref();
                let aliases = list_aliases(&db, &site_id, content_filter).await?;
                if aliases.is_empty() {
                    println!("no aliases");
                    return Ok(());
                }

                println!("id\tcontent_id\talias_path\tkind");
                for row in aliases {
                    println!(
                        "{}\t{}\t{}\t{}",
                        row.id, row.content_id, row.alias_path, row.kind
                    );
                }
                Ok(())
            }
            ContentCommands::Revisions { content_id } => {
                let revisions = list_revisions(&db.clone(), &content_id).await?;
                if revisions.is_empty() {
                    println!("no revisions");
                    return Ok(());
                }

                println!("id\trevision_number\ttitle\tdraft\tcreated_at");
                for row in revisions {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        row.id, row.revision_number, row.title, row.draft, row.created_at
                    );
                }
                Ok(())
            }
            ContentCommands::RevisionAliases { revision_id } => {
                let aliases = list_revision_aliases(&db, &revision_id).await?;
                if aliases.is_empty() {
                    println!("no revision aliases");
                    return Ok(());
                }

                println!("id\talias_path\tkind");
                for row in aliases {
                    println!("{}\t{}\t{}", row.id, row.alias_path, row.kind);
                }
                Ok(())
            }
            ContentCommands::RevisionTags { revision_id } => {
                let tags = list_revision_tags(&db, &revision_id).await?;
                if tags.is_empty() {
                    println!("no revision tags");
                    return Ok(());
                }

                println!("id\tname");
                for row in tags {
                    println!("{}\t{}", row.id, row.name);
                }
                Ok(())
            }
            ContentCommands::Inspect { content_id } => {
                let content = get_content(&db, &content_id).await?;
                let aliases = list_aliases(&db, &content.site_id, Some(&content_id)).await?;
                let tags = list_content_tags(&db, &content_id).await?;
                let revisions = list_revisions(&db, &content_id).await?;

                let public_path = content_primary_route(&content);
                println!("id:\t{}", content.id);
                println!("site_id:\t{}", content.site_id);
                println!("title:\t{}", content.title);
                println!("slug:\t{}", content.slug);
                println!("page_type:\t{}", content.page_type);
                println!("draft:\t{}", content.draft);
                println!(
                    "published_at:\t{}",
                    content.published_at.as_deref().unwrap_or("n/a")
                );
                println!("created_at:\t{}", content.created_at);
                println!("updated_at:\t{}", content.last_updated);
                println!("primary_route:\t/{}", public_path.trim_matches('/'));
                println!("aliases:");
                if aliases.is_empty() {
                    println!("\t(none)");
                } else {
                    for alias in aliases {
                        println!("\t{}\t{}", alias.kind, alias.alias_path);
                    }
                }
                println!("tags:");
                if tags.is_empty() {
                    println!("\t(none)");
                } else {
                    for tag in tags {
                        println!("\t{}", tag.name);
                    }
                }
                println!("revisions:\t{}", revisions.len());
                if let Some(latest_revision) = revisions.first() {
                    println!(
                        "latest_revision:\t{} @ {}",
                        latest_revision.revision_number, latest_revision.created_at
                    );
                }
                Ok(())
            }
            ContentCommands::Update {
                content_id,
                page_type,
                title,
                slug,
                page_content,
                draft,
                published_at,
                editor_sub,
            } => {
                let content = update_content(
                    &db,
                    UpdateContent {
                        content_id,
                        page_type,
                        title,
                        slug,
                        page_content,
                        draft,
                        published_at,
                        editor_sub,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    &db,
                    &content.creator_sub,
                    "update_content",
                    "content_item",
                    &content.id,
                    Some(&content.site_id),
                    Some(&format!(
                        "{}",
                        json!({
                            "content_id": &content.id,
                            "page_type": &content.page_type,
                            "slug": &content.slug,
                            "title": &content.title
                        })
                    )),
                )
                .await?;
                println!("updated content: {} {}", content.id, content.title);
                Ok(())
            }
            ContentCommands::TagAdd {
                content_id,
                site_id,
                tag_name,
            } => {
                let content_tag = add_content_tag(
                    &db,
                    NewContentTag {
                        content_id,
                        site_id,
                        tag_name,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    &db,
                    "system",
                    "add_content_tag",
                    "content_tag",
                    &content_tag.id,
                    Some(&content_tag.content_id),
                    Some(&format!(
                        "{}",
                        json!({
                            "content_id": &content_tag.content_id,
                            "tag_id": &content_tag.tag_id
                        })
                    )),
                )
                .await?;
                println!(
                    "linked content tag: {} {}",
                    content_tag.id, content_tag.tag_id
                );
                Ok(())
            }
            ContentCommands::TagList { content_id } => {
                let tags = list_content_tags(&db, &content_id).await?;
                if tags.is_empty() {
                    println!("no tags");
                    return Ok(());
                }

                println!("id\tname");
                for row in tags {
                    println!("{}\t{}", row.id, row.name);
                }
                Ok(())
            }
            ContentCommands::ImportWordpress {
                site_id,
                file_path,
                creator_sub,
            } => {
                let imported = import_wordpress(&db, &site_id, &file_path, &creator_sub).await?;
                let _ = log_audit_event(
                    &db,
                    &creator_sub,
                    "import_wordpress",
                    "content_item",
                    &site_id,
                    Some(&site_id),
                    Some(&format!("{}", json!({"imported": imported}))),
                )
                .await?;
                println!("imported {} wordpress items", imported);
                Ok(())
            }
        },
    }
}

fn cli_value(value: &Option<String>) -> String {
    value
        .as_deref()
        .map_or_else(|| "<unset>".to_string(), |value| value.to_string())
}
