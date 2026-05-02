use super::state::*;
use super::*;

pub(crate) async fn admin_themes(
    State(state): State<AdminState>,
    session: Session,
    Query(query): Query<AdminThemesQuery>,
) -> Result<AdminThemesTemplate, SiteError> {
    require_global_admin(&session).await?;
    let themes = theme_admin_rows(state.db.as_ref(), state.site_templates_root.as_path()).await?;
    let ssh_keys = list_theme_ssh_keys(&state.theme_ssh_key_dir).await?;
    let template_shared = AdminTemplateData::new("Themes");
    let template_shared = if query.installed.is_some() {
        template_shared.with_toast_message(&"Theme installed.", &"installed")
    } else if query.updated.is_some() {
        template_shared.with_toast_message(&"Theme updated.", &"updated")
    } else if query.deleted.is_some() {
        template_shared.with_toast_message(&"Theme deleted.", &"deleted")
    } else {
        template_shared
    };

    Ok(AdminThemesTemplate {
        template_shared,
        themes,
        ssh_keys,
    })
}

pub(crate) async fn admin_themes_create(
    State(state): State<AdminState>,
    session: Session,
    Form(form): Form<ThemeInstallForm>,
) -> Result<Redirect, SiteError> {
    let actor = require_global_admin(&session).await?.subject;
    let repo_url = form.repo_url.trim().to_string();
    if repo_url.is_empty() {
        return Err(SiteError::BadRequest("missing repository url".to_string()));
    }

    let request = ThemeInstallRequest {
        slug: form.slug.and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }),
        repo_url,
        branch: form.branch.and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }),
        ssh_key_name: form.ssh_key_name,
    };
    let model = install_theme(
        state.db.as_ref(),
        &actor,
        state.site_templates_root.as_path(),
        &state.theme_ssh_key_dir,
        &state.theme_ssh_known_hosts_path,
        request,
    )
    .await?;

    Ok(Redirect::to(&format!(
        "/admin/themes?installed={}",
        model.slug
    )))
}

pub(crate) async fn admin_theme_edit(
    State(state): State<AdminState>,
    session: Session,
    Path(slug): Path<String>,
) -> Result<AdminThemeEditTemplate, SiteError> {
    require_global_admin(&session).await?;
    let theme = get_theme(
        state.db.as_ref(),
        state.site_templates_root.as_path(),
        &slug,
    )
    .await?;
    let ssh_keys = list_theme_ssh_keys(&state.theme_ssh_key_dir).await?;
    Ok(AdminThemeEditTemplate {
        template_shared: AdminTemplateData::new("Edit Theme"),
        slug: theme.slug,
        repo_url: theme.repo_url,
        branch: theme.branch.unwrap_or_default(),
        ssh_key_name: theme.ssh_key_name.unwrap_or_default(),
        ssh_keys,
    })
}

pub(crate) async fn admin_theme_edit_update(
    State(state): State<AdminState>,
    session: Session,
    Path(slug): Path<String>,
    Form(form): Form<ThemeEditForm>,
) -> Result<Redirect, SiteError> {
    let actor = require_global_admin(&session).await?.subject;
    let model = update_theme_metadata(
        state.db.as_ref(),
        &actor,
        &slug,
        state.site_templates_root.as_path(),
        &state.theme_ssh_key_dir,
        ThemeUpdateRequest {
            repo_url: form.repo_url,
            branch: form.branch,
            ssh_key_name: form.ssh_key_name,
        },
    )
    .await?;
    Ok(Redirect::to(&format!(
        "/admin/themes?updated={}",
        model.slug
    )))
}

pub(crate) async fn admin_theme_update(
    State(state): State<AdminState>,
    session: Session,
    Path(slug): Path<String>,
) -> Result<Redirect, SiteError> {
    let actor = require_global_admin(&session).await?.subject;
    let model = update_theme(
        state.db.as_ref(),
        &actor,
        &slug,
        state.site_templates_root.as_path(),
        &state.theme_ssh_key_dir,
        &state.theme_ssh_known_hosts_path,
    )
    .await?;

    Ok(Redirect::to(&format!(
        "/admin/themes?updated={}",
        model.slug
    )))
}

pub(crate) async fn admin_theme_delete(
    State(state): State<AdminState>,
    session: Session,
    Path(slug): Path<String>,
) -> Result<Redirect, SiteError> {
    let actor = require_global_admin(&session).await?.subject;
    delete_theme(
        state.db.as_ref(),
        &actor,
        &slug,
        state.site_templates_root.as_path(),
    )
    .await?;

    Ok(Redirect::to("/admin/themes?deleted=1"))
}
