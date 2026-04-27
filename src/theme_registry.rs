use crate::constants::DEFAULT_TEMPLATE_NAME;
use crate::entities::audit_event::log_audit_event;
use crate::entities::site;
use crate::entities::theme_registry;
use crate::errors::SiteError;
use chrono::Utc;
use gix::bstr::ByteSlice;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};
use tokio::fs;
use tokio_util::io::SyncIoBridge;
use uuid::Uuid;

type BlockingGitTransport = gix_transport::client::git::blocking_io::Connection<
    Box<dyn Read + Send>,
    Box<dyn Write + Send>,
>;

#[derive(Debug, Clone)]
pub struct ThemeInstallRequest {
    pub slug: Option<String>,
    pub repo_url: String,
    pub branch: Option<String>,
    pub ssh_key_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ThemeUpdateRequest {
    pub repo_url: String,
    pub branch: Option<String>,
    pub ssh_key_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ThemeAdminRow {
    pub slug: String,
    pub repo_url: String,
    pub branch: String,
    pub ssh_key_name: String,
    pub installed: bool,
    pub site_count: usize,
    pub edit_href: String,
    pub update_href: String,
    pub delete_href: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeSshKeyOption {
    pub name: String,
}

fn theme_path(root: &Path, slug: &str) -> PathBuf {
    root.join(slug)
}

pub fn derive_theme_slug(repo_url: &str) -> String {
    let trimmed = repo_url.trim().trim_end_matches('/');
    let candidate = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(trimmed)
        .trim_end_matches(".git");
    normalize_theme_slug(candidate).unwrap_or_else(|_| "theme".to_string())
}

pub fn normalize_theme_slug(input: &str) -> Result<String, SiteError> {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in input.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if matches!(ch, '-' | '_') {
            if !last_was_dash {
                slug.push(ch);
                last_was_dash = true;
            }
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let slug = slug.trim_matches(['-', '_']).to_string();
    if slug.is_empty() {
        return Err(SiteError::BadRequest(
            "theme slug cannot be empty".to_string(),
        ));
    }
    if slug == DEFAULT_TEMPLATE_NAME {
        return Err(SiteError::BadRequest(
            "default is reserved for the built-in template".to_string(),
        ));
    }

    Ok(slug)
}

fn theme_branch_display(branch: Option<&str>) -> String {
    branch.unwrap_or("default branch").to_string()
}

fn theme_key_display(key_name: Option<&str>) -> String {
    key_name.unwrap_or("None").to_string()
}

fn is_ssh_repo_url(repo_url: &str) -> bool {
    let trimmed = repo_url.trim();
    trimmed.starts_with("ssh://")
        || trimmed.starts_with("git+ssh://")
        || trimmed
            .split_once(':')
            .is_some_and(|(host_part, _)| host_part.contains('@') && !host_part.contains('/'))
}

fn normalize_theme_ssh_key_name(input: Option<String>) -> Result<Option<String>, SiteError> {
    let Some(value) = input else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed == "." || trimmed == ".." || trimmed.contains('/') || trimmed.contains('\\') {
        return Err(SiteError::BadRequest(
            "theme ssh key must be selected from the configured key directory".to_string(),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn resolve_theme_ssh_key_path(
    key_dir: &Path,
    key_name: Option<&str>,
    repo_url: &str,
) -> Result<Option<PathBuf>, SiteError> {
    let key_name = key_name.filter(|value| !value.trim().is_empty());
    if is_ssh_repo_url(repo_url) && key_name.is_none() {
        return Err(SiteError::BadRequest(
            "SSH theme repositories require a selected SSH key".to_string(),
        ));
    }
    Ok(key_name.map(|name| key_dir.join(name)))
}

pub async fn list_theme_ssh_keys(key_dir: &Path) -> Result<Vec<ThemeSshKeyOption>, SiteError> {
    let mut entries = match fs::read_dir(key_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to read theme ssh key directory {}: {error}",
                key_dir.display()
            )));
        }
    };
    let mut keys = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        SiteError::internal(format!("failed to enumerate theme ssh keys: {error}"))
    })? {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "known_hosts" || name.ends_with(".pub") {
            continue;
        }
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        if fs::File::open(entry.path()).await.is_err() {
            continue;
        }
        keys.push(ThemeSshKeyOption { name });
    }
    keys.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(keys)
}

async fn read_installed_theme_remote(
    repo_path: &Path,
) -> Result<(String, Option<String>), SiteError> {
    let repo = gix::open(repo_path)
        .map_err(|error| SiteError::internal(format!("failed to open theme repo: {error}")))?;
    let remote = repo
        .find_fetch_remote(None)
        .map_err(|error| SiteError::internal(format!("failed to load theme remote: {error}")))?;
    let repo_url = remote
        .url(gix::remote::Direction::Fetch)
        .ok_or_else(|| SiteError::internal("theme repo is missing a fetch URL"))?
        .to_string();
    let branch = repo
        .head_name()
        .map_err(|error| SiteError::internal(format!("failed to read theme branch: {error}")))?;
    let branch = branch.map(|name| name.shorten().to_string());
    Ok((repo_url, branch))
}

async fn adopt_installed_theme(
    txn: &impl ConnectionTrait,
    slug: &str,
    repo_url: String,
    branch: Option<String>,
) -> Result<(), SiteError> {
    let now = Utc::now();
    let model = theme_registry::ActiveModel {
        id: Set(Uuid::now_v7()),
        slug: Set(slug.to_string()),
        repo_url: Set(repo_url),
        branch: Set(branch),
        ssh_key_name: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    match model.insert(txn).await {
        Ok(_) => {}
        Err(error) => {
            let message = error.to_string();
            if !(message.contains("UNIQUE constraint failed")
                && message.contains("theme_registry.slug"))
            {
                return Err(SiteError::internal(format!(
                    "failed to adopt theme: {error}"
                )));
            }
        }
    }
    Ok(())
}

async fn sync_discovered_themes_in_txn(
    txn: &impl ConnectionTrait,
    templates_root: &Path,
) -> Result<usize, SiteError> {
    let mut adopted = 0usize;
    let existing = theme_registry::Entity::find()
        .all(txn)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load theme registry: {error}")))?;
    let existing_slugs = existing
        .into_iter()
        .map(|theme| theme.slug)
        .collect::<HashSet<_>>();

    let mut entries = match fs::read_dir(templates_root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to read theme directory: {error}"
            )));
        }
    };

    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        SiteError::internal(format!("failed to enumerate theme directory: {error}"))
    })? {
        let file_type = entry.file_type().await.map_err(|error| {
            SiteError::internal(format!(
                "failed to inspect theme entry {}: {error}",
                entry.path().display()
            ))
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let slug = entry.file_name().to_string_lossy().to_string();
        if slug == DEFAULT_TEMPLATE_NAME || existing_slugs.contains(&slug) {
            continue;
        }

        let repo_path = entry.path();
        if fs::metadata(repo_path.join(".git")).await.is_err() {
            continue;
        }

        let Ok((repo_url, branch)) = read_installed_theme_remote(&repo_path).await else {
            continue;
        };
        adopt_installed_theme(txn, &slug, repo_url, branch).await?;
        adopted = adopted.saturating_add(1);
    }

    Ok(adopted)
}

pub async fn sync_discovered_themes(
    db: &DatabaseConnection,
    templates_root: &Path,
) -> Result<usize, SiteError> {
    let txn = db.begin().await?;
    let adopted = sync_discovered_themes_in_txn(&txn, templates_root).await?;
    txn.commit().await?;
    Ok(adopted)
}

pub async fn available_template_names(
    db: &DatabaseConnection,
    templates_root: &Path,
    include_name: Option<&str>,
) -> Result<Vec<String>, SiteError> {
    let _ = sync_discovered_themes(db, templates_root).await?;
    let mut names = vec![DEFAULT_TEMPLATE_NAME.to_string()];
    let mut themes = theme_registry::Entity::find()
        .order_by_asc(theme_registry::Column::Slug)
        .all(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load theme registry: {error}")))?;
    themes.sort_by(|left, right| left.slug.cmp(&right.slug));
    for theme in themes {
        if !names.contains(&theme.slug) {
            names.push(theme.slug);
        }
    }
    if let Some(name) = include_name
        && !names.iter().any(|candidate| candidate == name)
    {
        names.push(name.to_string());
    }
    Ok(names)
}

pub async fn theme_admin_rows(
    db: &DatabaseConnection,
    templates_root: &Path,
) -> Result<Vec<ThemeAdminRow>, SiteError> {
    let _ = sync_discovered_themes(db, templates_root).await?;
    let mut themes = theme_registry::Entity::find()
        .order_by_asc(theme_registry::Column::Slug)
        .all(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load theme registry: {error}")))?;
    themes.sort_by(|left, right| left.slug.cmp(&right.slug));

    let mut rows = Vec::with_capacity(themes.len());
    for theme in themes {
        let installed = fs::metadata(theme_path(templates_root, &theme.slug))
            .await
            .is_ok();
        let site_count = site::Entity::find()
            .filter(site::Column::TemplateName.eq(theme.slug.clone()))
            .count(db)
            .await
            .map_err(|error| SiteError::internal(format!("failed to count theme usage: {error}")))?
            as usize;
        rows.push(ThemeAdminRow {
            slug: theme.slug.clone(),
            repo_url: theme.repo_url.clone(),
            branch: theme_branch_display(theme.branch.as_deref()),
            ssh_key_name: theme_key_display(theme.ssh_key_name.as_deref()),
            installed,
            site_count,
            edit_href: format!("/admin/themes/{}/edit", theme.slug),
            update_href: format!("/admin/themes/{}/update", theme.slug),
            delete_href: format!("/admin/themes/{}/delete", theme.slug),
        });
    }

    Ok(rows)
}

#[derive(Debug, Clone)]
struct SshRepoUrl {
    user: String,
    host: String,
    port: u16,
    path: String,
}

#[derive(Debug, thiserror::Error)]
enum ThemeSshError {
    #[error("failed to connect to SSH theme repository: {0}")]
    Russh(#[from] russh::Error),
    #[error("failed to decode SSH theme key: {0}")]
    KeyDecode(#[from] russh::keys::Error),
    #[error("failed to read SSH theme key: {0}")]
    KeyIo(#[from] std::io::Error),
    #[error("failed to encode SSH host key: {0}")]
    HostKeyEncode(#[from] russh::keys::ssh_key::Error),
    #[error("SSH host key for {0} changed")]
    HostKeyChanged(String),
}

#[derive(Clone)]
struct ThemeSshClient {
    host_key_name: String,
    known_hosts_path: PathBuf,
}

impl russh::client::Handler for ThemeSshClient {
    type Error = ThemeSshError;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let encoded_key = server_public_key.to_openssh()?;
        let known_hosts = read_theme_known_hosts(&self.known_hosts_path)?;
        match known_hosts.get(&self.host_key_name) {
            Some(stored_key) if stored_key == &encoded_key => Ok(true),
            Some(_) => Err(ThemeSshError::HostKeyChanged(self.host_key_name.clone())),
            None => {
                write_theme_known_host(&self.known_hosts_path, &self.host_key_name, &encoded_key)?;
                Ok(true)
            }
        }
    }
}

fn read_theme_known_hosts(path: &Path) -> Result<BTreeMap<String, String>, std::io::Error> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut hosts = BTreeMap::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((host, key)) = trimmed.split_once(' ') {
            hosts.insert(host.to_string(), key.to_string());
        }
    }
    Ok(hosts)
}

fn write_theme_known_host(
    path: &Path,
    host: &str,
    encoded_key: &str,
) -> Result<(), std::io::Error> {
    let mut hosts = read_theme_known_hosts(path)?;
    hosts.insert(host.to_string(), encoded_key.to_string());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut contents = String::new();
    for (host, key) in hosts {
        contents.push_str(&host);
        contents.push(' ');
        contents.push_str(&key);
        contents.push('\n');
    }
    std::fs::write(path, contents)
}

fn parse_ssh_repo_url(repo_url: &str) -> Result<SshRepoUrl, SiteError> {
    let trimmed = repo_url.trim();
    if let Some(rest) = trimmed
        .strip_prefix("ssh://")
        .or_else(|| trimmed.strip_prefix("git+ssh://"))
    {
        let (authority, path) = rest.split_once('/').ok_or_else(|| {
            SiteError::BadRequest(
                "SSH theme repository URL is missing a repository path".to_string(),
            )
        })?;
        let (user, host_port) = authority.split_once('@').ok_or_else(|| {
            SiteError::BadRequest("SSH theme repository URL is missing a username".to_string())
        })?;
        let (host, port) = match host_port.rsplit_once(':') {
            Some((host, port)) => (
                host,
                port.parse::<u16>().map_err(|_| {
                    SiteError::BadRequest(
                        "SSH theme repository URL has an invalid port".to_string(),
                    )
                })?,
            ),
            None => (host_port, 22),
        };
        if host.is_empty() || path.is_empty() {
            return Err(SiteError::BadRequest(
                "SSH theme repository URL is missing a host or repository path".to_string(),
            ));
        }
        return Ok(SshRepoUrl {
            user: user.to_string(),
            host: host.to_string(),
            port,
            path: format!("/{path}"),
        });
    }

    let (user_host, path) = trimmed.split_once(':').ok_or_else(|| {
        SiteError::BadRequest("SSH theme repository URL is missing a repository path".to_string())
    })?;
    let (user, host) = user_host.split_once('@').ok_or_else(|| {
        SiteError::BadRequest("SSH theme repository URL is missing a username".to_string())
    })?;
    if host.is_empty() || path.is_empty() {
        return Err(SiteError::BadRequest(
            "SSH theme repository URL is missing a host or repository path".to_string(),
        ));
    }
    Ok(SshRepoUrl {
        user: user.to_string(),
        host: host.to_string(),
        port: 22,
        path: path.to_string(),
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn open_ssh_git_transport(
    repo_url: &str,
    key_path: &Path,
    known_hosts_path: &Path,
) -> Result<BlockingGitTransport, SiteError> {
    let ssh_url = parse_ssh_repo_url(repo_url)?;
    let runtime = Arc::new(tokio::runtime::Runtime::new().map_err(|error| {
        SiteError::internal(format!("failed to create SSH transport runtime: {error}"))
    })?);
    let runtime_for_reader = Arc::clone(&runtime);
    let runtime_for_writer = Arc::clone(&runtime);
    let stream = runtime
        .block_on(async {
            let key_contents = tokio::fs::read_to_string(key_path).await?;
            let private_key = russh::keys::decode_secret_key(&key_contents, None)?;
            let config = Arc::new(russh::client::Config::default());
            let host_key_name = if ssh_url.port == 22 {
                ssh_url.host.clone()
            } else {
                format!("[{}]:{}", ssh_url.host, ssh_url.port)
            };
            let mut session = russh::client::connect(
                config,
                (ssh_url.host.as_str(), ssh_url.port),
                ThemeSshClient {
                    host_key_name,
                    known_hosts_path: known_hosts_path.to_path_buf(),
                },
            )
            .await?;
            let auth = session
                .authenticate_publickey(
                    ssh_url.user.clone(),
                    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(private_key), None),
                )
                .await?;
            if !auth.success() {
                return Err(ThemeSshError::Russh(russh::Error::NotAuthenticated));
            }
            let channel = session.channel_open_session().await?;
            channel
                .exec(
                    true,
                    format!("git-upload-pack {}", shell_quote(&ssh_url.path)),
                )
                .await?;
            Ok::<_, ThemeSshError>(channel.into_stream())
        })
        .map_err(|error| {
            SiteError::internal(format!("failed to open SSH theme transport: {error}"))
        })?;
    let (reader, writer) = tokio::io::split(stream);
    let reader: Box<dyn Read + Send> = Box::new(SshRuntimeReader {
        inner: SyncIoBridge::new_with_handle(reader, runtime_for_reader.handle().clone()),
        _runtime: runtime_for_reader,
    });
    let writer: Box<dyn Write + Send> = Box::new(SshRuntimeWriter {
        inner: SyncIoBridge::new_with_handle(writer, runtime_for_writer.handle().clone()),
        _runtime: runtime_for_writer,
    });
    Ok(gix_transport::client::git::blocking_io::Connection::new(
        reader,
        writer,
        gix_transport::Protocol::V2,
        ssh_url.path.into_bytes(),
        None::<(String, Option<u16>)>,
        gix_transport::client::git::ConnectMode::Process,
        false,
    )
    .custom_url(Some(repo_url.as_bytes().to_vec().into())))
}

struct SshRuntimeReader<R> {
    inner: SyncIoBridge<R>,
    _runtime: Arc<tokio::runtime::Runtime>,
}

impl<R: tokio::io::AsyncRead + Unpin> Read for SshRuntimeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

struct SshRuntimeWriter<W> {
    inner: SyncIoBridge<W>,
    _runtime: Arc<tokio::runtime::Runtime>,
}

impl<W: tokio::io::AsyncWrite + Unpin> Write for SshRuntimeWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn checkout_theme_worktree(repo: &gix::Repository, ref_name: &str) -> Result<(), SiteError> {
    let mut reference = repo.find_reference(ref_name).map_err(|error| {
        SiteError::internal(format!("failed to resolve fetched theme ref: {error}"))
    })?;
    let commit = reference.peel_to_commit().map_err(|error| {
        SiteError::internal(format!("fetched theme ref is not a commit: {error}"))
    })?;
    let tree_id = commit.tree_id().map_err(|error| {
        SiteError::internal(format!("failed to load fetched theme tree: {error}"))
    })?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| SiteError::internal("theme repository is missing a worktree"))?
        .to_path_buf();
    let (mut stream, _) = repo.worktree_stream(tree_id).map_err(|error| {
        SiteError::internal(format!("failed to stream theme worktree: {error}"))
    })?;
    while let Some(mut entry) = stream.next_entry().map_err(|error| {
        SiteError::internal(format!("failed to read theme worktree entry: {error}"))
    })? {
        let relative = entry.relative_path().to_str().map_err(|error| {
            SiteError::internal(format!("theme worktree path is not utf-8: {error}"))
        })?;
        let path = workdir.join(relative);
        if entry.mode.is_tree() {
            std::fs::create_dir_all(&path).map_err(|error| {
                SiteError::internal(format!(
                    "failed to create theme directory {}: {error}",
                    path.display()
                ))
            })?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                SiteError::internal(format!(
                    "failed to create theme parent {}: {error}",
                    parent.display()
                ))
            })?;
        }
        if entry.mode.is_link() {
            let mut target = Vec::new();
            entry.read_to_end(&mut target).map_err(|error| {
                SiteError::internal(format!(
                    "failed to read theme symlink {}: {error}",
                    path.display()
                ))
            })?;
            std::os::unix::fs::symlink(String::from_utf8_lossy(&target).as_ref(), &path).map_err(
                |error| {
                    SiteError::internal(format!(
                        "failed to create theme symlink {}: {error}",
                        path.display()
                    ))
                },
            )?;
            continue;
        }
        let mut file = std::fs::File::create(&path).map_err(|error| {
            SiteError::internal(format!(
                "failed to create theme file {}: {error}",
                path.display()
            ))
        })?;
        std::io::copy(&mut entry, &mut file).map_err(|error| {
            SiteError::internal(format!(
                "failed to write theme file {}: {error}",
                path.display()
            ))
        })?;
        #[cfg(unix)]
        if entry.mode.is_executable() {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = file
                .metadata()
                .map_err(|error| {
                    SiteError::internal(format!(
                        "failed to stat theme file {}: {error}",
                        path.display()
                    ))
                })?
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).map_err(|error| {
                SiteError::internal(format!(
                    "failed to set theme file mode {}: {error}",
                    path.display()
                ))
            })?;
        }
    }
    Ok(())
}

fn clone_theme_repo_over_ssh(
    repo_url: &str,
    destination: &Path,
    branch: Option<&String>,
    ssh_key_path: &Path,
    known_hosts_path: &Path,
) -> Result<(), SiteError> {
    let repo = gix::init(destination).map_err(|error| {
        SiteError::internal(format!("failed to initialize theme repo: {error}"))
    })?;
    let mut remote = repo.remote_at(repo_url.to_string()).map_err(|error| {
        SiteError::internal(format!("failed to configure theme remote: {error}"))
    })?;
    remote = remote
        .with_refspecs(
            Some("+refs/heads/*:refs/remotes/origin/*"),
            gix::remote::Direction::Fetch,
        )
        .map_err(|error| {
            SiteError::internal(format!("failed to configure theme fetch refspecs: {error}"))
        })?;
    let transport = open_ssh_git_transport(repo_url, ssh_key_path, known_hosts_path)?;
    let connection = remote.to_connection_with_transport(transport);
    let mut fetch_options = gix::remote::ref_map::Options::default();
    let head_refspec = gix::refspec::parse(
        "HEAD:refs/remotes/origin/HEAD".into(),
        gix::refspec::parse::Operation::Fetch,
    )
    .map_err(|error| SiteError::internal(format!("invalid theme HEAD refspec: {error}")))?
    .to_owned();
    fetch_options.extra_refspecs.push(head_refspec);
    if let Some(branch) = branch.map(String::as_str) {
        fetch_options.extra_refspecs.push(
            gix::refspec::parse(
                branch.as_bytes().as_bstr(),
                gix::refspec::parse::Operation::Fetch,
            )
            .map_err(|error| {
                SiteError::internal(format!("invalid theme branch {branch}: {error}"))
            })?
            .to_owned(),
        );
    }
    let should_interrupt = AtomicBool::new(false);
    let pending_pack = connection
        .prepare_fetch(gix::progress::Discard, fetch_options)
        .map_err(|error| {
            SiteError::internal(format!("failed to prepare SSH theme fetch: {error}"))
        })?;
    pending_pack
        .with_write_packed_refs_only(true)
        .receive(gix::progress::Discard, &should_interrupt)
        .map_err(|error| SiteError::internal(format!("failed to fetch SSH theme repo: {error}")))?;
    let checkout_ref = branch
        .map(String::as_str)
        .map(|branch| format!("refs/remotes/origin/{branch}"))
        .unwrap_or_else(|| "refs/remotes/origin/HEAD".to_string());
    checkout_theme_worktree(&repo, &checkout_ref)?;
    std::fs::write(
        destination.join(".git").join("HEAD"),
        format!("ref: {checkout_ref}\n"),
    )
    .map_err(|error| SiteError::internal(format!("failed to write theme HEAD: {error}")))?;
    Ok(())
}

async fn clone_theme_repo(
    repo_url: String,
    destination: PathBuf,
    branch: Option<String>,
    ssh_key_path: Option<PathBuf>,
    known_hosts_path: PathBuf,
) -> Result<(), SiteError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).await.map_err(|error| {
            SiteError::internal(format!(
                "failed to create theme directory parent {}: {error}",
                parent.display()
            ))
        })?;
    }

    tokio::task::spawn_blocking(move || -> Result<(), SiteError> {
        if destination.exists() {
            return Err(SiteError::BadRequest(format!(
                "theme directory already exists: {}",
                destination.display()
            )));
        }
        if is_ssh_repo_url(&repo_url) {
            let key_path = ssh_key_path.ok_or_else(|| {
                SiteError::BadRequest(
                    "SSH theme repositories require a selected SSH key".to_string(),
                )
            })?;
            return clone_theme_repo_over_ssh(
                &repo_url,
                &destination,
                branch.as_ref(),
                &key_path,
                &known_hosts_path,
            );
        }

        let should_interrupt = AtomicBool::new(false);
        let mut clone = gix::prepare_clone(repo_url, &destination).map_err(|error| {
            SiteError::internal(format!("failed to prepare theme clone: {error}"))
        })?;
        if let Some(branch) = branch.as_deref() {
            clone = clone.with_ref_name(Some(branch)).map_err(|error| {
                SiteError::internal(format!("invalid theme branch {branch}: {error}"))
            })?;
        }
        let (mut checkout, _) = clone
            .fetch_then_checkout(gix::progress::Discard, &should_interrupt)
            .map_err(|error| SiteError::internal(format!("failed to fetch theme repo: {error}")))?;
        let _ = checkout
            .main_worktree(gix::progress::Discard, &should_interrupt)
            .map_err(|error| {
                SiteError::internal(format!("failed to checkout theme repo: {error}"))
            })?;
        Ok(())
    })
    .await
    .map_err(|error| SiteError::internal(format!("theme clone task failed: {error}")))?
}

pub async fn install_theme(
    db: &DatabaseConnection,
    actor_sub: &str,
    templates_root: &Path,
    ssh_key_dir: &Path,
    known_hosts_path: &Path,
    request: ThemeInstallRequest,
) -> Result<theme_registry::Model, SiteError> {
    let slug = match request.slug {
        Some(slug) if !slug.trim().is_empty() => normalize_theme_slug(&slug)?,
        _ => normalize_theme_slug(&derive_theme_slug(&request.repo_url))?,
    };
    let branch = request
        .branch
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let ssh_key_name = normalize_theme_ssh_key_name(request.ssh_key_name)?;
    let ssh_key_path =
        resolve_theme_ssh_key_path(ssh_key_dir, ssh_key_name.as_deref(), &request.repo_url)?;
    let theme_dir = theme_path(templates_root, &slug);

    sync_discovered_themes(db, templates_root).await?;
    if theme_registry::Entity::find()
        .filter(theme_registry::Column::Slug.eq(&slug))
        .one(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to query theme registry: {error}")))?
        .is_some()
    {
        return Err(SiteError::BadRequest(format!(
            "theme {slug} already exists"
        )));
    }

    clone_theme_repo(
        request.repo_url.clone(),
        theme_dir.clone(),
        branch.clone(),
        ssh_key_path,
        known_hosts_path.to_path_buf(),
    )
    .await?;

    let detected_branch = if branch.is_some() {
        branch.clone()
    } else {
        match gix::open(&theme_dir) {
            Ok(repo) => repo
                .head_name()
                .map_err(|error| {
                    SiteError::internal(format!("failed to detect cloned theme branch: {error}"))
                })?
                .map(|name| name.shorten().to_string()),
            Err(_) => None,
        }
    };

    let txn = db.begin().await?;
    let now = Utc::now();
    let model = theme_registry::ActiveModel {
        id: Set(Uuid::now_v7()),
        slug: Set(slug.clone()),
        repo_url: Set(request.repo_url.clone()),
        branch: Set(detected_branch.clone()),
        ssh_key_name: Set(ssh_key_name.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&txn)
    .await
    .map_err(|error| {
        SiteError::internal(format!("failed to save installed theme {slug}: {error}"))
    })?;
    log_audit_event(
        &txn,
        actor_sub,
        "install_theme",
        "theme_registry",
        &model.slug,
        None,
        Some(serde_json::json!({
            "slug": model.slug,
            "repo_url": model.repo_url,
            "branch": model.branch,
            "ssh_key_name": model.ssh_key_name,
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log theme install audit: {error}")))?;
    txn.commit().await?;

    Ok(model)
}

pub async fn update_theme(
    db: &DatabaseConnection,
    actor_sub: &str,
    slug: &str,
    templates_root: &Path,
    ssh_key_dir: &Path,
    known_hosts_path: &Path,
) -> Result<theme_registry::Model, SiteError> {
    sync_discovered_themes(db, templates_root).await?;
    let existing = theme_registry::Entity::find()
        .filter(theme_registry::Column::Slug.eq(slug))
        .one(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load theme {slug}: {error}")))?
        .ok_or_else(|| SiteError::SiteNotFound(slug.to_string()))?;

    let parent = theme_path(templates_root, slug)
        .parent()
        .ok_or_else(|| SiteError::internal("theme path has no parent"))?
        .to_path_buf();
    let temp_path = parent.join(format!(".{slug}-update-{}", Uuid::now_v7()));
    let ssh_key_path = resolve_theme_ssh_key_path(
        ssh_key_dir,
        existing.ssh_key_name.as_deref(),
        &existing.repo_url,
    )?;

    clone_theme_repo(
        existing.repo_url.clone(),
        temp_path.clone(),
        existing.branch.clone(),
        ssh_key_path,
        known_hosts_path.to_path_buf(),
    )
    .await?;

    let final_path = theme_path(templates_root, slug);
    let backup_path = parent.join(format!(".{slug}-backup-{}", Uuid::now_v7()));
    if fs::metadata(&final_path).await.is_ok() {
        fs::rename(&final_path, &backup_path)
            .await
            .map_err(|error| {
                SiteError::internal(format!(
                    "failed to stage existing theme for update {}: {error}",
                    final_path.display()
                ))
            })?;
    }

    match fs::rename(&temp_path, &final_path).await {
        Ok(()) => {
            let _ = fs::remove_dir_all(&backup_path).await;
        }
        Err(error) => {
            if fs::metadata(&backup_path).await.is_ok() {
                let _ = fs::rename(&backup_path, &final_path).await;
            }
            return Err(SiteError::internal(format!(
                "failed to replace theme {}: {error}",
                final_path.display()
            )));
        }
    }

    let txn = db.begin().await?;
    let now = Utc::now();
    let mut active = existing.into_active_model();
    active.updated_at = Set(now);
    let model = active.update(&txn).await.map_err(|error| {
        SiteError::internal(format!(
            "failed to update theme registry for {slug}: {error}"
        ))
    })?;
    log_audit_event(
        &txn,
        actor_sub,
        "update_theme",
        "theme_registry",
        &model.slug,
        None,
        Some(serde_json::json!({
            "slug": model.slug,
            "repo_url": model.repo_url,
            "branch": model.branch,
            "ssh_key_name": model.ssh_key_name,
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log theme update audit: {error}")))?;
    txn.commit().await?;

    Ok(model)
}

pub async fn get_theme(
    db: &DatabaseConnection,
    templates_root: &Path,
    slug: &str,
) -> Result<theme_registry::Model, SiteError> {
    sync_discovered_themes(db, templates_root).await?;
    theme_registry::Entity::find()
        .filter(theme_registry::Column::Slug.eq(slug))
        .one(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load theme {slug}: {error}")))?
        .ok_or_else(|| SiteError::SiteNotFound(slug.to_string()))
}

pub async fn update_theme_metadata(
    db: &DatabaseConnection,
    actor_sub: &str,
    slug: &str,
    templates_root: &Path,
    request: ThemeUpdateRequest,
) -> Result<theme_registry::Model, SiteError> {
    sync_discovered_themes(db, templates_root).await?;
    let existing = theme_registry::Entity::find()
        .filter(theme_registry::Column::Slug.eq(slug))
        .one(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load theme {slug}: {error}")))?
        .ok_or_else(|| SiteError::SiteNotFound(slug.to_string()))?;
    let repo_url = request.repo_url.trim().to_string();
    if repo_url.is_empty() {
        return Err(SiteError::BadRequest("missing repository url".to_string()));
    }
    let branch = request
        .branch
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let ssh_key_name = normalize_theme_ssh_key_name(request.ssh_key_name)?;

    let txn = db.begin().await?;
    let now = Utc::now();
    let mut active = existing.into_active_model();
    active.repo_url = Set(repo_url);
    active.branch = Set(branch);
    active.ssh_key_name = Set(ssh_key_name);
    active.updated_at = Set(now);
    let model = active.update(&txn).await.map_err(|error| {
        SiteError::internal(format!(
            "failed to update theme metadata for {slug}: {error}"
        ))
    })?;
    log_audit_event(
        &txn,
        actor_sub,
        "edit_theme",
        "theme_registry",
        &model.slug,
        None,
        Some(serde_json::json!({
            "slug": model.slug,
            "repo_url": model.repo_url,
            "branch": model.branch,
            "ssh_key_name": model.ssh_key_name,
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log theme edit audit: {error}")))?;
    txn.commit().await?;
    Ok(model)
}

pub async fn delete_theme(
    db: &DatabaseConnection,
    actor_sub: &str,
    slug: &str,
    templates_root: &Path,
) -> Result<(), SiteError> {
    sync_discovered_themes(db, templates_root).await?;
    let existing = theme_registry::Entity::find()
        .filter(theme_registry::Column::Slug.eq(slug))
        .one(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load theme {slug}: {error}")))?
        .ok_or_else(|| SiteError::SiteNotFound(slug.to_string()))?;
    let theme_usage_count = site::Entity::find()
        .filter(site::Column::TemplateName.eq(slug))
        .count(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to check theme usage: {error}")))?;
    if theme_usage_count > 0 {
        return Err(SiteError::BadRequest(format!(
            "theme {slug} is still used by {theme_usage_count} site(s)"
        )));
    }

    let repo_url = existing.repo_url.clone();
    let txn = db.begin().await?;
    existing
        .into_active_model()
        .delete(&txn)
        .await
        .map_err(|error| SiteError::internal(format!("failed to delete theme {slug}: {error}")))?;
    log_audit_event(
        &txn,
        actor_sub,
        "delete_theme",
        "theme_registry",
        slug,
        None,
        Some(serde_json::json!({
            "slug": slug,
            "repo_url": repo_url,
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log theme delete audit: {error}")))?;
    txn.commit().await?;

    match fs::remove_dir_all(theme_path(templates_root, slug)).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to remove theme directory {}: {error}",
                theme_path(templates_root, slug).display()
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::constants::DEFAULT_TEMPLATE_NAME;
    use crate::db::test_db_start;
    use std::process::Command;
    use tempfile::TempDir;

    fn copy_dir_recursive(source: &Path, target: &Path) {
        std::fs::create_dir_all(target).expect("failed to create template fixture target");
        for entry in std::fs::read_dir(source).expect("failed to read template fixture source") {
            let entry = entry.expect("failed to read template fixture entry");
            let entry_path = entry.path();
            let target_path = target.join(entry.file_name());
            if entry_path.is_dir() {
                copy_dir_recursive(&entry_path, &target_path);
            } else {
                std::fs::copy(&entry_path, &target_path)
                    .expect("failed to copy template fixture file");
            }
        }
    }

    fn seed_site_templates_root() -> TempDir {
        let root = TempDir::new().expect("failed to create template root");
        let default_source = crate::resolve_site_templates_root().join("default");
        copy_dir_recursive(&default_source, &root.path().join(DEFAULT_TEMPLATE_NAME));
        root
    }

    fn git_command(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Codex")
            .env("GIT_AUTHOR_EMAIL", "codex@example.com")
            .env("GIT_COMMITTER_NAME", "Codex")
            .env("GIT_COMMITTER_EMAIL", "codex@example.com")
            .args(args)
            .status()
            .expect("failed to run git command");
        assert!(status.success(), "git command failed: {:?}", args);
    }

    fn create_theme_repo() -> TempDir {
        let repo = TempDir::new().expect("failed to create theme repo");
        git_command(repo.path(), &["init", "-b", "main"]);
        std::fs::write(repo.path().join("theme.txt"), "version-one")
            .expect("failed to write theme file");
        git_command(repo.path(), &["add", "theme.txt"]);
        git_command(repo.path(), &["commit", "-m", "initial theme"]);
        repo
    }

    fn commit_theme_update(repo: &Path, new_content: &str, message: &str) {
        std::fs::write(repo.join("theme.txt"), new_content).expect("failed to update theme file");
        git_command(repo, &["add", "theme.txt"]);
        git_command(repo, &["commit", "-m", message]);
    }

    #[test]
    fn slug_helpers_normalize_repo_names() {
        assert_eq!(
            derive_theme_slug("https://example.com/acme/my-theme.git"),
            "my-theme"
        );
        assert_eq!(
            normalize_theme_slug("My Theme").expect("failed to normalize slug"),
            "my-theme"
        );
    }

    #[test]
    fn ssh_repo_urls_are_kept_as_ssh() {
        assert!(is_ssh_repo_url("git@github.com:owner/theme.git"));
        assert!(is_ssh_repo_url("ssh://git@gitlab.com/owner/theme.git"));
        assert!(is_ssh_repo_url(
            "git+ssh://git@bitbucket.org/owner/theme.git"
        ));
        assert!(!is_ssh_repo_url("https://github.com/owner/theme.git"));
    }

    #[test]
    fn theme_ssh_key_names_reject_path_traversal() {
        let error = normalize_theme_ssh_key_name(Some("../id_ed25519".to_string()))
            .expect_err("path traversal key name should be rejected");
        assert!(
            error
                .to_string()
                .contains("selected from the configured key directory")
        );
    }

    #[test]
    fn ssh_clone_config_requires_selected_key() {
        let key_dir = Path::new("/tmp/theme-keys");
        assert!(
            resolve_theme_ssh_key_path(key_dir, None, "git@example.com:owner/theme.git").is_err()
        );
        assert!(
            resolve_theme_ssh_key_path(key_dir, None, "https://example.com/owner/theme.git")
                .expect("https clone should not need key")
                .is_none()
        );
        assert_eq!(
            resolve_theme_ssh_key_path(
                key_dir,
                Some("id_ed25519"),
                "git@example.com:owner/theme.git"
            )
            .expect("ssh key should resolve"),
            Some(key_dir.join("id_ed25519"))
        );
    }

    #[tokio::test]
    async fn list_theme_ssh_keys_filters_and_sorts_entries() {
        let key_dir = TempDir::new().expect("failed to create ssh key dir");
        std::fs::write(key_dir.path().join("z_key"), "key").expect("failed to write key");
        std::fs::write(key_dir.path().join("a_key"), "key").expect("failed to write key");
        std::fs::write(key_dir.path().join("a_key.pub"), "pub").expect("failed to write pub key");
        std::fs::write(key_dir.path().join(".hidden"), "key").expect("failed to write hidden key");
        std::fs::write(key_dir.path().join("known_hosts"), "host")
            .expect("failed to write known hosts");
        std::fs::create_dir(key_dir.path().join("nested")).expect("failed to create nested dir");

        let keys = list_theme_ssh_keys(key_dir.path())
            .await
            .expect("failed to enumerate ssh keys");
        assert_eq!(
            keys.into_iter().map(|key| key.name).collect::<Vec<_>>(),
            vec!["a_key".to_string(), "z_key".to_string()]
        );
    }

    #[tokio::test]
    async fn sync_discovered_themes_adopts_existing_git_repo_and_lists_it() {
        let db = test_db_start().await;
        let templates_root = seed_site_templates_root();
        let repo = create_theme_repo();
        let installed_path = templates_root.path().join("sample-theme");
        git_command(
            templates_root.path(),
            &[
                "clone",
                repo.path().to_str().expect("repo path should be utf-8"),
                "sample-theme",
            ],
        );

        let adopted = sync_discovered_themes(&db, templates_root.path())
            .await
            .expect("failed to sync discovered themes");
        assert_eq!(adopted, 1);

        let names = available_template_names(&db, templates_root.path(), None)
            .await
            .expect("failed to list template names");
        assert_eq!(
            names.first().map(String::as_str),
            Some(DEFAULT_TEMPLATE_NAME)
        );
        assert!(names.iter().any(|name| name == "sample-theme"));

        let rows = theme_admin_rows(&db, templates_root.path())
            .await
            .expect("failed to load theme admin rows");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.slug, "sample-theme");
        assert!(row.installed);
        assert_eq!(row.site_count, 0);
        assert!(installed_path.exists());
    }

    #[tokio::test]
    async fn install_then_update_theme_refreshes_cloned_content() {
        let db = test_db_start().await;
        let templates_root = seed_site_templates_root();
        let ssh_key_dir = TempDir::new().expect("failed to create ssh key dir");
        let known_hosts = TempDir::new().expect("failed to create known hosts dir");
        let repo = create_theme_repo();
        let install = install_theme(
            &db,
            "actor",
            templates_root.path(),
            ssh_key_dir.path(),
            &known_hosts.path().join("known_hosts"),
            ThemeInstallRequest {
                slug: Some("sample-theme".to_string()),
                repo_url: repo.path().to_string_lossy().to_string(),
                branch: None,
                ssh_key_name: None,
            },
        )
        .await
        .expect("failed to install theme");
        assert_eq!(install.slug, "sample-theme");

        let installed_file = templates_root.path().join("sample-theme").join("theme.txt");
        let initial =
            std::fs::read_to_string(&installed_file).expect("failed to read installed file");
        assert_eq!(initial, "version-one");

        commit_theme_update(repo.path(), "version-two", "update theme");
        let updated = update_theme(
            &db,
            "actor",
            "sample-theme",
            templates_root.path(),
            ssh_key_dir.path(),
            &known_hosts.path().join("known_hosts"),
        )
        .await
        .expect("failed to update theme");
        assert_eq!(updated.slug, "sample-theme");

        let refreshed = std::fs::read_to_string(&installed_file)
            .expect("failed to read refreshed installed file");
        assert_eq!(refreshed, "version-two");
    }
}
