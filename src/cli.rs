use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use url::Url;

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
        default_value = "sqlite://./database.sqlite?mode=rwc",
        help = "SQLite database URL for the management database"
    )]
    pub database_url: String,
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
    /// Add a user membership to a site.
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
