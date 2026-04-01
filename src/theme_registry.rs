use crate::constants::DEFAULT_TEMPLATE_NAME;
use crate::entities::audit_event::log_audit_event;
use crate::entities::site;
use crate::entities::theme_registry;
use crate::errors::SiteError;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use tokio::fs;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ThemeInstallRequest {
    pub slug: Option<String>,
    pub repo_url: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ThemeAdminRow {
    pub slug: String,
    pub repo_url: String,
    pub branch: String,
    pub installed: bool,
    pub site_count: usize,
    pub update_href: String,
    pub delete_href: String,
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
            installed,
            site_count,
            update_href: format!("/admin/themes/{}/update", theme.slug),
            delete_href: format!("/admin/themes/{}/delete", theme.slug),
        });
    }

    Ok(rows)
}

async fn clone_theme_repo(
    repo_url: String,
    destination: PathBuf,
    branch: Option<String>,
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

    clone_theme_repo(request.repo_url.clone(), theme_dir.clone(), branch.clone()).await?;

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

    clone_theme_repo(
        existing.repo_url.clone(),
        temp_path.clone(),
        existing.branch.clone(),
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
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log theme update audit: {error}")))?;
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
        let repo = create_theme_repo();
        let install = install_theme(
            &db,
            "actor",
            templates_root.path(),
            ThemeInstallRequest {
                slug: Some("sample-theme".to_string()),
                repo_url: repo.path().to_string_lossy().to_string(),
                branch: None,
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
        let updated = update_theme(&db, "actor", "sample-theme", templates_root.path())
            .await
            .expect("failed to update theme");
        assert_eq!(updated.slug, "sample-theme");

        let refreshed = std::fs::read_to_string(&installed_file)
            .expect("failed to read refreshed installed file");
        assert_eq!(refreshed, "version-two");
    }
}
