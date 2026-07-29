use super::assets::normalize_optional;
use super::dashboard::{load_log_view, normalize_log_level_filter};
use super::state::*;
use super::*;

const WORDPRESS_IMPORT_FLASH_KEY: &str = "wordpress_import_flash";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WordpressImportFlash {
    imported_count: usize,
    updated_count: usize,
    updated_titles: Vec<String>,
}

impl From<&crate::WordpressImportSummary> for WordpressImportFlash {
    fn from(summary: &crate::WordpressImportSummary) -> Self {
        Self {
            imported_count: summary.imported_count,
            updated_count: summary.updated_count,
            updated_titles: summary.updated_titles.clone(),
        }
    }
}

impl From<WordpressImportFlash> for crate::WordpressImportSummary {
    fn from(flash: WordpressImportFlash) -> Self {
        Self {
            imported_count: flash.imported_count,
            updated_count: flash.updated_count,
            updated_titles: flash.updated_titles,
        }
    }
}

pub(crate) async fn admin_sites_new(
    State(state): State<AdminState>,
) -> Result<Response, SiteError> {
    let templates =
        get_template_names(state.db.as_ref(), state.site_templates_root.as_path(), None).await?;
    Ok(AdminSitesNewTemplate {
        template_shared: AdminTemplateData::new("Create Site"),
        templates,
    }
    .into_response())
}

pub(crate) async fn admin_sites_import(
    session: Session,
) -> Result<AdminSitesImportTemplate, SiteError> {
    require_global_admin(&session).await?;
    Ok(AdminSitesImportTemplate {
        template_shared: AdminTemplateData::new("Import Site"),
    })
}

#[allow(dead_code)]
pub(crate) async fn admin_sites_import_check(
    State(state): State<AdminState>,
    session: Session,
    Query(query): Query<SiteImportLookupQuery>,
) -> Result<Json<SiteImportLookupResponse>, SiteError> {
    require_global_admin(&session).await?;
    let short_name = query.short_name.trim();
    if short_name.is_empty() {
        return Err(SiteError::BadRequest("missing site short_name".to_string()));
    }

    let existing = entities::site::Entity::find()
        .filter(entities::site::Column::ShortName.eq(short_name))
        .one(state.db.as_ref())
        .await?;

    Ok(Json(SiteImportLookupResponse {
        short_name: short_name.to_string(),
        exists: existing.is_some(),
        full_title: existing.map(|site| site.full_title),
    }))
}

pub(crate) async fn admin_sites_import_create(
    State(state): State<AdminState>,
    session: Session,
    mut multipart: Multipart,
) -> Result<Redirect, SiteError> {
    let actor = require_global_admin(&session).await?;
    let mut upload_bytes: Option<Vec<u8>> = None;
    let mut replace_existing = false;

    loop {
        let field = multipart.next_field().await.map_err(|error| {
            SiteError::internal(format!("failed to parse site import: {error}"))
        })?;
        let Some(field) = field else { break };
        let field_name = field.name().map(str::to_string);
        if field_name.as_deref() == Some("replace_existing") {
            let bytes = field.bytes().await.map_err(|error| {
                SiteError::internal(format!("failed to read site import upload: {error}"))
            })?;
            replace_existing = !bytes.is_empty();
            continue;
        }
        if field_name.as_deref() != Some("file") {
            continue;
        }
        let bytes = field.bytes().await.map_err(|error| {
            SiteError::internal(format!("failed to read site import upload: {error}"))
        })?;
        if bytes.is_empty() {
            continue;
        }
        upload_bytes = Some(bytes.to_vec());
    }

    let upload_bytes = upload_bytes
        .ok_or_else(|| SiteError::BadRequest("provide a site export JSON file".to_string()))?;
    let export = deserialize_site_export(&upload_bytes)?;
    let existing_site = entities::site::Entity::find()
        .filter(entities::site::Column::ShortName.eq(export.site.short_name.clone()))
        .one(state.db.as_ref())
        .await?;
    let mut existing_site_files = None;
    if let Some(existing_site) = existing_site {
        if !replace_existing {
            return Err(SiteError::BadRequest(format!(
                "site {} already exists; select replace existing site to continue",
                export.site.short_name
            )));
        }
        let media_filenames = collect_site_media_filenames(state.db.as_ref(), existing_site.id)
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to collect site files: {error}"))
            })?;
        existing_site_files = Some((existing_site.id, media_filenames));
    }

    let txn = state.db.begin().await?;
    if let Some((site_id, _)) = existing_site_files.as_ref() {
        entities::audit_event::Entity::delete_many()
            .filter(entities::audit_event::Column::SiteId.eq(*site_id))
            .exec(&txn)
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to delete site audit events: {error}"))
            })?;
        delete_site(&txn, *site_id).await?;
    }
    let result = import_site_export(&txn, &export).await?;
    log_audit_event(
        &txn,
        &actor.subject,
        "import_site",
        "site",
        result.site_id,
        Some(result.site_id),
        Some(json!({
            "site_short_name": &result.site_short_name,
            "created_users": result.created_users,
            "reused_users": result.reused_users,
            "warnings": &result.warnings,
            "replaced_existing_site": existing_site_files.is_some(),
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log import audit: {error}")))?;
    txn.commit().await?;

    if let Some((site_id, media_filenames)) = existing_site_files {
        remove_deleted_site_files(state.upload_root.as_path(), site_id, &media_filenames).await?;
    }

    Ok(Redirect::to("/admin?imported=1"))
}

pub(crate) async fn ensure_site_owner_membership<C: ConnectionTrait>(
    db: &C,
    user_sub: &str,
    user_email: Option<&str>,
    site_id: Uuid,
) -> Result<Option<crate::entities::site_membership::Model>, SiteError> {
    let user = upsert_user_login(db, user_sub, user_email, None)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load user: {error}")))?;
    let existing = crate::get_membership_for_subject(db, site_id, user_sub)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;
    if existing.is_some() {
        return Ok(None);
    }

    let membership = create_membership(
        db,
        crate::NewMembership {
            site_id,
            user_id: user.id,
            role: SiteRole::Owner,
        },
    )
    .await?;

    Ok(Some(membership))
}

pub(crate) async fn admin_sites_create(
    State(state): State<AdminState>,
    session: Session,
    Form(form): Form<CreateSiteForm>,
) -> Result<Redirect, SiteError> {
    let template_name = form
        .template_name
        .unwrap_or_else(|| DEFAULT_TEMPLATE_NAME.to_string());
    let actor = current_user(&session).await?;
    let user_sub = actor.subject.clone();
    let short_name = form.short_name;
    let full_title = form.full_title;

    let txn = state.db.begin().await?;

    let user_sub = user_sub.clone();
    let template_name = template_name.clone();
    let site = create_site(&txn, short_name, full_title, template_name)
        .await
        .map_err(|error| SiteError::internal(format!("failed to create site: {error}")))?;
    log_audit_event(
        &txn,
        &actor.subject,
        "create_site",
        "site",
        &site.id,
        Some(site.id),
        Some(json!({ "short_name": &site.short_name, "full_title": &site.full_title })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log site audit: {error}")))?;

    if let Some(membership) =
        ensure_site_owner_membership(&txn, &user_sub, actor.email.as_deref(), site.id).await?
    {
        log_audit_event(
            &txn,
            &actor.subject,
            "create_membership",
            "site_membership",
            &membership.id,
            Some(membership.site_id),
            Some(json!({
                "site_id": membership.site_id,
                "user_id": membership.user_id,
                "role": membership.role.label()
            })),
        )
        .await
        .map_err(|error| SiteError::internal(format!("failed to log membership audit: {error}")))?;
    }
    txn.commit().await?;
    Ok(Redirect::to("/admin"))
}

pub(crate) fn parse_optional_datetime(
    value: Option<String>,
    label: &str,
) -> Result<Option<DateTime<Utc>>, SiteError> {
    value
        .map(|raw| {
            DateTime::parse_from_rfc3339(&raw)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|error| SiteError::internal(format!("invalid {label}: {error}")))
        })
        .transpose()
}

pub(crate) async fn get_template_names(
    db: &DatabaseConnection,
    templates_root: &StdPath,
    include_name: Option<&str>,
) -> Result<Vec<String>, SiteError> {
    available_template_names(db, templates_root, include_name).await
}

#[derive(Debug, Deserialize)]
pub(crate) struct SiteTemplateEditorQuery {
    saved: Option<String>,
    reset: Option<String>,
}

pub(crate) fn validate_customizable_template_file(
    file_name: &str,
) -> Result<&'static str, SiteError> {
    CUSTOMIZABLE_TEMPLATE_FILES
        .iter()
        .copied()
        .find(|candidate| *candidate == file_name)
        .ok_or_else(|| SiteError::BadRequest(format!("unsupported template file: {file_name}")))
}

pub(crate) fn site_template_edit_href(site_id: Uuid, file_name: &str) -> String {
    format!("/admin/site/{site_id}/settings/templates/{file_name}")
}

pub(crate) fn site_template_reset_href(site_id: Uuid, file_name: &str) -> String {
    format!("/admin/site/{site_id}/settings/templates/{file_name}/reset")
}

pub(crate) async fn describe_template_source_origin(
    templates_root: &StdPath,
    upload_root: &StdPath,
    site_id: Uuid,
    template_name: &str,
    file_name: &str,
) -> Result<(String, bool), SiteError> {
    let override_path =
        resolve_site_template_override_root_with_upload_root(upload_root, site_id).join(file_name);
    if fs::metadata(&override_path).await.is_ok() {
        return Ok(("site override".to_string(), true));
    }

    let shared_path = templates_root.join(template_name).join(file_name);
    if fs::metadata(&shared_path).await.is_ok() {
        return Ok((format!("shared template ({template_name})"), false));
    }

    Ok((format!("default template ({DEFAULT_TEMPLATE_NAME})"), false))
}

pub(crate) async fn load_editable_template_source(
    templates_root: &StdPath,
    upload_root: &StdPath,
    site_id: Uuid,
    template_name: &str,
    file_name: &str,
) -> Result<(String, String, bool), SiteError> {
    let override_path =
        resolve_site_template_override_root_with_upload_root(upload_root, site_id).join(file_name);
    if let Ok(source) = fs::read_to_string(&override_path).await {
        return Ok((source, "site override".to_string(), true));
    }

    let shared_path = templates_root.join(template_name).join(file_name);
    if let Ok(source) = fs::read_to_string(&shared_path).await {
        return Ok((source, format!("shared template ({template_name})"), false));
    }

    let default_path = templates_root.join(DEFAULT_TEMPLATE_NAME).join(file_name);
    let source = fs::read_to_string(&default_path).await.map_err(|error| {
        SiteError::internal(format!(
            "failed to load default template {file_name}: {error}"
        ))
    })?;
    Ok((
        source,
        format!("default template ({DEFAULT_TEMPLATE_NAME})"),
        false,
    ))
}

pub(crate) async fn build_site_template_file_rows(
    templates_root: &StdPath,
    upload_root: &StdPath,
    site_id: Uuid,
    template_name: &str,
) -> Result<Vec<AdminSiteTemplateFileRow>, SiteError> {
    let mut rows = Vec::with_capacity(CUSTOMIZABLE_TEMPLATE_FILES.len());
    for file_name in CUSTOMIZABLE_TEMPLATE_FILES {
        let (source_origin, override_exists) = describe_template_source_origin(
            templates_root,
            upload_root,
            site_id,
            template_name,
            file_name,
        )
        .await?;
        rows.push(AdminSiteTemplateFileRow {
            file_name: file_name.to_string(),
            source_origin,
            edit_href: site_template_edit_href(site_id, file_name),
            reset_href: site_template_reset_href(site_id, file_name),
            override_exists,
        });
    }

    Ok(rows)
}

pub(crate) async fn admin_site_settings(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminSiteSettingsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let viewer = current_user(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let template_files = build_site_template_file_rows(
        state.site_templates_root.as_path(),
        state.upload_root.as_path(),
        site.id,
        &site.template_name,
    )
    .await?;
    let templates = get_template_names(
        state.db.as_ref(),
        state.site_templates_root.as_path(),
        Some(&site.template_name),
    )
    .await?;
    let membership = if viewer.admin {
        None
    } else {
        get_membership_for_subject(state.db.as_ref(), site_id, &viewer.subject)
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to load site membership: {error}"))
            })?
    };
    let can_import_wordpress = viewer.admin
        || membership
            .as_ref()
            .is_some_and(|membership| role_satisfies(membership.role, SiteRole::Author));
    let can_manage_publish = viewer.admin
        || membership
            .as_ref()
            .is_some_and(|membership| role_satisfies(membership.role, SiteRole::Owner));
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let mut links = vec![
        AdminLink::new(&format!("/admin/site/{site_id}/memberships"), "Memberships"),
        AdminLink::new(
            &format!("/admin/site/{site_id}/content/scan"),
            "Scan content",
        ),
        AdminLink::new(
            &format!("/admin/site/{site_id}/assets/mass-import"),
            "Mass asset import",
        ),
    ];
    if viewer.admin
        || membership
            .as_ref()
            .is_some_and(|membership| role_satisfies(membership.role, SiteRole::Owner))
    {
        links.push(AdminLink::new(
            &format!("/admin/site/{site_id}/export.json"),
            "Download site export JSON",
        ));
    }
    if can_manage_publish {
        links.push(AdminLink::new(
            &format!("/admin/site/{site_id}/publish"),
            "Publish settings",
        ));
    }

    let import_flash = session
        .remove::<WordpressImportFlash>(WORDPRESS_IMPORT_FLASH_KEY)
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to read wordpress import flash: {error}"))
        })?;
    let template_shared = AdminTemplateData::new("Site Settings")
        .with_site_context(&site)
        .with_site_publish_configured(site_publish_configured)
        .with_links(links);
    let template_shared = if let Some(import_flash) = import_flash {
        template_shared.with_toast_message(
            &wordpress_import_message(&import_flash),
            &"wordpress-import",
        )
    } else {
        template_shared
    };

    Ok(AdminSiteSettingsTemplate {
        template_shared,
        site_id: site.id,
        site_short_name: site.short_name,
        full_title: site.full_title,
        template_name: site.template_name,
        publish_on_render: site.publish_on_render,
        internal_domains: site.internal_domains.join("\n"),
        mass_import_assets: site.mass_import_assets.unwrap_or_default(),
        can_delete_site: viewer.admin,
        can_import_wordpress,
        can_manage_publish,
        templates,
        template_files,
    })
}

pub(crate) async fn admin_site_publish(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AdminSitePublishQuery>,
) -> Result<AdminSitePublishTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;

    let publish_config = get_site_publish_config(state.db.as_ref(), site_id).await?;
    let site_publish_configured = publish_config
        .as_ref()
        .is_some_and(|config| config.method != PublishMethod::Disabled);
    let runs = list_site_publish_runs(state.db.as_ref(), site_id, 20).await?;
    let template_shared = AdminTemplateData::new("Publish Settings")
        .with_site_context(&site)
        .with_site_publish_configured(site_publish_configured)
        .with_links(vec![
            AdminLink::new(
                &format!("/admin/site/{site_id}/settings"),
                "Back to settings",
            ),
            AdminLink::new(&format!("/admin/site/{site_id}/render"), "Render site"),
        ]);
    let template_shared = if let Some(saved) = query.saved {
        let msg = if saved == 1 {
            "Publish configuration saved.".to_string()
        } else {
            "Publish configuration updated.".to_string()
        };
        template_shared.with_toast_message(&msg, &"saved")
    } else if let Some(queued) = query.queued {
        let msg = if queued == 1 {
            "Publish job queued.".to_string()
        } else {
            format!("{queued} publish jobs queued.")
        };
        template_shared.with_toast_message(&msg, &"queued")
    } else if let Some(disabled) = query.disabled {
        let msg = if disabled == 1 {
            "Publish configuration disabled.".to_string()
        } else {
            format!("{disabled} publish configurations disabled.")
        };
        template_shared.with_toast_message(&msg, &"disabled")
    } else {
        template_shared
    };

    let current_publish_method = publish_config
        .as_ref()
        .map(|config| config.method)
        .unwrap_or(PublishMethod::Disabled);
    let (
        method_label,
        method_description,
        endpoint_url,
        bucket,
        prefix,
        region,
        access_key_id,
        secret_present,
        force_path_style,
        ssh_host,
        ssh_user,
        ssh_port,
        remote_path,
        identity_file,
    ) = match publish_config {
        Some(config) => match config.method {
            PublishMethod::S3Compatible => {
                let config: S3CompatiblePublishConfig = serde_json::from_value(config.config_json)
                    .map_err(|error| {
                        SiteError::BadRequest(format!("invalid publish config: {error}"))
                    })?;
                (
                    PublishMethod::S3Compatible.label().to_string(),
                    "Render the site and mirror the output to an S3-compatible object store."
                        .to_string(),
                    config.endpoint_url.unwrap_or_default(),
                    config.bucket,
                    config.prefix,
                    config.region,
                    config.access_key_id,
                    true,
                    config.force_path_style,
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                )
            }
            PublishMethod::RsyncSsh => {
                let config: RsyncPublishConfig = serde_json::from_value(config.config_json)
                    .map_err(|error| {
                        SiteError::BadRequest(format!("invalid publish config: {error}"))
                    })?;
                (
                    PublishMethod::RsyncSsh.label().to_string(),
                    "Render the site and mirror the output to a remote SSH target using rsync."
                        .to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                    crate::publish::DEFAULT_S3_REGION.to_string(),
                    String::new(),
                    false,
                    false,
                    config.ssh_host,
                    config.ssh_user.unwrap_or_default(),
                    config
                        .ssh_port
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    config.remote_path,
                    config.identity_file.unwrap_or_default(),
                )
            }
            PublishMethod::Disabled => (
                PublishMethod::Disabled.label().to_string(),
                "Publishing is disabled for this site.".to_string(),
                String::new(),
                String::new(),
                String::new(),
                crate::publish::DEFAULT_S3_REGION.to_string(),
                String::new(),
                false,
                false,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
        },
        None => (
            PublishMethod::Disabled.label().to_string(),
            "Publishing is disabled for this site.".to_string(),
            String::new(),
            String::new(),
            String::new(),
            crate::publish::DEFAULT_S3_REGION.to_string(),
            String::new(),
            false,
            false,
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ),
    };
    let can_publish_now = matches!(
        current_publish_method,
        PublishMethod::S3Compatible | PublishMethod::RsyncSsh
    );
    let show_s3_config = current_publish_method == PublishMethod::S3Compatible;
    let show_rsync_config = current_publish_method == PublishMethod::RsyncSsh;

    Ok(AdminSitePublishTemplate {
        template_shared,
        site_id: site.id,
        site_short_name: site.short_name,
        full_title: site.full_title,
        method_label,
        method_description,
        endpoint_url,
        bucket,
        prefix,
        region,
        access_key_id,
        secret_present,
        force_path_style,
        ssh_host,
        ssh_user,
        ssh_port,
        remote_path,
        identity_file,
        can_publish_now,
        csrf_token: session
            .issue_csrf_token(&site_publish_csrf_scope(site_id))
            .await?,
        publish_csrf_token: session
            .issue_csrf_token(&site_publish_run_csrf_scope(site_id))
            .await?,
        runs: runs
            .into_iter()
            .map(|run| AdminSitePublishRunRow {
                id: run.id,
                detail_href: format!("/admin/site/{site_id}/publish/run/{}", run.id),
                status: run.status.to_string(),
                method: run.method.to_string(),
                actor_sub: run.actor_sub,
                created_at: run.created_at.to_rfc3339(),
                started_at: run
                    .started_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_else(|| "not started".to_string()),
                finished_at: run
                    .finished_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_else(|| "not finished".to_string()),
                rendered_file_count: run.rendered_file_count,
                published_file_count: run.published_file_count,
                deleted_object_count: run.deleted_object_count,
                error_message: run.error_message.unwrap_or_default(),
            })
            .collect(),
        publish_methods: PublishMethod::iter().collect(),
        current_publish_method,
        show_s3_config,
        show_rsync_config,
    })
}

pub(crate) async fn admin_site_publish_run_detail(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, run_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<AdminSitePublishRunQuery>,
) -> Result<AdminSitePublishRunTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let run = get_site_publish_run(state.db.as_ref(), site_id, run_id)
        .await?
        .ok_or(SiteError::NotFound)?;

    let line_limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let level_filter = normalize_log_level_filter(query.level.as_deref());
    let search_query = query
        .q
        .filter(|query| !query.trim().is_empty())
        .unwrap_or_else(|| run_id.to_string());
    let log_file_path = state.log_path.clone();
    let (lines, total_lines, matched_lines, truncated) = load_log_view(
        &log_file_path,
        line_limit,
        Some(search_query.as_str()),
        level_filter.as_deref(),
    )
    .await?;

    let template_shared = AdminTemplateData::new("Publish Run")
        .with_site_context(&site)
        .with_site_publish_configured(site_has_publish_config(state.db.as_ref(), site_id).await?)
        .with_links(vec![
            AdminLink::new(
                &format!("/admin/site/{site_id}/publish"),
                "Back to publish settings",
            ),
            AdminLink::new(&format!("/admin/logs?q={run_id}"), "Open in log viewer"),
            AdminLink::new(&format!("/admin/site/{site_id}/render"), "Render site"),
        ]);

    Ok(AdminSitePublishRunTemplate {
        template_shared,
        site_id: site.id,
        site_short_name: site.short_name,
        full_title: site.full_title,
        run_id,
        status: run.status.to_string(),
        method: run.method.to_string(),
        actor_sub: run.actor_sub,
        created_at: run.created_at.to_rfc3339(),
        started_at: run
            .started_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "not started".to_string()),
        finished_at: run
            .finished_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "not finished".to_string()),
        rendered_file_count: run.rendered_file_count,
        published_file_count: run.published_file_count,
        deleted_object_count: run.deleted_object_count,
        error_message: run.error_message.unwrap_or_default(),
        log_file_path: log_file_path.display().to_string(),
        search_query,
        level_filter: level_filter.unwrap_or_default(),
        line_limit,
        total_lines,
        matched_lines,
        truncated,
        lines,
    })
}

pub(crate) async fn admin_site_publish_update(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<UpdateSitePublishForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    session
        .validate_csrf_token(&site_publish_csrf_scope(site_id), &form.csrf_token)
        .await?;
    let actor = current_user(&session).await?.subject;

    let method = form.method.trim();
    if method == "disabled" {
        let txn = state.db.begin().await?;
        delete_site_publish_config(&txn, site_id).await?;
        log_audit_event(
            &txn,
            &actor,
            "disable_site_publish_config",
            "site_publish_config",
            &site_id.to_string(),
            Some(site_id),
            Some(json!({ "method": "disabled" })),
        )
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to log publish config disable audit: {error}"
            ))
        })?;
        txn.commit().await?;

        return Ok(Redirect::to(&format!(
            "/admin/site/{site_id}/publish?disabled=1"
        )));
    }

    let existing_s3_publish_config = if method == "s3_compatible" {
        Some(get_s3_publish_config(state.db.as_ref(), site_id).await?)
    } else {
        None
    };

    let txn = state.db.begin().await?;
    let saved = if method == "s3_compatible" {
        let endpoint_url = normalize_optional(Some(form.endpoint_url));
        let bucket = form.bucket.trim().to_string();
        let prefix = normalize_optional(form.prefix).unwrap_or_default();
        let region = form.region.trim().to_string();
        let access_key_id = form.access_key_id.trim().to_string();
        let secret_access_key = normalize_optional(form.secret_access_key);
        let force_path_style = form.force_path_style.is_some();

        if bucket.is_empty() {
            return Err(SiteError::BadRequest(
                "publish bucket is required".to_string(),
            ));
        }
        if region.is_empty() {
            return Err(SiteError::BadRequest(
                "publish region is required".to_string(),
            ));
        }
        if access_key_id.is_empty() {
            return Err(SiteError::BadRequest(
                "publish access_key_id is required".to_string(),
            ));
        }

        let secret_access_key = match (secret_access_key, existing_s3_publish_config.flatten()) {
            (Some(secret_access_key), _) => secret_access_key,
            (None, Some(existing)) => existing.secret_access_key,
            (None, None) => {
                return Err(SiteError::BadRequest(
                    "publish secret_access_key is required".to_string(),
                ));
            }
        };

        save_s3_publish_config(
            &txn,
            site_id,
            S3CompatiblePublishConfig {
                endpoint_url,
                bucket,
                prefix,
                region,
                access_key_id,
                secret_access_key,
                force_path_style,
            },
        )
        .await?
    } else if method == "rsync_ssh" {
        let ssh_host = form.ssh_host.trim().to_string();
        let ssh_user = normalize_optional(form.ssh_user);
        let ssh_port = match normalize_optional(form.ssh_port) {
            Some(port) => Some(port.parse::<u16>().map_err(|error| {
                SiteError::BadRequest(format!("publish ssh_port is invalid: {error}"))
            })?),
            None => None,
        };
        let remote_path = form.remote_path.trim().to_string();
        let identity_file = normalize_optional(form.identity_file);

        save_rsync_publish_config(
            &txn,
            site_id,
            RsyncPublishConfig {
                ssh_host,
                ssh_user,
                ssh_port,
                remote_path,
                identity_file,
            },
        )
        .await?
    } else {
        return Err(SiteError::BadRequest(format!(
            "unsupported publish method: {method}"
        )));
    };
    log_audit_event(
        &txn,
        &actor,
        "update_site_publish_config",
        "site_publish_config",
        &saved.site_id.to_string(),
        Some(site_id),
        Some(json!({
            "method": saved.method,
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log publish config audit: {error}")))?;
    txn.commit().await?;

    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/publish?saved=1"
    )))
}

pub(crate) async fn admin_site_publish_run(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<SitePublishActionForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    session
        .validate_csrf_token(&site_publish_run_csrf_scope(site_id), &form.csrf_token)
        .await?;
    let actor = current_user(&session).await?.subject;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;

    let _run = queue_site_publish(
        Arc::clone(&state.db),
        site_id,
        actor.clone(),
        state.site_templates_root.clone(),
        state.upload_root.clone(),
    )
    .await?;

    let txn = state.db.begin().await?;
    if let Err(error) = log_audit_event(
        &txn,
        &actor,
        "queue_site_publish",
        "site_publish_run",
        &site_id.to_string(),
        Some(site_id),
        Some(json!({
            "site_short_name": site.short_name,
        })),
    )
    .await
    {
        error!("failed to log publish queue audit: {error}");
    }
    txn.commit().await?;

    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/publish?queued=1"
    )))
}

pub(crate) async fn admin_site_wordpress_import(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;

    let mut upload_bytes: Option<Vec<u8>> = None;
    let mut uploaded_name: Option<String> = None;

    loop {
        let field = multipart.next_field().await.map_err(|error| {
            SiteError::internal(format!("failed to parse WordPress import upload: {error}"))
        })?;
        let Some(field) = field else { break };
        if field.name() != Some("file") {
            continue;
        }

        let uploaded_file_name = field.file_name().map(|value| value.to_string());
        let bytes = field.bytes().await.map_err(|error| {
            SiteError::internal(format!("failed to read WordPress import upload: {error}"))
        })?;
        if bytes.is_empty() {
            continue;
        }

        uploaded_name = uploaded_file_name;
        upload_bytes = Some(bytes.to_vec());
        break;
    }

    let upload_bytes = upload_bytes
        .ok_or_else(|| SiteError::BadRequest("provide a WordPress XML file".to_string()))?;
    let xml = String::from_utf8(upload_bytes).map_err(|error| {
        SiteError::BadRequest(format!("uploaded WordPress XML must be UTF-8: {error}"))
    })?;

    let txn = state.db.begin().await?;
    let imported = import_wordpress_xml(&txn, site_id, xml.as_str(), &actor.subject).await?;
    log_audit_event(
        &txn,
        &actor.subject,
        "import_wordpress",
        "site",
        &site.id.to_string(),
        Some(site.id),
        Some(json!({
            "imported": imported.imported_count,
            "updated": imported.updated_count,
            "updated_titles": imported.updated_titles,
            "file_name": uploaded_name,
        })),
    )
    .await
    .map_err(|error| {
        SiteError::internal(format!("failed to log WordPress import audit: {error}"))
    })?;
    session
        .insert(
            WORDPRESS_IMPORT_FLASH_KEY,
            WordpressImportFlash::from(&imported),
        )
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to store wordpress import flash: {error}"))
        })?;
    txn.commit().await?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/settings")))
}

pub(crate) async fn admin_site_settings_update(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<UpdateSiteSettingsForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let actor = current_user(&session).await?.subject;
    let full_title = form.full_title.trim().to_string();
    let template_name = form.template_name.trim().to_string();
    let publish_on_render = form.publish_on_render.is_some();
    if full_title.is_empty() {
        return Err(SiteError::internal("missing full title".to_string()));
    }
    if template_name.is_empty() {
        return Err(SiteError::internal("missing template name".to_string()));
    }

    let actor = actor.clone();
    let full_title = full_title.clone();
    let template_name = template_name.clone();
    let valid_templates = get_template_names(
        state.db.as_ref(),
        state.site_templates_root.as_path(),
        Some(&template_name),
    )
    .await?;
    if !valid_templates
        .iter()
        .any(|candidate| candidate == &template_name)
    {
        return Err(SiteError::BadRequest(format!(
            "unknown template: {template_name}"
        )));
    }
    let internal_domains = parse_internal_domains_form(form.internal_domains.as_deref());
    let mass_import_assets = form.mass_import_assets;
    let txn = state.db.begin().await?;
    let site = update_site_settings(
        &txn,
        site_id,
        full_title,
        template_name,
        publish_on_render,
        internal_domains,
        mass_import_assets,
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to update site: {error}")))?;
    log_audit_event(
        &txn,
        &actor,
        "update_site",
        "site",
        &site.id.to_string(),
        Some(site.id),
        Some(json!({
            "short_name": site.short_name,
            "full_title": site.full_title,
            "template_name": site.template_name,
            "publish_on_render": site.publish_on_render,
            "internal_domains": site.internal_domains,
            "mass_import_assets": site.mass_import_assets,
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log audit: {error}")))?;
    txn.commit().await?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/settings")))
}

fn parse_internal_domains_form(raw: Option<&str>) -> Vec<String> {
    let values = raw
        .unwrap_or_default()
        .split([',', '\n', '\r'])
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    crate::normalize_internal_domains(values)
}

fn wordpress_import_message(imported: &WordpressImportFlash) -> String {
    crate::format_wordpress_import_summary(&imported.clone().into())
}

pub(crate) async fn collect_site_media_filenames(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> Result<Vec<String>, SiteError> {
    let assets = list_assets(db, site_id).await?;
    let asset_ids = assets.iter().map(|asset| asset.id).collect::<Vec<_>>();
    let mut filenames = assets
        .into_iter()
        .map(|asset| asset.storage_basename)
        .collect::<Vec<_>>();

    if !asset_ids.is_empty() {
        let variants = entities::asset_variant::Entity::find()
            .filter(entities::asset_variant::Column::AssetId.is_in(asset_ids))
            .all(db)
            .await
            .map_err(SiteError::from)?;
        filenames.extend(variants.into_iter().map(|variant| variant.filename));
    }

    filenames.sort();
    filenames.dedup();
    Ok(filenames)
}

pub(crate) async fn remove_deleted_site_files(
    upload_root: &StdPath,
    site_id: Uuid,
    media_filenames: &[String],
) -> Result<(), SiteError> {
    for filename in media_filenames {
        let path = upload_root.join(filename);
        match fs::remove_file(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to remove deleted site media file {}: {error}",
                    path.display()
                )));
            }
        }
    }

    let override_root = resolve_site_template_override_root_with_upload_root(upload_root, site_id);
    match fs::remove_dir_all(&override_root).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to remove deleted site template overrides {}: {error}",
                override_root.display()
            )));
        }
    }

    Ok(())
}

pub(crate) async fn admin_site_delete_confirm(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminSiteDeleteConfirmTemplate, SiteError> {
    require_global_admin(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let csrf_token = session
        .issue_csrf_token(&site_delete_csrf_scope(site_id))
        .await?;

    Ok(AdminSiteDeleteConfirmTemplate {
        template_shared: AdminTemplateData::new("Confirm Site Deletion")
            .with_site_context(&site)
            .with_site_publish_configured(
                site_has_publish_config(state.db.as_ref(), site_id).await?,
            )
            .with_links(vec![AdminLink::new(
                &format!("/admin/site/{site_id}/settings"),
                "Back to settings",
            )]),
        site_id: site.id,
        site_full_title: site.full_title,
        site_short_name: site.short_name,
        csrf_token,
    })
}

pub(crate) async fn admin_site_delete(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<CsrfTokenForm>,
) -> Result<Redirect, SiteError> {
    let actor = require_global_admin(&session).await?.subject;
    session
        .validate_csrf_token(&site_delete_csrf_scope(site_id), &form.csrf_token)
        .await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let media_filenames = collect_site_media_filenames(state.db.as_ref(), site_id).await?;

    let txn = state.db.begin().await?;
    log_audit_event(
        &txn,
        &actor,
        "delete_site",
        "site",
        &site.id.to_string(),
        Some(site.id),
        Some(json!({
            "short_name": site.short_name,
            "full_title": site.full_title,
            "template_name": site.template_name
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log site delete audit: {error}")))?;
    delete_site(&txn, site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to delete site: {error}")))?;
    txn.commit().await?;

    remove_deleted_site_files(state.upload_root.as_path(), site_id, &media_filenames).await?;

    Ok(Redirect::to("/admin?deleted=1"))
}

pub(crate) async fn admin_site_template_editor(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, file_name)): Path<(Uuid, String)>,
    Query(query): Query<SiteTemplateEditorQuery>,
) -> Result<AdminSiteTemplateEditorTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let file_name = validate_customizable_template_file(&file_name)?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let (source, source_origin, override_exists) = load_editable_template_source(
        state.site_templates_root.as_path(),
        state.upload_root.as_path(),
        site.id,
        &site.template_name,
        file_name,
    )
    .await?;

    let template_shared = AdminTemplateData::new(&format!("Template Override: {file_name}"))
        .with_site_context(&site)
        .with_site_publish_configured(site_has_publish_config(state.db.as_ref(), site_id).await?)
        .with_links(vec![
            AdminLink::new(
                &format!("/admin/site/{site_id}/settings"),
                "Back to settings",
            ),
            AdminLink::new(&format!("/admin/site/{site_id}/render"), "Render site"),
        ]);
    let template_shared = if query.saved.is_some() {
        template_shared.with_toast_message(&"Template override saved.", &"saved")
    } else if query.reset.is_some() {
        template_shared.with_toast_message(&"Template override reset.", &"reset")
    } else {
        template_shared
    };

    Ok(AdminSiteTemplateEditorTemplate {
        template_shared,
        site_id: site.id,
        site_full_title: site.full_title,
        template_name: site.template_name,
        file_name: file_name.to_string(),
        source,
        source_origin,
        override_exists,
    })
}

pub(crate) async fn admin_site_template_override_update(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, file_name)): Path<(Uuid, String)>,
    Form(form): Form<UpdateSiteTemplateOverrideForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let file_name = validate_customizable_template_file(&file_name)?;
    if form.source.trim().is_empty() {
        return Err(SiteError::BadRequest("missing template source".to_string()));
    }

    let actor = current_user(&session).await?.subject;
    let override_root =
        resolve_site_template_override_root_with_upload_root(state.upload_root.as_path(), site_id);
    fs::create_dir_all(&override_root).await.map_err(|error| {
        SiteError::internal(format!(
            "failed to create template override directory: {error}"
        ))
    })?;
    let override_path = override_root.join(file_name);
    fs::write(&override_path, form.source.as_bytes())
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to write template override {file_name}: {error}"
            ))
        })?;

    let txn = state.db.begin().await?;
    log_audit_event(
        &txn,
        &actor,
        "update_template_override",
        "site_template_override",
        file_name,
        Some(site_id),
        Some(json!({ "file_name": file_name })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log template audit: {error}")))?;
    txn.commit().await?;

    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/settings/templates/{file_name}?saved=1"
    )))
}

pub(crate) async fn admin_site_template_override_reset(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, file_name)): Path<(Uuid, String)>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let file_name = validate_customizable_template_file(&file_name)?;
    let actor = current_user(&session).await?.subject;
    let override_path =
        resolve_site_template_override_root_with_upload_root(state.upload_root.as_path(), site_id)
            .join(file_name);
    match fs::remove_file(&override_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to remove template override {file_name}: {error}"
            )));
        }
    }

    let txn = state.db.begin().await?;
    log_audit_event(
        &txn,
        &actor,
        "reset_template_override",
        "site_template_override",
        file_name,
        Some(site_id),
        Some(json!({ "file_name": file_name })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log template reset audit: {error}")))?;
    txn.commit().await?;

    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/settings/templates/{file_name}?reset=1"
    )))
}

pub(crate) async fn admin_site_render(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AdminSiteRenderQuery>,
) -> Result<AdminRenderTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Editor).await?;
    let site = entities::site::Entity::find_by_id(site_id)
        .one(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?
        .ok_or(SiteError::NotFound)?;
    let files_written = render_site(
        state.db.as_ref(),
        site_id,
        state.site_templates_root.as_path(),
        state.rendered_root.as_path(),
        state.upload_root.as_path(),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to render site {site_id}: {error}")))?;

    let mut message = format!("Site rendered with {} file(s) written.", files_written);
    if site.publish_on_render || query.publish.is_some() {
        let actor_sub = current_user(&session).await?.subject;
        match publish_rendered_site(
            state.db.as_ref(),
            site_id,
            actor_sub,
            state.rendered_root.as_path(),
        )
        .await
        {
            Ok(outcome) => {
                message.push_str(&format!(
                    " Published {} file(s).",
                    outcome.published_file_count
                ));
            }
            Err(error) => {
                error!(
                    site_id = %site_id,
                    error = %error,
                    "failed to auto-publish rendered site"
                );
                message.push_str(&format!(" Publish failed: {error}"));
            }
        }
    }

    Ok(AdminRenderTemplate {
        template_shared: AdminTemplateData::new("Render Site")
            .with_site_context(&site)
            .with_site_publish_configured(
                site_has_publish_config(state.db.as_ref(), site_id).await?,
            )
            .with_message(&message)
            .with_links(vec![
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
                AdminLink::new(&format!("/admin/site/{site_id}/render"), "Run render again"),
            ]),
    })
}

pub(crate) async fn admin_site_export(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<Response, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let override_root =
        resolve_site_template_override_root_with_upload_root(state.upload_root.as_path(), site_id);
    let export = export_site_with_roots(
        state.db.as_ref(),
        site_id,
        state.upload_root.as_path(),
        &override_root,
    )
    .await?;
    let file_name = format!("{}-site-export.json", export.site.short_name);
    let body = serialize_site_export_pretty(&export)?;
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{file_name}\"")).map_err(
            |error| SiteError::internal(format!("failed to build export download header: {error}")),
        )?,
    );

    Ok(response)
}
