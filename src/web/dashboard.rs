use super::state::*;
use super::*;

pub(crate) async fn admin_root() -> Redirect {
    Redirect::to("/admin")
}

pub(crate) async fn not_found(OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        NotFoundTemplate {
            requested_path: uri.path().to_string(),
        },
    )
}

pub(crate) async fn get_sites(Query(query): Query<DashboardQuery>) -> Redirect {
    if query.imported.is_some() {
        Redirect::to("/admin?imported=1")
    } else if query.deleted.is_some() {
        Redirect::to("/admin?deleted=1")
    } else {
        Redirect::to("/admin")
    }
}

/// The home page
pub(crate) async fn get_index(
    State(state): State<AdminState>,
    session: Session,
    Query(query): Query<DashboardQuery>,
) -> Result<AdminIndexTemplate, SiteError> {
    let viewer = current_user(&session).await?;
    let sites = list_sites(state.db.as_ref()).await?;
    let mut links = vec![AdminLink::new("/admin/sites/new", "New site")];
    if viewer.admin {
        links.push(AdminLink::new("/admin/sites/import", "Import site"));
        links.push(AdminLink::new("/admin/themes", "Themes"));
        links.push(AdminLink::new("/admin/users", "Users"));
        links.push(AdminLink::new("/admin/logs", "Logs"));
    }

    let template_shared = AdminTemplateData::new("Admin Dashboard").with_links(links);
    let template_shared = if query.imported.is_some() {
        template_shared.with_toast_message("Site import complete.", "imported")
    } else if query.deleted.is_some() {
        template_shared.with_toast_message("Site deleted.", "deleted")
    } else {
        template_shared
    };

    Ok(AdminIndexTemplate {
        template_shared,
        sites,
    })
}

pub(crate) fn normalize_log_level_filter(level: Option<&str>) -> Option<String> {
    let level = level?.trim().to_lowercase();
    match level.as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => Some(level),
        _ => None,
    }
}

pub(crate) fn log_line_matches(
    line: &str,
    search_query: Option<&str>,
    level_filter: Option<&str>,
) -> bool {
    if let Some(level_filter) = level_filter {
        let level_token = format!(" {} ", level_filter.to_uppercase());
        if !line.contains(&level_token) {
            return false;
        }
    }

    if let Some(search_query) = search_query {
        let search_query = search_query.trim();
        if !search_query.is_empty() && !line.to_lowercase().contains(&search_query.to_lowercase()) {
            return false;
        }
    }

    true
}

pub(crate) async fn tail_log_file(
    path: &StdPath,
    max_bytes: usize,
) -> Result<Option<String>, SiteError> {
    let metadata = match fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SiteError::from(error)),
    };

    if !metadata.is_file() {
        return Ok(None);
    }

    let file_size = metadata.len() as usize;
    let start = file_size.saturating_sub(max_bytes);
    let mut file = fs::File::open(path).await?;
    if start > 0 {
        file.seek(SeekFrom::Start(start as u64)).await?;
    }

    let mut buffer = Vec::with_capacity(file_size.saturating_sub(start));
    file.read_to_end(&mut buffer).await?;
    let mut contents = String::from_utf8_lossy(&buffer).into_owned();

    if start > 0
        && let Some(split_at) = contents.find('\n')
    {
        contents = contents[split_at + 1..].to_string();
    }

    Ok(Some(contents))
}

pub(crate) async fn load_log_view(
    path: &StdPath,
    line_limit: usize,
    search_query: Option<&str>,
    level_filter: Option<&str>,
) -> Result<(Vec<String>, usize, usize, bool), SiteError> {
    let Some(contents) = tail_log_file(path, 512 * 1024).await? else {
        return Ok((Vec::new(), 0, 0, false));
    };

    let raw_lines: Vec<String> = contents.lines().map(|line| line.to_string()).collect();
    let total_lines = raw_lines.len();
    let mut filtered: Vec<String> = raw_lines
        .into_iter()
        .filter(|line| log_line_matches(line, search_query, level_filter))
        .collect();
    let matched_lines = filtered.len();
    let truncated = matched_lines > line_limit;

    if truncated {
        filtered = filtered.into_iter().rev().take(line_limit).collect();
        filtered.reverse();
    }

    Ok((filtered, total_lines, matched_lines, truncated))
}

pub(crate) async fn admin_logs(
    State(state): State<AdminState>,
    session: Session,
    Query(query): Query<AdminLogsQuery>,
) -> Result<AdminLogsTemplate, SiteError> {
    require_global_admin(&session).await?;
    let line_limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let level_filter = normalize_log_level_filter(query.level.as_deref());
    let search_query = query.q.unwrap_or_default();
    let log_file_path = state.log_path.clone();
    let (lines, total_lines, matched_lines, truncated) = load_log_view(
        &log_file_path,
        line_limit,
        (!search_query.is_empty()).then_some(search_query.as_str()),
        level_filter.as_deref(),
    )
    .await?;

    Ok(AdminLogsTemplate {
        template_shared: AdminTemplateData::new("Logs"),
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

pub(crate) async fn admin_users(
    State(state): State<AdminState>,
    session: Session,
    Query(query): Query<AdminUsersQuery>,
) -> Result<AdminUsersTemplate, SiteError> {
    require_global_admin(&session).await?;
    let mut users = list_users(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load users: {error}")))?;
    users.sort_by(|left, right| left.subject.cmp(&right.subject));
    let template_shared = AdminTemplateData::new("Users");
    let template_shared = if query.created.is_some() {
        template_shared.with_toast_message("User created.", "created")
    } else {
        template_shared
    };

    Ok(AdminUsersTemplate {
        template_shared,
        create_user_csrf_token: session
            .issue_csrf_token(admin_user_create_csrf_scope())
            .await?,
        users: users
            .into_iter()
            .map(|user| AdminUserListRow {
                profile_href: format!("/admin/users/{}", user.id),
                subject: user.subject,
                display_name: user.display_name.unwrap_or_else(|| "n/a".to_string()),
                email: user.email.unwrap_or_else(|| "n/a".to_string()),
                is_admin: user.admin,
            })
            .collect(),
    })
}

pub(crate) async fn admin_users_create(
    State(state): State<AdminState>,
    session: Session,
    Form(form): Form<AdminUserCreateForm>,
) -> Result<Redirect, SiteError> {
    let actor = require_global_admin(&session).await?;
    session
        .validate_csrf_token(admin_user_create_csrf_scope(), &form.csrf_token)
        .await?;
    let subject = form.subject.trim().to_string();
    if subject.is_empty() {
        return Err(SiteError::BadRequest("subject is required".to_string()));
    }
    let existing = entities::user::Entity::find()
        .filter(entities::user::Column::Subject.eq(subject.clone()))
        .one(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load users: {error}")))?;
    if existing.is_some() {
        return Err(SiteError::BadRequest("subject already exists".to_string()));
    }
    let email = form
        .email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let display_name = form
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let is_admin = form.admin.is_some();
    let create_result = state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            let subject = subject.clone();
            let email = email.clone();
            let display_name = display_name.clone();
            Box::pin(async move {
                let user = entities::user::create_user(
                    txn,
                    &subject,
                    email.as_deref(),
                    display_name.as_deref(),
                    is_admin,
                )
                .await?;
                log_audit_event(
                    txn,
                    &actor.subject,
                    "create_user",
                    "user",
                    user.id,
                    None,
                    Some(json!({
                        "subject": user.subject,
                        "email": user.email,
                        "display_name": user.display_name,
                        "admin": user.admin
                    })),
                )
                .await?;
                Ok(())
            })
        })
        .await;
    match map_transaction_error(create_result) {
        Err(SiteError::Database(message))
            if message.contains("UNIQUE constraint failed: user.subject")
                || message.contains("idx_user_subject") =>
        {
            return Err(SiteError::BadRequest("subject already exists".to_string()));
        }
        Err(error) => return Err(error),
        Ok(()) => {}
    }

    Ok(Redirect::to("/admin/users?created=1"))
}

pub(crate) async fn admin_login(
    State(state): State<AdminState>,
    session: Session,
) -> Result<Response, SiteError> {
    let client = match build_oidc_client(&state).await {
        Ok(client) => client,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to initialize OIDC client: {error}"
            )));
        }
    };

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_state, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    if session
        .insert(SESSION_OIDC_STATE_KEY, csrf_state.secret().to_string())
        .await
        .is_err()
        || session
            .insert(SESSION_OIDC_PKCE_KEY, pkce_verifier.secret().to_string())
            .await
            .is_err()
        || session
            .insert(SESSION_OIDC_NONCE_KEY, nonce.secret().to_string())
            .await
            .is_err()
    {
        return Err(SiteError::internal(
            "failed to persist OIDC session data".to_string(),
        ));
    }

    let auth_url = auth_url.to_string();
    Ok(Redirect::to(&auth_url).into_response())
}

pub(crate) async fn admin_logout(session: Session) -> Redirect {
    let _ = session.clear().await;
    Redirect::to("/admin/login")
}
