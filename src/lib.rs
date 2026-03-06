use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;
use std::result::Result as StdResult;

use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, Database, DatabaseBackend, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, Set, Statement,
};
use tokio::fs;
use uuid::Uuid;

pub mod entities;

pub mod migration;

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
pub async fn ensure_schema(database_url: &str) -> StdResult<(), String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    for statement in migration::SCHEMA_SQL {
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            (*statement).to_owned(),
        ))
        .await
        .map_err(|error: DbErr| error.to_string())?;
    }

    let _ = db.close().await;
    Ok(())
}

const DEFAULT_POST_TEMPLATE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>{{title}}</title></head><body><h1>{{title}}</h1><article>{{content}}</article></body></html>";
const DEFAULT_PAGE_TEMPLATE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>{{title}}</title></head><body><h1>{{title}}</h1><article>{{content}}</article></body></html>";
const DEFAULT_INDEX_TEMPLATE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>{{site}}</title></head><body><h1>{{site}}</h1><ul>{{items}}</ul></body></html>";
const DEFAULT_RSS_TEMPLATE: &str = "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>{{site}}</title>{{items}}</channel></rss>";
const DEFAULT_ATOM_TEMPLATE: &str = "<?xml version=\"1.0\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><title>{{site}}</title><updated>{{updated}}</updated>{{entries}}</feed>";
const DEFAULT_TAG_TEMPLATE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>{{tag}}</title></head><body><h1>{{tag}}</h1><ul>{{items}}</ul></body></html>";

/// Renders all published content for a site into ./rendered/<site_short_name>.
pub async fn render_site(
    database_url: &str,
    site_id: &str,
    templates_dir: &str,
    rendered_dir: &str,
) -> StdResult<usize, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let site = entities::site::Entity::find_by_id(site_id.to_owned())
        .one(&db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "site not found".to_string())?;

    let content_items = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site.id.clone()))
        .filter(entities::content_item::Column::Draft.eq(false))
        .order_by_desc(entities::content_item::Column::CreatedAt)
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let template_root = Path::new(templates_dir).join(site.template_name.clone());
    let post_template = load_template(&template_root, "post.html", DEFAULT_POST_TEMPLATE).await?;
    let page_template = load_template(&template_root, "page.html", DEFAULT_PAGE_TEMPLATE).await?;
    let index_template =
        load_template(&template_root, "index.html", DEFAULT_INDEX_TEMPLATE).await?;
    let rss_template = load_template(&template_root, "rss.xml", DEFAULT_RSS_TEMPLATE).await?;
    let atom_template = load_template(&template_root, "atom.xml", DEFAULT_ATOM_TEMPLATE).await?;
    let tag_template = load_template(&template_root, "tag.html", DEFAULT_TAG_TEMPLATE).await?;

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
            .all(&db)
            .await
            .map_err(|error: DbErr| error.to_string())?;
        for alias in aliases {
            routes.insert(alias.alias_path);
        }

        let template = if item.page_type == "post" {
            post_template.as_str()
        } else {
            page_template.as_str()
        };

        let html = markdown::to_html(&item.page_content);
        let rendered = apply_content_template(template, &item.title, &html, &item.slug);

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

    let rendered_index =
        apply_index_template(index_template.as_str(), &site.full_title, &index_rows);
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
    let rendered_rss = apply_rss_template(rss_template.as_str(), &site.full_title, &rss_items);
    fs::write(tmp_root.join("rss.xml"), rendered_rss.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    files_written = files_written.saturating_add(1);

    let atom_entries = render_atom_entries_xml(&post_items);
    let rendered_atom = apply_atom_template(
        atom_template.as_str(),
        &site.full_title,
        &post_items,
        &atom_entries,
    );
    fs::write(tmp_root.join("atom.xml"), rendered_atom.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    files_written = files_written.saturating_add(1);

    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(site.id.clone()))
        .all(&db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    for tag in tags {
        let mut tag_rows = String::new();
        let links = entities::content_tag::Entity::find()
            .filter(entities::content_tag::Column::TagId.eq(tag.id.clone()))
            .all(&db)
            .await
            .map_err(|error: DbErr| error.to_string())?;

        for link in links {
            if let Some(content) = entities::content_item::Entity::find_by_id(link.content_id)
                .one(&db)
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

        let tag_output = apply_tag_template(&tag_template, &tag.name, &tag_rows);
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
        &db,
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

    let _ = db.close().await;
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

fn apply_content_template(template: &str, title: &str, content: &str, slug: &str) -> String {
    template
        .replace("{{title}}", title)
        .replace("{{content}}", content)
        .replace("{{slug}}", slug)
}

fn apply_index_template(template: &str, site: &str, items: &str) -> String {
    template
        .replace("{{site}}", site)
        .replace("{{items}}", items)
}

fn apply_tag_template(template: &str, tag_name: &str, items: &str) -> String {
    template
        .replace("{{tag}}", tag_name)
        .replace("{{items}}", items)
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

fn apply_rss_template(template: &str, site: &str, items: &str) -> String {
    let updated = Utc::now().to_rfc2822();
    template
        .replace("{{site}}", site)
        .replace("{{link}}", "/")
        .replace("{{updated}}", updated.as_str())
        .replace("{{items}}", items)
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

fn apply_atom_template(
    template: &str,
    site: &str,
    content_items: &[entities::content_item::Model],
    entries: &str,
) -> String {
    let updated = content_items
        .first()
        .map(content_publish_timestamp)
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    template
        .replace("{{site}}", site)
        .replace("{{link}}", "/")
        .replace("{{updated}}", updated.as_str())
        .replace("{{entries}}", entries)
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

/// Creates a site record and returns the persisted row.
pub async fn create_site(
    database_url: &str,
    short_name: String,
    full_title: String,
    template_name: String,
) -> StdResult<entities::site::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let model = entities::site::ActiveModel {
        id: Set(uuid_v7()),
        short_name: Set(short_name),
        full_title: Set(full_title),
        template_name: Set(template_name),
        created_at: Set(now.clone()),
        updated_at: Set(now),
    };

    let model = model.insert(&db).await.map_err(|error| error.to_string())?;
    let _ = db.close().await;

    Ok(model)
}

/// Returns all sites ordered by short name.
pub async fn list_sites(database_url: &str) -> StdResult<Vec<entities::site::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let sites = entities::site::Entity::find()
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
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
    database_url: &str,
    input: NewContent,
) -> StdResult<entities::content_item::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

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
    .insert(&db)
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
    .insert(&db)
    .await
    .map_err(|error| error.to_string())?;

    drop(revision);
    let _ = db.close().await;
    Ok(content)
}

/// Updates a content row and appends a revision snapshot.
pub async fn update_content(
    database_url: &str,
    input: UpdateContent,
) -> StdResult<entities::content_item::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let now = utc_now();
    let existing = entities::content_item::Entity::find_by_id(&input.content_id)
        .one(&db)
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
        .update(&db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    let revisions = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(input.content_id.as_str()))
        .all(&db)
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
    .insert(&db)
    .await
    .map_err(|error| error.to_string())?;

    let content_tags = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content.id.clone()))
        .all(&db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    for content_tag in content_tags {
        entities::content_revision_tag::ActiveModel {
            id: Set(uuid_v7()),
            revision_id: Set(revision.id.clone()),
            tag_id: Set(content_tag.tag_id),
        }
        .insert(&db)
        .await
        .map_err(|error: DbErr| error.to_string())?;
    }

    let aliases = entities::content_alias::Entity::find()
        .filter(entities::content_alias::Column::ContentId.eq(content.id.clone()))
        .all(&db)
        .await
        .map_err(|error: DbErr| error.to_string())?;

    for alias in aliases {
        let _revision_alias = entities::content_revision_alias::ActiveModel {
            id: Set(uuid_v7()),
            revision_id: Set(revision.id.clone()),
            alias_path: Set(alias.alias_path),
            kind: Set(alias.kind),
        }
        .insert(&db)
        .await
        .map_err(|error: DbErr| error.to_string())?;
    }

    let _ = db.close().await;
    Ok(content)
}

/// Returns all content records for a site, optionally filtered by page_type.
pub async fn list_content(
    database_url: &str,
    site_id: &str,
    page_type: Option<&str>,
) -> StdResult<Vec<entities::content_item::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let query = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id.to_owned()));
    let query = if let Some(filter) = page_type {
        query.filter(entities::content_item::Column::PageType.eq(filter.to_owned()))
    } else {
        query
    };

    let content = query.all(&db).await.map_err(|error| error.to_string())?;
    let _ = db.close().await;
    Ok(content)
}

/// Creates a content alias entry.
pub async fn create_alias(
    database_url: &str,
    input: NewAlias,
) -> StdResult<entities::content_alias::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let model = entities::content_alias::ActiveModel {
        id: Set(uuid_v7()),
        content_id: Set(input.content_id),
        site_id: Set(input.site_id),
        alias_path: Set(input.alias_path),
        kind: Set(input.kind),
    }
    .insert(&db)
    .await
    .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(model)
}

/// Returns all content aliases, optionally scoped to content_id.
pub async fn list_aliases(
    database_url: &str,
    site_id: &str,
    content_id: Option<&str>,
) -> StdResult<Vec<entities::content_alias::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let query = entities::content_alias::Entity::find()
        .filter(entities::content_alias::Column::SiteId.eq(site_id.to_owned()));
    let query = if let Some(content_id) = content_id {
        query.filter(entities::content_alias::Column::ContentId.eq(content_id.to_owned()))
    } else {
        query
    };

    let aliases = query.all(&db).await.map_err(|error| error.to_string())?;
    let _ = db.close().await;
    Ok(aliases)
}

/// Creates a tag record.
pub async fn create_tag(
    database_url: &str,
    input: NewTag,
) -> StdResult<entities::tag::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let model = entities::tag::ActiveModel {
        id: Set(uuid_v7()),
        site_id: Set(input.site_id),
        name: Set(input.name),
    };

    let model = model.insert(&db).await.map_err(|error| error.to_string())?;
    let _ = db.close().await;
    Ok(model)
}

/// Returns all tags for a site.
pub async fn list_tags(
    database_url: &str,
    site_id: &str,
) -> StdResult<Vec<entities::tag::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(site_id.to_owned()))
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(tags)
}

/// Adds a tag to content (creates the tag if missing).
pub async fn add_content_tag(
    database_url: &str,
    input: NewContentTag,
) -> StdResult<entities::content_tag::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let content = entities::content_item::Entity::find_by_id(&input.content_id)
        .one(&db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "content not found".to_string())?;

    if content.site_id != input.site_id {
        let _ = db.close().await;
        return Err("content does not belong to provided site".to_string());
    }

    let tag = if let Some(tag) = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(input.site_id.as_str()))
        .filter(entities::tag::Column::Name.eq(input.tag_name.clone()))
        .one(&db)
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
        .insert(&db)
        .await
        .map_err(|error| error.to_string())?
    };

    let model = entities::content_tag::ActiveModel {
        id: Set(uuid_v7()),
        content_id: Set(input.content_id),
        tag_id: Set(tag.id),
    }
    .insert(&db)
    .await
    .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(model)
}

/// Returns tags currently attached to a content item.
pub async fn list_content_tags(
    database_url: &str,
    content_id: &str,
) -> StdResult<Vec<entities::tag::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let links = entities::content_tag::Entity::find()
        .filter(entities::content_tag::Column::ContentId.eq(content_id.to_owned()))
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    if links.is_empty() {
        let _ = db.close().await;
        return Ok(Vec::new());
    }

    let tag_ids = links
        .into_iter()
        .map(|link| link.tag_id)
        .collect::<Vec<_>>();
    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::Id.is_in(tag_ids))
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(tags)
}

/// Returns all aliases for a specific content revision.
pub async fn list_revision_aliases(
    database_url: &str,
    revision_id: &str,
) -> StdResult<Vec<entities::content_revision_alias::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let revision_aliases = entities::content_revision_alias::Entity::find()
        .filter(entities::content_revision_alias::Column::RevisionId.eq(revision_id.to_owned()))
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(revision_aliases)
}

/// Returns all tags captured for a specific content revision.
pub async fn list_revision_tags(
    database_url: &str,
    revision_id: &str,
) -> StdResult<Vec<entities::tag::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let links = entities::content_revision_tag::Entity::find()
        .filter(entities::content_revision_tag::Column::RevisionId.eq(revision_id.to_owned()))
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    if links.is_empty() {
        let _ = db.close().await;
        return Ok(Vec::new());
    }

    let tag_ids = links
        .into_iter()
        .map(|link| link.tag_id)
        .collect::<Vec<_>>();
    let tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::Id.is_in(tag_ids))
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(tags)
}

/// Returns all revisions for a content item sorted by revision number.
pub async fn list_revisions(
    database_url: &str,
    content_id: &str,
) -> StdResult<Vec<entities::content_revision::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let revisions = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(content_id.to_owned()))
        .order_by_asc(entities::content_revision::Column::RevisionNumber)
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(revisions)
}

fn content_primary_route(content: &entities::content_item::Model) -> String {
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
    database_url: &str,
    input: NewUser,
) -> StdResult<entities::user::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let model = entities::user::ActiveModel {
        id: Set(uuid_v7()),
        subject: Set(input.subject),
        created_at: Set(utc_now()),
        last_login_at: Set(None),
    };

    let model = model.insert(&db).await.map_err(|error| error.to_string())?;
    let _ = db.close().await;
    Ok(model)
}

/// Returns all users.
pub async fn list_users(database_url: &str) -> StdResult<Vec<entities::user::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let users = entities::user::Entity::find()
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(users)
}

/// Creates one site membership record.
pub async fn create_membership(
    database_url: &str,
    input: NewMembership,
) -> StdResult<entities::site_membership::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let model = entities::site_membership::ActiveModel {
        id: Set(uuid_v7()),
        site_id: Set(input.site_id),
        user_id: Set(input.user_id),
        role: Set(input.role),
    };

    let model = model.insert(&db).await.map_err(|error| error.to_string())?;
    let _ = db.close().await;
    Ok(model)
}

/// Returns memberships for a site.
pub async fn list_memberships(
    database_url: &str,
    site_id: &str,
) -> StdResult<Vec<entities::site_membership::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let memberships = entities::site_membership::Entity::find()
        .filter(entities::site_membership::Column::SiteId.eq(site_id.to_owned()))
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(memberships)
}

/// Creates an asset record.
pub async fn create_asset(
    database_url: &str,
    input: NewAsset,
) -> StdResult<entities::asset::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

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

    let model = model.insert(&db).await.map_err(|error| error.to_string())?;
    let _ = db.close().await;
    Ok(model)
}

/// Returns all assets for a site.
pub async fn list_assets(
    database_url: &str,
    site_id: &str,
) -> StdResult<Vec<entities::asset::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let assets = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id.to_owned()))
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(assets)
}

/// Creates an asset variant entry.
pub async fn create_asset_variant(
    database_url: &str,
    input: NewAssetVariant,
) -> StdResult<entities::asset_variant::Model, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

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

    let model = model.insert(&db).await.map_err(|error| error.to_string())?;
    let _ = db.close().await;
    Ok(model)
}

/// Returns all variants for an asset.
pub async fn list_asset_variants(
    database_url: &str,
    asset_id: &str,
) -> StdResult<Vec<entities::asset_variant::Model>, String> {
    let db = Database::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;

    let variants = entities::asset_variant::Entity::find()
        .filter(entities::asset_variant::Column::AssetId.eq(asset_id.to_owned()))
        .all(&db)
        .await
        .map_err(|error| error.to_string())?;

    let _ = db.close().await;
    Ok(variants)
}
