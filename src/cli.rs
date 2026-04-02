use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use sea_orm::{EntityTrait, TransactionTrait};
use serde_json::json;
use tokio::fs;
use url::Url;
use uuid::Uuid;

use crate::api_docs::dump_openapi_spec;
use crate::entities::audit_event::log_audit_event;
use crate::entities::user::create_user;
use crate::*;

#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub tls_cert_path: PathBuf,
    pub tls_key_path: PathBuf,
    pub frontend_url: Url,
    pub oidc_client_id: String,
    pub oidc_discovery_url: String,
    pub oidc_client_secret: Option<String>,
}

#[derive(Debug, Args, Clone, Default)]
pub struct OidcConfigArgs {
    /// Path to the TLS certificate file.
    #[arg(
        long = "tls-cert-path",
        env = "WEBSITES_TLS_CERT_PATH",
        value_name = "FILE"
    )]
    pub tls_cert_path: Option<PathBuf>,

    /// Path to the TLS private key file.
    #[arg(
        long = "tls-key-path",
        env = "WEBSITES_TLS_KEY_PATH",
        value_name = "FILE"
    )]
    pub tls_key_path: Option<PathBuf>,

    /// Public frontend URL for OIDC redirect and callback configuration.
    #[arg(
        long = "frontend-url",
        env = "WEBSITES_FRONTEND_URL",
        value_name = "URL"
    )]
    pub frontend_url: Option<Url>,

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

    /// OIDC client secret, this is optional. If not provided, the OIDC client will be configured without a secret which may be appropriate for public clients that don't have a secure way to store a secret.
    #[arg(
        long = "client-secret",
        env = "WEBSITES_OIDC_CLIENT_SECRET",
        value_name = "CLIENT_SECRET"
    )]
    pub oidc_client_secret: Option<String>,
}

impl OidcConfigArgs {
    fn require_complete(&self) -> Result<OidcConfig, String> {
        Ok(OidcConfig {
            tls_cert_path: self
                .tls_cert_path
                .clone()
                .ok_or_else(|| "missing required OIDC/TLS config: --tls-cert-path".to_string())?,
            tls_key_path: self
                .tls_key_path
                .clone()
                .ok_or_else(|| "missing required OIDC/TLS config: --tls-key-path".to_string())?,
            frontend_url: self
                .frontend_url
                .clone()
                .ok_or_else(|| "missing required OIDC/TLS config: --frontend-url".to_string())?,
            oidc_client_id: self
                .oidc_client_id
                .clone()
                .ok_or_else(|| "missing required OIDC/TLS config: --client-id".to_string())?,
            oidc_discovery_url: self
                .oidc_discovery_url
                .clone()
                .ok_or_else(|| "missing required OIDC/TLS config: --discovery-url".to_string())?,
            oidc_client_secret: self.oidc_client_secret.clone(),
        })
    }
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
        env = "WEBSITES_DB_PATH",
        default_value = "./database.sqlite",
        help = "SQLite database path for the management database"
    )]
    pub db_path: PathBuf,
    #[arg(
        long = "site-templates-dir",
        env = "WEBSITES_SITE_TEMPLATES_DIR",
        default_value = crate::constants::SITE_TEMPLATES_DIR,
        value_name = "DIR",
        help = "Root directory containing site templates"
    )]
    pub site_templates_dir: PathBuf,
    #[arg(
        long = "upload-dir",
        env = "WEBSITES_UPLOAD_ROOT",
        value_name = "DIR",
        help = "Root directory where uploaded media files are written"
    )]
    pub upload_dir: Option<PathBuf>,
    #[arg(
        long = "rendered-dir",
        env = "WEBSITES_RENDERED_DIR",
        default_value = crate::constants::RENDERED_DIR,
        value_name = "DIR",
        help = "Root directory where rendered site output is written"
    )]
    pub rendered_dir: PathBuf,
    #[arg(
        long = "log-path",
        env = "WEBSITES_LOG_PATH",
        default_value = crate::constants::LOG_PATH,
        value_name = "FILE",
        help = "File path where persistent application logs are written"
    )]
    pub log_path: PathBuf,
    #[command(flatten)]
    pub oidc: OidcConfigArgs,
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
    /// Dump the OpenAPI spec to a file
    DumpOpenApiSpec {
        #[arg(value_name = "FILE")]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum ServeCommands {
    /// Start the admin web UI server.
    Admin {
        #[arg(long, default_value = "127.0.0.1:9000", env = "WEBSITES_LISTEN_ADDR")]
        listen: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuditCommands {
    /// List recent audit events.
    List {
        #[arg(long)]
        site_id: Option<Uuid>,
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
        site_id: Uuid,
        #[arg(long)]
        user_id: Uuid,
        #[arg(long, value_parser = ["owner", "editor", "author", "viewer"])]
        role: String,
    },
    /// List site memberships.
    MemberList {
        #[arg(long)]
        site_id: Uuid,
    },
    /// Create a site tag.
    TagCreate {
        #[arg(long)]
        site_id: Uuid,
        #[arg(long)]
        name: String,
    },
    /// List site tags.
    TagList {
        #[arg(long)]
        site_id: Uuid,
    },
    /// Render published content to the rendered output.
    Render {
        #[arg(long)]
        site_id: Uuid,
        #[arg(long)]
        templates_dir: Option<PathBuf>,
    },
    /// Export site data as a versioned JSON document.
    Export {
        #[arg(long)]
        site_id: Uuid,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Import a site from a versioned JSON document.
    Import {
        #[arg(long, value_name = "FILE")]
        input: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum UserCommands {
    /// Create a user record.
    Create {
        #[arg(long)]
        subject: String,
        #[arg(long)]
        email: Option<String>,
        #[arg(
            long,
            default_value = "false",
            help = "Create the user with system-admin level permissions"
        )]
        admin: bool,
    },
    /// List users.
    List,
}

#[derive(Debug, Subcommand)]
pub enum AssetCommands {
    /// Create an asset record.
    Create {
        #[arg(long)]
        site_id: Uuid,
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
        site_id: Uuid,
    },
    /// Create a derivative variant for an asset.
    VariantCreate {
        #[arg(long)]
        asset_id: Uuid,
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
        asset_id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
pub enum ContentCommands {
    /// Create a content item.
    Create {
        #[arg(long)]
        site_id: Uuid,
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
        site_id: Uuid,
        #[arg(long)]
        page_type: Option<String>,
    },
    /// Create a content alias.
    AliasCreate {
        #[arg(long)]
        content_id: Uuid,
        #[arg(long)]
        site_id: Uuid,
        #[arg(long)]
        alias_path: String,
        #[arg(long, value_parser = ["primary", "alias"], default_value = "alias")]
        kind: String,
    },
    /// List aliases for a site.
    AliasList {
        #[arg(long)]
        site_id: Uuid,
        #[arg(long)]
        content_id: Option<Uuid>,
    },
    /// List revision history for one content id.
    Revisions {
        #[arg(long)]
        content_id: Uuid,
    },
    /// Show aliases captured for a revision.
    RevisionAliases {
        #[arg(long)]
        revision_id: Uuid,
    },
    /// Show tags captured for a revision.
    RevisionTags {
        #[arg(long)]
        revision_id: Uuid,
    },
    /// Show a full content detail with derived URL, aliases, tags, and revision count.
    Inspect {
        #[arg(long)]
        content_id: Uuid,
    },
    /// Update a content item and create a new revision.
    Update {
        #[arg(long)]
        content_id: Uuid,
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
        content_id: Uuid,
        #[arg(long)]
        site_id: Uuid,
        #[arg(long)]
        tag_name: String,
    },
    /// List tags attached to a content item.
    TagList {
        #[arg(long)]
        content_id: Uuid,
    },
    /// Import content from a WordPress WXR XML export.
    ImportWordpress {
        #[arg(long)]
        site_id: Uuid,
        #[arg(long, value_name = "FILE", required = true)]
        file_path: Vec<String>,
        #[arg(long)]
        creator_sub: String,
    },
}

pub async fn execute(
    command: Commands,
    db_path: &Path,
    site_templates_dir: &Path,
    upload_root: &Path,
    rendered_dir: &Path,
    log_path: &Path,
    oidc: &OidcConfigArgs,
) -> Result<(), String> {
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
            let tls_cert_path = oidc
                .tls_cert_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<unset>".to_string());
            let tls_key_path = oidc
                .tls_key_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<unset>".to_string());
            let frontend_url = oidc
                .frontend_url
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "<unset>".to_string());
            let oidc_client_id = oidc
                .oidc_client_id
                .clone()
                .unwrap_or_else(|| "<unset>".to_string());
            let oidc_discovery_url = oidc
                .oidc_discovery_url
                .clone()
                .unwrap_or_else(|| "<unset>".to_string());

            println!("tls_cert_path={tls_cert_path}");
            println!("tls_key_path={tls_key_path}");
            println!("frontend_url={frontend_url}");
            println!("oidc_client_id={oidc_client_id}");
            println!("oidc_discovery_url={oidc_discovery_url}");
            println!("site_templates_dir={}", site_templates_dir.display());
            println!("upload_dir={}", upload_root.display());
            println!("rendered_dir={}", rendered_dir.display());
            println!("log_path={}", log_path.display());
            Ok(())
        }
        Commands::Site { command } => match command {
            SiteCommands::Create {
                short_name,
                full_title,
                template_name,
            } => {
                let txn = db_ref
                    .begin()
                    .await
                    .map_err(|error| format!("failed to begin transaction: {error}"))?;

                let site = create_site(&txn, short_name, full_title, template_name)
                    .await
                    .map_err(|error| format!("failed to create site: {error}"))?;
                log_audit_event(
                    &txn,
                    "system",
                    "create_site",
                    "site",
                    &site.id,
                    Some(site.id),
                    Some(json!({
                        "short_name": &site.short_name,
                        "full_title": &site.full_title
                    })),
                )
                .await
                .map_err(|err| format!("Failed to create audit event: {}", err))?;
                txn.commit().await.map_err(|error| {
                    format!("failed to commit transaction, rolling back: {error}")
                })?;
                println!("created site: {} ({})", site.id, site.short_name);
                Ok(())
            }
            SiteCommands::List => {
                let sites = list_sites(db_ref)
                    .await
                    .map_err(|err| format!("failed to get sites {err}"))?;
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
                let role = match role.as_str() {
                    "viewer" => SiteRole::Viewer,
                    "author" => SiteRole::Author,
                    "editor" => SiteRole::Editor,
                    "owner" => SiteRole::Owner,
                    _ => return Err(format!("invalid site role: {role}")),
                };
                let membership = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let membership = create_membership(
                                txn,
                                NewMembership {
                                    site_id,
                                    user_id,
                                    role,
                                },
                            )
                            .await
                            .map_err(|error| error.to_string())?;
                            log_audit_event(
                                txn,
                                "system",
                                "create_membership",
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
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(membership)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to create membership: {error}"))?;
                println!(
                    "created membership: {} {}",
                    membership.id,
                    membership.role.label()
                );
                Ok(())
            }
            SiteCommands::MemberList { site_id } => {
                let memberships = list_memberships(db_ref, site_id).await?;
                if memberships.is_empty() {
                    println!("no memberships");
                    return Ok(());
                }

                println!("id\tsite_id\tuser_id\trole");
                for row in memberships {
                    println!(
                        "{}\t{}\t{}\t{}",
                        row.id,
                        row.site_id,
                        row.user_id,
                        row.role.label()
                    );
                }
                Ok(())
            }
            SiteCommands::TagCreate { site_id, name } => {
                let tag = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let tag = create_tag(txn, NewTag { site_id, name })
                                .await
                                .map_err(|error| format!("failed to create tag: {error}"))?;
                            log_audit_event(
                                txn,
                                "system",
                                "create_tag",
                                "tag",
                                &tag.id,
                                Some(tag.site_id),
                                Some(json!({"name": &tag.name})),
                            )
                            .await
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(tag)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to create tag: {error}"))?;
                println!("created tag: {} {}", tag.id, tag.name);
                Ok(())
            }
            SiteCommands::TagList { site_id } => {
                let tags = list_tags(db_ref, site_id)
                    .await
                    .map_err(|err| format!("Failed to list tags: {err}"))?;
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
            } => {
                let templates_dir =
                    resolve_site_render_templates_dir(templates_dir, site_templates_dir);
                let files_written =
                    render_site(db_ref, site_id, &templates_dir, rendered_dir, upload_root)
                        .await
                        .map_err(|err| err.to_string())?;
                println!("rendered site {} files {}", site_id, files_written);
                Ok(())
            }
            SiteCommands::Export { site_id, output } => {
                let override_root =
                    resolve_site_template_override_root_with_upload_root(upload_root, site_id);
                let export = export_site_with_roots(db_ref, site_id, upload_root, &override_root)
                    .await
                    .map_err(|error| format!("failed to export site: {error}"))?;
                let json = serialize_site_export_pretty(&export)
                    .map_err(|error| format!("failed to serialize site export: {error}"))?;
                write_site_export_output(output.as_deref(), &json)
                    .await
                    .map_err(|error| format!("failed to write site export: {error}"))?;
                Ok(())
            }
            SiteCommands::Import { input } => {
                let json = fs::read(&input)
                    .await
                    .map_err(|error| format!("failed to read site import file: {error}"))?;
                let result = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let result = import_site_json(txn, &json)
                                .await
                                .map_err(|error| format!("failed to import site: {error}"))?;
                            log_audit_event(
                                txn,
                                "system",
                                "import_site",
                                "site",
                                result.site_id,
                                Some(result.site_id),
                                Some(json!({
                                    "site_short_name": &result.site_short_name,
                                    "created_users": result.created_users,
                                    "reused_users": result.reused_users,
                                    "warnings": &result.warnings,
                                })),
                            )
                            .await
                            .map_err(|error| format!("Failed to create audit event: {error}"))?;
                            Ok(result)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to import site: {error}"))?;
                println!(
                    "imported site: {} ({})",
                    result.site_id, result.site_short_name
                );
                for warning in result.warnings {
                    println!("warning: {warning}");
                }
                Ok(())
            }
        },
        Commands::User { command } => match command {
            UserCommands::Create {
                subject,
                email,
                admin,
            } => {
                let user = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let user = create_user(txn, &subject, email.as_deref(), None, admin)
                                .await
                                .map_err(|error| error.to_string())?;
                            log_audit_event(
                                txn,
                                "system",
                                "create_user",
                                "user",
                                &user.id,
                                None,
                                Some(json!({"subject": &user.subject, "email": &user.email})),
                            )
                            .await
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(user)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to create user: {error}"))?;
                println!("created user: {} {}", user.id, user.subject);
                Ok(())
            }
            UserCommands::List => {
                let users = list_users(db_ref)
                    .await
                    .map_err(|err| format!("Failed to list users: {err}"))?;
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
                let asset = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let asset = create_asset(
                                txn,
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
                            log_audit_event(
                                txn,
                                &asset.uploader_sub,
                                "create_asset",
                                "asset",
                                &asset.id,
                                Some(asset.site_id),
                                Some(json!({
                                    "original_filename": &asset.original_filename,
                                    "storage_basename": &asset.storage_basename
                                })),
                            )
                            .await
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(asset)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to create asset: {error}"))?;
                println!("created asset: {} {}", asset.id, asset.original_filename);
                Ok(())
            }
            AssetCommands::List { site_id } => {
                let assets = list_assets(db_ref, site_id)
                    .await
                    .map_err(|err| format!("Failed to list assets: {err}"))?;
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
                let variant = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let asset = crate::entities::asset::Entity::find_by_id(asset_id)
                                .one(txn)
                                .await
                                .map_err(|error| {
                                    format!("failed to load asset for variant: {error}")
                                })?
                                .ok_or_else(|| {
                                    format!("asset not found for variant creation: {asset_id}")
                                })?;
                            let variant = create_asset_variant(
                                txn,
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
                            log_audit_event(
                                txn,
                                "system",
                                "create_asset_variant",
                                "asset_variant",
                                &variant.id,
                                Some(asset.site_id),
                                Some(json!({
                                    "variant_kind": &variant.variant_kind,
                                    "filename": &variant.filename
                                })),
                            )
                            .await
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(variant)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to create asset variant: {error}"))?;
                println!("created variant: {} {}", variant.id, variant.filename);
                Ok(())
            }
            AssetCommands::VariantList { asset_id } => {
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
                let oidc = oidc.require_complete()?;
                crate::web::run_admin_server(
                    db.clone(),
                    &listen,
                    &oidc,
                    site_templates_dir.to_path_buf(),
                    upload_root.to_path_buf(),
                    rendered_dir.to_path_buf(),
                    log_path.to_path_buf(),
                )
                .await
                .map_err(|err| err.to_string())
            }
        },
        Commands::Audit { command } => match command {
            AuditCommands::List { site_id } => {
                let events = list_audit_events(db_ref, site_id).await?;
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
                let page_type = PageType::from_str(&page_type)?;
                let published_at = parse_optional_datetime(published_at, "published_at")?;
                let content = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let content = create_content(
                                txn,
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
                            .await
                            .map_err(|err| err.to_string())?;
                            log_audit_event(
                                txn,
                                &content.creator_sub,
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
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(content)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to create content: {error}"))?;
                println!("created content: {} {}", content.id, content.title);
                Ok(())
            }
            ContentCommands::List { site_id, page_type } => {
                let page_filter = page_type.as_deref().map(PageType::from_str).transpose()?;
                let content = list_content(db_ref, site_id, page_filter, None).await?;
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
                let alias = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let alias = create_alias(
                                txn,
                                NewAlias {
                                    content_id,
                                    site_id,
                                    alias_path,
                                    kind,
                                },
                            )
                            .await
                            .map_err(|err| format!("Failed to create alias: {}", err))?;
                            log_audit_event(
                                txn,
                                "system",
                                "create_alias",
                                "content_alias",
                                &alias.id,
                                Some(alias.site_id),
                                Some(json!({
                                    "content_id": alias.content_id,
                                    "alias_path": &alias.alias_path,
                                    "kind": &alias.kind
                                })),
                            )
                            .await
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(alias)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to create alias: {error}"))?;
                println!("created alias: {} {}", alias.id, alias.alias_path);
                Ok(())
            }
            ContentCommands::AliasList {
                site_id,
                content_id,
            } => {
                let aliases = list_aliases(db_ref, site_id, content_id)
                    .await
                    .map_err(|err| format!("Failed to get aliases: {}", err))?;
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
                let content = entities::content_item::Entity::find_by_id(content_id)
                    .one(&*db)
                    .await
                    .map_err(|err| format!("failed to load content {content_id}: {err}"))?
                    .ok_or(format!("content not found: {content_id}"))?;
                let aliases = list_aliases(db_ref, content.site_id, Some(content_id))
                    .await
                    .map_err(|err| format!("Failed to list aliases: {err}"))?;
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
                let page_type = page_type.as_deref().map(PageType::from_str).transpose()?;
                let published_at = parse_optional_datetime(published_at, "published_at")?;
                let content = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let content = update_content(
                                txn,
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
                            log_audit_event(
                                txn,
                                &content.creator_sub,
                                "update_content",
                                "content_item",
                                &content.id,
                                Some(content.site_id),
                                Some(json!({
                                    "content_id": content.id,
                                    "page_type": content.page_type,
                                    "slug": &content.slug,
                                    "title": &content.title
                                })),
                            )
                            .await
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(content)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to update content: {error}"))?;
                println!("updated content: {} {}", content.id, content.title);
                Ok(())
            }
            ContentCommands::TagAdd {
                content_id,
                site_id,
                tag_name,
            } => {
                let content_tag = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let content_tag = add_content_tag(
                                txn,
                                NewContentTag {
                                    content_id,
                                    site_id,
                                    tag_name,
                                },
                            )
                            .await?;
                            log_audit_event(
                                txn,
                                "system",
                                "add_content_tag",
                                "content_tag",
                                &content_tag.id,
                                Some(content_tag.content_id),
                                Some(json!({
                                    "content_id": content_tag.content_id,
                                    "tag_id": content_tag.tag_id
                                })),
                            )
                            .await
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(content_tag)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to add content tag: {error}"))?;
                println!(
                    "linked content tag: {} {}",
                    content_tag.id, content_tag.tag_id
                );
                Ok(())
            }
            ContentCommands::TagList { content_id } => {
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
                let imported = db_ref
                    .transaction::<_, _, String>(|txn| {
                        Box::pin(async move {
                            let mut imported = 0usize;
                            for current_file in &file_path {
                                imported = imported.saturating_add(
                                    import_wordpress(txn, site_id, current_file, &creator_sub)
                                        .await
                                        .map_err(|err| err.to_string())?,
                                );
                            }
                            log_audit_event(
                                txn,
                                &creator_sub,
                                "import_wordpress",
                                "content_item",
                                &site_id,
                                Some(site_id),
                                Some(json!({"imported": imported, "files": file_path})),
                            )
                            .await
                            .map_err(|err| format!("Failed to create audit event: {}", err))?;
                            Ok(imported)
                        })
                    })
                    .await
                    .map_err(|error| format!("failed to import wordpress: {error}"))?;
                println!("imported {} wordpress items", imported);
                Ok(())
            }
        },
        Commands::DumpOpenApiSpec { output } => {
            dump_openapi_spec(&output)
                .await
                .map_err(|error| format!("failed to dump OpenAPI spec: {error}"))?;

            Ok(())
        }
    }
}

fn resolve_site_render_templates_dir(
    templates_dir: Option<PathBuf>,
    site_templates_dir: &Path,
) -> PathBuf {
    templates_dir.unwrap_or_else(|| site_templates_dir.to_path_buf())
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

async fn write_site_export_output(output: Option<&Path>, json: &str) -> Result<(), String> {
    if let Some(path) = output {
        fs::write(path, json.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
    } else {
        println!("{json}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[tokio::test]
    async fn write_site_export_output_writes_requested_file() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");
        let output = dir.path().join("site-export.json");
        let payload = "{\n  \"format_version\": 1\n}";

        write_site_export_output(Some(output.as_path()), payload)
            .await
            .expect("failed to write export output");

        let written = fs::read_to_string(&output)
            .await
            .expect("failed to read export output");
        assert_eq!(written, payload);
    }

    #[test]
    fn site_import_command_requires_input_path() {
        let cli =
            Cli::try_parse_from(["websites", "site", "import", "--input", "site-export.json"])
                .expect("failed to parse site import command");

        match cli.command.expect("missing command") {
            Commands::Site {
                command: SiteCommands::Import { input },
            } => assert_eq!(input, PathBuf::from("site-export.json")),
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[test]
    fn dump_openapi_spec_command_parses_without_oidc_flags() {
        let cli = Cli::try_parse_from(["websites", "dump-open-api-spec", "openapi.json"])
            .expect("failed to parse dump-open-api-spec command");

        match cli.command.expect("missing command") {
            Commands::DumpOpenApiSpec { output } => {
                assert_eq!(output, PathBuf::from("openapi.json"));
            }
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[test]
    fn database_path_can_be_loaded_from_env() {
        let _guard = env_lock().lock().expect("failed to lock env");
        let original = std::env::var_os("WEBSITES_DB_PATH");

        unsafe {
            std::env::set_var("WEBSITES_DB_PATH", "/tmp/websites-from-env.sqlite");
        }

        let cli = Cli::try_parse_from(["websites"])
            .expect("failed to parse CLI arguments with env override");

        assert_eq!(cli.db_path, PathBuf::from("/tmp/websites-from-env.sqlite"));

        match original {
            Some(value) => unsafe {
                std::env::set_var("WEBSITES_DB_PATH", value);
            },
            None => unsafe {
                std::env::remove_var("WEBSITES_DB_PATH");
            },
        }
    }

    #[test]
    fn site_templates_dir_can_be_loaded_from_flag() {
        let cli = Cli::try_parse_from([
            "websites",
            "--site-templates-dir",
            "/tmp/site-templates-from-flag",
        ])
        .expect("failed to parse CLI arguments with site templates flag");

        assert_eq!(
            cli.site_templates_dir,
            PathBuf::from("/tmp/site-templates-from-flag")
        );
    }

    #[test]
    fn site_templates_dir_can_be_loaded_from_env() {
        let _guard = env_lock().lock().expect("failed to lock env");
        let original = std::env::var_os("WEBSITES_SITE_TEMPLATES_DIR");

        unsafe {
            std::env::set_var(
                "WEBSITES_SITE_TEMPLATES_DIR",
                "/tmp/site-templates-from-env",
            );
        }

        let cli = Cli::try_parse_from(["websites"])
            .expect("failed to parse CLI arguments with site templates env override");

        assert_eq!(
            cli.site_templates_dir,
            PathBuf::from("/tmp/site-templates-from-env")
        );

        match original {
            Some(value) => unsafe {
                std::env::set_var("WEBSITES_SITE_TEMPLATES_DIR", value);
            },
            None => unsafe {
                std::env::remove_var("WEBSITES_SITE_TEMPLATES_DIR");
            },
        }
    }

    #[test]
    fn upload_dir_can_be_loaded_from_flag() {
        let cli = Cli::try_parse_from(["websites", "--upload-dir", "/tmp/uploads-from-flag"])
            .expect("failed to parse CLI arguments with upload flag");

        assert_eq!(
            cli.upload_dir,
            Some(PathBuf::from("/tmp/uploads-from-flag"))
        );
    }

    #[test]
    fn upload_dir_can_be_loaded_from_env() {
        let _guard = env_lock().lock().expect("failed to lock env");
        let original = std::env::var_os("WEBSITES_UPLOAD_ROOT");

        unsafe {
            std::env::set_var("WEBSITES_UPLOAD_ROOT", "/tmp/uploads-from-env");
        }

        let cli = Cli::try_parse_from(["websites"])
            .expect("failed to parse CLI arguments with upload env override");

        assert_eq!(cli.upload_dir, Some(PathBuf::from("/tmp/uploads-from-env")));

        match original {
            Some(value) => unsafe {
                std::env::set_var("WEBSITES_UPLOAD_ROOT", value);
            },
            None => unsafe {
                std::env::remove_var("WEBSITES_UPLOAD_ROOT");
            },
        }
    }

    #[test]
    fn rendered_dir_can_be_loaded_from_flag() {
        let cli = Cli::try_parse_from(["websites", "--rendered-dir", "/tmp/rendered-from-flag"])
            .expect("failed to parse CLI arguments with rendered flag");

        assert_eq!(cli.rendered_dir, PathBuf::from("/tmp/rendered-from-flag"));
    }

    #[test]
    fn rendered_dir_can_be_loaded_from_env() {
        let _guard = env_lock().lock().expect("failed to lock env");
        let original = std::env::var_os("WEBSITES_RENDERED_DIR");

        unsafe {
            std::env::set_var("WEBSITES_RENDERED_DIR", "/tmp/rendered-from-env");
        }

        let cli = Cli::try_parse_from(["websites"])
            .expect("failed to parse CLI arguments with rendered env override");

        assert_eq!(cli.rendered_dir, PathBuf::from("/tmp/rendered-from-env"));

        match original {
            Some(value) => unsafe {
                std::env::set_var("WEBSITES_RENDERED_DIR", value);
            },
            None => unsafe {
                std::env::remove_var("WEBSITES_RENDERED_DIR");
            },
        }
    }

    #[test]
    fn log_path_can_be_loaded_from_flag() {
        let cli = Cli::try_parse_from(["websites", "--log-path", "/tmp/logs-from-flag"])
            .expect("failed to parse CLI arguments with log path flag");

        assert_eq!(cli.log_path, PathBuf::from("/tmp/logs-from-flag"));
    }

    #[test]
    fn log_path_can_be_loaded_from_env() {
        let _guard = env_lock().lock().expect("failed to lock env");
        let original = std::env::var_os("WEBSITES_LOG_PATH");

        unsafe {
            std::env::set_var("WEBSITES_LOG_PATH", "/tmp/logs-from-env");
        }

        let cli = Cli::try_parse_from(["websites"])
            .expect("failed to parse CLI arguments with log path env override");

        assert_eq!(cli.log_path, PathBuf::from("/tmp/logs-from-env"));

        match original {
            Some(value) => unsafe {
                std::env::set_var("WEBSITES_LOG_PATH", value);
            },
            None => unsafe {
                std::env::remove_var("WEBSITES_LOG_PATH");
            },
        }
    }

    #[test]
    fn site_render_uses_global_templates_dir_when_command_override_is_missing() {
        let _guard = env_lock().lock().expect("failed to lock env");
        let cli = Cli::try_parse_from([
            "websites",
            "--site-templates-dir",
            "/tmp/site-templates-global",
            "site",
            "render",
            "--site-id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .expect("failed to parse site render command");
        let site_templates_dir = cli.site_templates_dir.clone();

        match cli.command.expect("missing command") {
            Commands::Site {
                command:
                    SiteCommands::Render {
                        site_id,
                        templates_dir,
                    },
            } => {
                assert_eq!(
                    site_id,
                    Uuid::parse_str("00000000-0000-0000-0000-000000000001")
                        .expect("failed to parse uuid")
                );
                assert!(templates_dir.is_none());
                let resolved =
                    resolve_site_render_templates_dir(templates_dir, &site_templates_dir);
                assert_eq!(resolved, PathBuf::from("/tmp/site-templates-global"));
                assert_eq!(
                    cli.rendered_dir,
                    PathBuf::from(crate::constants::RENDERED_DIR)
                );
            }
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[test]
    fn site_render_prefers_explicit_templates_dir_override() {
        let _guard = env_lock().lock().expect("failed to lock env");
        let cli = Cli::try_parse_from([
            "websites",
            "--site-templates-dir",
            "/tmp/site-templates-global",
            "site",
            "render",
            "--site-id",
            "00000000-0000-0000-0000-000000000001",
            "--templates-dir",
            "/tmp/site-templates-explicit",
        ])
        .expect("failed to parse site render command with explicit override");
        let site_templates_dir = cli.site_templates_dir.clone();

        match cli.command.expect("missing command") {
            Commands::Site {
                command: SiteCommands::Render { templates_dir, .. },
            } => {
                assert_eq!(
                    templates_dir,
                    Some(PathBuf::from("/tmp/site-templates-explicit"))
                );
                let resolved =
                    resolve_site_render_templates_dir(templates_dir, &site_templates_dir);
                assert_eq!(resolved, PathBuf::from("/tmp/site-templates-explicit"));
            }
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[test]
    fn oidc_config_args_require_complete_values_for_serve() {
        let error = OidcConfigArgs::default()
            .require_complete()
            .expect_err("expected missing OIDC config to fail");

        assert_eq!(error, "missing required OIDC/TLS config: --tls-cert-path");
    }
}
