use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use url::Url;
use uuid::Uuid;

use crate::entities::audit_event::log_audit_event;
use crate::*;

#[derive(Debug, Args, Clone)]
pub struct OidcConfig {
    /// Path to the TLS certificate file.
    #[arg(
        long = "tls-cert-path",
        env = "WEBSITES_TLS_CERT_PATH",
        value_name = "FILE"
    )]
    pub tls_cert_path: PathBuf,

    /// Path to the TLS private key file.
    #[arg(
        long = "tls-key-path",
        env = "WEBSITES_TLS_KEY_PATH",
        value_name = "FILE"
    )]
    pub tls_key_path: PathBuf,

    /// Public frontend URL for OIDC redirect and callback configuration.
    #[arg(
        long = "frontend-url",
        env = "WEBSITES_FRONTEND_URL",
        value_name = "URL"
    )]
    pub frontend_url: Url,

    /// OIDC client ID.
    #[arg(
        long = "client-id",
        env = "WEBSITES_OIDC_CLIENT_ID",
        value_name = "STRING"
    )]
    pub oidc_client_id: Option<String>,

    /// OIDC discovery document URL.
    #[arg(
        long = "discovery-url",
        env = "WEBSITES_OIDC_DISCOVERY_URL",
        value_name = "URL"
    )]
    pub oidc_discovery_url: Option<String>,
}

#[derive(Debug, Parser)]
#[command(
    name = "websites",
    about = "Rust static site management platform",
    version
)]
pub struct Cli {
    #[arg(
        long = "database-url",
        default_value = "./database.sqlite",
        help = "SQLite database path for the management database"
    )]
    pub db_path: PathBuf,
    #[command(flatten)]
    pub oidc: OidcConfig,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Initialize required schema tables.
    Init,
    /// Show effective OIDC configuration loaded from CLI flags and env vars.
    ShowConfig,
    /// Start the admin web UI server.
    Serve {
        #[command(subcommand)]
        command: ServeCommands,
    },
    /// Manage managed sites.
    Site {
        #[command(subcommand)]
        command: SiteCommands,
    },
    /// Manage users.
    User {
        #[command(subcommand)]
        command: UserCommands,
    },
    /// Manage assets.
    Asset {
        #[command(subcommand)]
        command: AssetCommands,
    },
    /// Inspect audit log.
    Audit {
        #[command(subcommand)]
        command: AuditCommands,
    },
    /// Manage content.
    Content {
        #[command(subcommand)]
        command: ContentCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum ServeCommands {
    /// Start the admin web UI server.
    Admin {
        #[arg(long, default_value = "127.0.0.1:9000")]
        listen: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuditCommands {
    /// List recent audit events.
    List {
        #[arg(long)]
        site_id: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SiteCommands {
    /// Create a site record.
    Create {
        #[arg(long)]
        short_name: String,
        #[arg(long)]
        full_title: String,
        #[arg(long, default_value = "default")]
        template_name: String,
    },
    /// List all sites.
    List,
    // /// Add a user membership to a site.
    MemberAdd {
        #[arg(long)]
        site_id: String,
        #[arg(long)]
        user_id: String,
        #[arg(long, value_parser = ["owner", "editor", "author", "viewer"])]
        role: String,
    },
    /// List site memberships.
    MemberList {
        #[arg(long)]
        site_id: String,
    },
    /// Create a site tag.
    TagCreate {
        #[arg(long)]
        site_id: String,
        #[arg(long)]
        name: String,
    },
    /// List site tags.
    TagList {
        #[arg(long)]
        site_id: String,
    },
    /// Render published content to the rendered output.
    Render {
        #[arg(long)]
        site_id: String,
        #[arg(long, default_value = "templates")]
        templates_dir: String,
        #[arg(long, default_value = "./rendered")]
        rendered_dir: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum UserCommands {
    /// Create a user record.
    Create {
        #[arg(long)]
        subject: String,
    },
    /// List users.
    List,
}

#[derive(Debug, Subcommand)]
pub enum AssetCommands {
    /// Create an asset record.
    Create {
        #[arg(long)]
        site_id: String,
        #[arg(long)]
        uploader_sub: String,
        #[arg(long)]
        original_filename: String,
        #[arg(long)]
        storage_basename: String,
        #[arg(long)]
        mime_type: String,
        #[arg(long)]
        byte_length: i32,
        #[arg(long)]
        width: Option<i32>,
        #[arg(long)]
        height: Option<i32>,
    },
    /// List assets for a site.
    List {
        #[arg(long)]
        site_id: String,
    },
    /// Create a derivative variant for an asset.
    VariantCreate {
        #[arg(long)]
        asset_id: String,
        #[arg(long, value_parser = ["original", "thumbnail"])]
        variant_kind: String,
        #[arg(long)]
        filename: String,
        #[arg(long)]
        mime_type: String,
        #[arg(long)]
        byte_length: i32,
        #[arg(long)]
        width: Option<i32>,
        #[arg(long)]
        height: Option<i32>,
    },
    /// List variants for an asset.
    VariantList {
        #[arg(long)]
        asset_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ContentCommands {
    /// Create a content item.
    Create {
        #[arg(long)]
        site_id: String,
        #[arg(long, value_parser = ["post", "page"])]
        page_type: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        slug: String,
        #[arg(long)]
        page_content: String,
        #[arg(long)]
        creator_sub: String,
        #[arg(long, default_value_t = false)]
        draft: bool,
        #[arg(long)]
        published_at: Option<String>,
    },
    /// List content for a site.
    List {
        #[arg(long)]
        site_id: String,
        #[arg(long)]
        page_type: Option<String>,
    },
    /// Create a content alias.
    AliasCreate {
        #[arg(long)]
        content_id: String,
        #[arg(long)]
        site_id: String,
        #[arg(long)]
        alias_path: String,
        #[arg(long, value_parser = ["primary", "alias"], default_value = "alias")]
        kind: String,
    },
    /// List aliases for a site.
    AliasList {
        #[arg(long)]
        site_id: String,
        #[arg(long)]
        content_id: Option<String>,
    },
    /// List revision history for one content id.
    Revisions {
        #[arg(long)]
        content_id: String,
    },
    /// Show aliases captured for a revision.
    RevisionAliases {
        #[arg(long)]
        revision_id: String,
    },
    /// Show tags captured for a revision.
    RevisionTags {
        #[arg(long)]
        revision_id: String,
    },
    /// Show a full content detail with derived URL, aliases, tags, and revision count.
    Inspect {
        #[arg(long)]
        content_id: String,
    },
    /// Update a content item and create a new revision.
    Update {
        #[arg(long)]
        content_id: String,
        #[arg(long, value_parser = ["post", "page"])]
        page_type: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        slug: Option<String>,
        #[arg(long)]
        page_content: Option<String>,
        #[arg(long)]
        draft: Option<bool>,
        #[arg(long)]
        published_at: Option<String>,
        #[arg(long)]
        editor_sub: String,
    },
    /// Add tag to content (creates tag if missing).
    TagAdd {
        #[arg(long)]
        content_id: String,
        #[arg(long)]
        site_id: String,
        #[arg(long)]
        tag_name: String,
    },
    /// List tags attached to a content item.
    TagList {
        #[arg(long)]
        content_id: String,
    },
    /// Import content from a WordPress WXR XML export.
    ImportWordpress {
        #[arg(long)]
        site_id: String,
        #[arg(long, value_name = "FILE")]
        file_path: String,
        #[arg(long)]
        creator_sub: String,
    },
}

pub async fn execute(command: Commands, db_path: &Path, oidc: &OidcConfig) -> Result<(), String> {
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    let db = crate::db::db_start(&db_url)
        .await
        .map_err(|err| format!("Failed to start database: {err}"))?;
    let db_ref = db.as_ref();
    match command {
        Commands::Init => {
            println!("database initialized: {}", db_path.display());
            Ok(())
        }
        Commands::ShowConfig => {
            println!("tls_cert_path={:?}", &oidc.tls_cert_path.display());
            println!("tls_key_path={:?}", &oidc.tls_key_path.display());
            println!("frontend_url={}", &oidc.frontend_url);
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
                let site = create_site(db_ref, short_name, full_title, template_name).await?;
                let _ = log_audit_event(
                    db_ref,
                    "system",
                    "create_site",
                    "site",
                    &site.id.to_string(),
                    Some(site.id),
                    Some(json!({
                        "short_name": &site.short_name,
                        "full_title": &site.full_title
                    })),
                )
                .await?;
                println!("created site: {} ({})", site.id, site.short_name);
                Ok(())
            }
            SiteCommands::List => {
                let sites = list_sites(db_ref).await?;
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
                let site_id = parse_uuid(&site_id, "site_id")?;
                let user_id = parse_uuid(&user_id, "user_id")?;
                let membership = create_membership(
                    db_ref,
                    NewMembership {
                        site_id,
                        user_id,
                        role,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    db_ref,
                    "system",
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
                .await?;
                println!("created membership: {} {}", membership.id, membership.role);
                Ok(())
            }
            SiteCommands::MemberList { site_id } => {
                let site_id = parse_uuid(&site_id, "site_id")?;
                let memberships = list_memberships(db_ref, site_id).await?;
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
                let site_id = parse_uuid(&site_id, "site_id")?;
                let tag = create_tag(db_ref, NewTag { site_id, name }).await?;
                let _ = log_audit_event(
                    db_ref,
                    "system",
                    "create_tag",
                    "tag",
                    &tag.id.to_string(),
                    Some(tag.site_id),
                    Some(json!({"name": &tag.name})),
                )
                .await?;
                println!("created tag: {} {}", tag.id, tag.name);
                Ok(())
            }
            SiteCommands::TagList { site_id } => {
                let site_id = parse_uuid(&site_id, "site_id")?;
                let tags = list_tags(db_ref, site_id).await?;
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
                let site_id = parse_uuid(&site_id, "site_id")?;
                let files_written =
                    render_site(db_ref, site_id, &templates_dir, &rendered_dir).await?;
                println!("rendered site {} files {}", site_id, files_written);
                Ok(())
            }
        },
        Commands::User { command } => match command {
            UserCommands::Create { subject } => {
                let user = create_user(db_ref, NewUser { subject }).await?;
                let _ = log_audit_event(
                    db_ref,
                    "system",
                    "create_user",
                    "user",
                    &user.id.to_string(),
                    None,
                    Some(json!({"subject": &user.subject})),
                )
                .await?;
                println!("created user: {} {}", user.id, user.subject);
                Ok(())
            }
            UserCommands::List => {
                let users = list_users(db_ref).await?;
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
                let site_id = parse_uuid(&site_id, "site_id")?;
                let asset = create_asset(
                    db_ref,
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
                    db_ref,
                    &asset.uploader_sub,
                    "create_asset",
                    "asset",
                    &asset.id.to_string(),
                    Some(asset.site_id),
                    Some(json!({
                        "original_filename": &asset.original_filename,
                        "storage_basename": &asset.storage_basename
                    })),
                )
                .await?;
                println!("created asset: {} {}", asset.id, asset.original_filename);
                Ok(())
            }
            AssetCommands::List { site_id } => {
                let site_id = parse_uuid(&site_id, "site_id")?;
                let assets = list_assets(db_ref, site_id).await?;
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
                let asset_id = parse_uuid(&asset_id, "asset_id")?;
                let variant = create_asset_variant(
                    db_ref,
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
                    db_ref,
                    "system",
                    "create_asset_variant",
                    "asset_variant",
                    &variant.id.to_string(),
                    Some(variant.asset_id),
                    Some(json!({
                        "variant_kind": &variant.variant_kind,
                        "filename": &variant.filename
                    })),
                )
                .await?;
                println!("created variant: {} {}", variant.id, variant.filename);
                Ok(())
            }
            AssetCommands::VariantList { asset_id } => {
                let asset_id = parse_uuid(&asset_id, "asset_id")?;
                let variants = list_asset_variants(db_ref, asset_id).await?;
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
                crate::web::run_admin_server(db.clone(), &listen, oidc)
                    .await
                    .map_err(|err| err.to_string())
            }
        },
        Commands::Audit { command } => match command {
            AuditCommands::List { site_id } => {
                let site_filter = site_id
                    .as_deref()
                    .map(|value| parse_uuid(value, "site_id"))
                    .transpose()?;
                let events = list_audit_events(db_ref, site_filter).await?;
                if events.is_empty() {
                    println!("no audit events");
                    return Ok(());
                }

                println!(
                    "id\tsite_id\tactor_sub\tevent_type\tentity_type\tentity_id\tcreated_at\tpayload_json"
                );
                for event in events {
                    let site_id = event
                        .site_id
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let payload = event
                        .payload_json
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "null".to_string());
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
                let site_id = parse_uuid(&site_id, "site_id")?;
                let page_type = PageType::from_str(&page_type)?;
                let published_at = parse_optional_datetime(published_at, "published_at")?;
                let content = create_content(
                    db_ref,
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
                    db_ref,
                    &content.creator_sub,
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
                .await?;
                println!("created content: {} {}", content.id, content.title);
                Ok(())
            }
            ContentCommands::List { site_id, page_type } => {
                let site_id = parse_uuid(&site_id, "site_id")?;
                let page_filter = page_type.as_deref().map(PageType::from_str).transpose()?;
                let content = list_content(db_ref, site_id, page_filter).await?;
                if content.is_empty() {
                    println!("no content");
                    return Ok(());
                }

                println!("id\ttitle\tslug\tpage_type\tdraft\tcreated_at\tpublished_at\turl");
                for row in content {
                    let published_at = row
                        .published_at
                        .as_ref()
                        .map_or_else(|| "n/a".to_string(), |value| value.to_rfc3339());
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
                let content_id = parse_uuid(&content_id, "content_id")?;
                let site_id = parse_uuid(&site_id, "site_id")?;
                let alias = create_alias(
                    db_ref,
                    NewAlias {
                        content_id,
                        site_id,
                        alias_path,
                        kind,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    db_ref,
                    "system",
                    "create_alias",
                    "content_alias",
                    &alias.id.to_string(),
                    Some(alias.site_id),
                    Some(json!({
                        "content_id": alias.content_id.to_string(),
                        "alias_path": &alias.alias_path,
                        "kind": &alias.kind
                    })),
                )
                .await?;
                println!("created alias: {} {}", alias.id, alias.alias_path);
                Ok(())
            }
            ContentCommands::AliasList {
                site_id,
                content_id,
            } => {
                let site_id = parse_uuid(&site_id, "site_id")?;
                let content_filter = content_id
                    .as_deref()
                    .map(|value| parse_uuid(value, "content_id"))
                    .transpose()?;
                let aliases = list_aliases(db_ref, site_id, content_filter).await?;
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
                let content_id = parse_uuid(&content_id, "content_id")?;
                let revisions = list_revisions(db_ref, content_id).await?;
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
                let revision_id = parse_uuid(&revision_id, "revision_id")?;
                let aliases = list_revision_aliases(db_ref, revision_id).await?;
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
                let revision_id = parse_uuid(&revision_id, "revision_id")?;
                let tags = list_revision_tags(db_ref, revision_id).await?;
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
                let content_id = parse_uuid(&content_id, "content_id")?;
                let content = get_content(db_ref, content_id).await?;
                let aliases = list_aliases(db_ref, content.site_id, Some(content_id)).await?;
                let tags = list_content_tags(db_ref, content_id).await?;
                let revisions = list_revisions(db_ref, content_id).await?;

                let public_path = content_primary_route(&content);
                println!("id:\t{}", content.id);
                println!("site_id:\t{}", content.site_id);
                println!("title:\t{}", content.title);
                println!("slug:\t{}", content.slug);
                println!("page_type:\t{}", content.page_type);
                println!("draft:\t{}", content.draft);
                println!(
                    "published_at:\t{}",
                    content
                        .published_at
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "n/a".to_string())
                );
                println!("created_at:\t{}", content.created_at);
                println!(
                    "updated_at:\t{}",
                    content
                        .last_updated
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "n/a".to_string())
                );
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
                let content_id = parse_uuid(&content_id, "content_id")?;
                let page_type = page_type.as_deref().map(PageType::from_str).transpose()?;
                let published_at = parse_optional_datetime(published_at, "published_at")?;
                let content = update_content(
                    db_ref,
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
                    db_ref,
                    &content.creator_sub,
                    "update_content",
                    "content_item",
                    &content.id.to_string(),
                    Some(content.site_id),
                    Some(json!({
                        "content_id": content.id.to_string(),
                        "page_type": content.page_type.to_string(),
                        "slug": &content.slug,
                        "title": &content.title
                    })),
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
                let content_id = parse_uuid(&content_id, "content_id")?;
                let site_id = parse_uuid(&site_id, "site_id")?;
                let content_tag = add_content_tag(
                    db_ref,
                    NewContentTag {
                        content_id,
                        site_id,
                        tag_name,
                    },
                )
                .await?;
                let _ = log_audit_event(
                    db_ref,
                    "system",
                    "add_content_tag",
                    "content_tag",
                    &content_tag.id.to_string(),
                    Some(content_tag.content_id),
                    Some(json!({
                        "content_id": content_tag.content_id.to_string(),
                        "tag_id": content_tag.tag_id.to_string()
                    })),
                )
                .await?;
                println!(
                    "linked content tag: {} {}",
                    content_tag.id, content_tag.tag_id
                );
                Ok(())
            }
            ContentCommands::TagList { content_id } => {
                let content_id = parse_uuid(&content_id, "content_id")?;
                let tags = list_content_tags(db_ref, content_id).await?;
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
                let site_id = parse_uuid(&site_id, "site_id")?;
                let imported = import_wordpress(db_ref, site_id, &file_path, &creator_sub).await?;
                let _ = log_audit_event(
                    db_ref,
                    &creator_sub,
                    "import_wordpress",
                    "content_item",
                    &site_id.to_string(),
                    Some(site_id),
                    Some(json!({"imported": imported})),
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

fn parse_uuid(value: &str, label: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value.trim()).map_err(|error| format!("Invalid {label} UUID: {error}"))
}

fn parse_optional_datetime(
    value: Option<String>,
    label: &str,
) -> Result<Option<DateTime<Utc>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed = DateTime::parse_from_rfc3339(trimmed)
        .map_err(|error| format!("Invalid {label} datetime: {error}"))?;
    Ok(Some(parsed.with_timezone(&Utc)))
}
