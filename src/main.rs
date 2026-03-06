use clap::{Args, Parser, Subcommand};

use websites::{
    NewAlias, NewAsset, NewAssetVariant, NewContent, NewContentTag, NewMembership, NewTag, NewUser,
    UpdateContent, add_content_tag, content_primary_route, create_alias, create_asset,
    create_asset_variant, create_content, create_membership, create_site, create_tag, create_user,
    ensure_schema, list_aliases, list_asset_variants, list_assets, list_content, list_content_tags,
    list_memberships, list_revision_aliases, list_revision_tags, list_revisions, list_sites,
    list_tags, list_users, render_site, update_content,
};

#[derive(Debug, Parser)]
#[command(
    name = "websites",
    about = "Rust static site management platform",
    version
)]
struct Cli {
    #[arg(
        short = 'd',
        long = "database-url",
        default_value = "sqlite://./database.sqlite",
        help = "SQLite database URL for the management database"
    )]
    database_url: String,
    #[command(flatten)]
    oidc: OidcConfig,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Args, Clone)]
struct OidcConfig {
    /// Path to the TLS certificate file.
    #[arg(
        long = "tls-cert-path",
        env = "WEBSITES_TLS_CERT_PATH",
        value_name = "FILE"
    )]
    tls_cert_path: Option<String>,

    /// Path to the TLS private key file.
    #[arg(
        long = "tls-key-path",
        env = "WEBSITES_TLS_KEY_PATH",
        value_name = "FILE"
    )]
    tls_key_path: Option<String>,

    /// Public frontend URL for OIDC redirect and callback configuration.
    #[arg(
        long = "frontend-url",
        env = "WEBSITES_FRONTEND_URL",
        value_name = "URL"
    )]
    frontend_url: Option<String>,

    /// OIDC client ID.
    #[arg(
        long = "client-id",
        env = "WEBSITES_OIDC_CLIENT_ID",
        value_name = "STRING"
    )]
    oidc_client_id: Option<String>,

    /// OIDC discovery document URL.
    #[arg(
        long = "discovery-url",
        env = "WEBSITES_OIDC_DISCOVERY_URL",
        value_name = "URL"
    )]
    oidc_discovery_url: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialize required schema tables.
    Init,
    /// Show effective OIDC configuration loaded from CLI flags and env vars.
    ShowConfig,
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
    Content {
        #[command(subcommand)]
        command: ContentCommands,
    },
}

#[derive(Debug, Subcommand)]
enum SiteCommands {
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
enum UserCommands {
    /// Create a user record.
    Create {
        #[arg(long)]
        subject: String,
    },
    /// List users.
    List,
}

#[derive(Debug, Subcommand)]
enum AssetCommands {
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
enum ContentCommands {
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
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::Init);

    if let Err(error) = execute(command, &cli.database_url, &cli.oidc).await {
        eprintln!("error: {}", error);
        std::process::exit(1);
    }
}

async fn execute(command: Commands, database_url: &str, oidc: &OidcConfig) -> Result<(), String> {
    match command {
        Commands::Init => {
            ensure_schema(database_url).await?;
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
                let site = create_site(database_url, short_name, full_title, template_name).await?;
                println!("created site: {} ({})", site.id, site.short_name);
                Ok(())
            }
            SiteCommands::List => {
                let sites = list_sites(database_url).await?;
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
                    database_url,
                    NewMembership {
                        site_id,
                        user_id,
                        role,
                    },
                )
                .await?;
                println!("created membership: {} {}", membership.id, membership.role);
                Ok(())
            }
            SiteCommands::MemberList { site_id } => {
                let memberships = list_memberships(database_url, &site_id).await?;
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
                let tag = create_tag(database_url, NewTag { site_id, name }).await?;
                println!("created tag: {} {}", tag.id, tag.name);
                Ok(())
            }
            SiteCommands::TagList { site_id } => {
                let tags = list_tags(database_url, &site_id).await?;
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
                    render_site(database_url, &site_id, &templates_dir, &rendered_dir).await?;
                println!("rendered site {} files {}", site_id, files_written);
                Ok(())
            }
        },
        Commands::User { command } => match command {
            UserCommands::Create { subject } => {
                let user = create_user(database_url, NewUser { subject }).await?;
                println!("created user: {} {}", user.id, user.subject);
                Ok(())
            }
            UserCommands::List => {
                let users = list_users(database_url).await?;
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
                    database_url,
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
                println!("created asset: {} {}", asset.id, asset.original_filename);
                Ok(())
            }
            AssetCommands::List { site_id } => {
                let assets = list_assets(database_url, &site_id).await?;
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
                    database_url,
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
                println!("created variant: {} {}", variant.id, variant.filename);
                Ok(())
            }
            AssetCommands::VariantList { asset_id } => {
                let variants = list_asset_variants(database_url, &asset_id).await?;
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
                    database_url,
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
                println!("created content: {} {}", content.id, content.title);
                Ok(())
            }
            ContentCommands::List { site_id, page_type } => {
                let page_filter = page_type.as_deref();
                let content = list_content(database_url, &site_id, page_filter).await?;
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
                    database_url,
                    NewAlias {
                        content_id,
                        site_id,
                        alias_path,
                        kind,
                    },
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
                let aliases = list_aliases(database_url, &site_id, content_filter).await?;
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
                let revisions = list_revisions(database_url, &content_id).await?;
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
                let aliases = list_revision_aliases(database_url, &revision_id).await?;
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
                let tags = list_revision_tags(database_url, &revision_id).await?;
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
                    database_url,
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
                println!("updated content: {} {}", content.id, content.title);
                Ok(())
            }
            ContentCommands::TagAdd {
                content_id,
                site_id,
                tag_name,
            } => {
                let content_tag = add_content_tag(
                    database_url,
                    NewContentTag {
                        content_id,
                        site_id,
                        tag_name,
                    },
                )
                .await?;
                println!(
                    "linked content tag: {} {}",
                    content_tag.id, content_tag.tag_id
                );
                Ok(())
            }
            ContentCommands::TagList { content_id } => {
                let tags = list_content_tags(database_url, &content_id).await?;
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
        },
    }
}

fn cli_value(value: &Option<String>) -> String {
    value
        .as_deref()
        .map_or_else(|| "<unset>".to_string(), |value| value.to_string())
}
