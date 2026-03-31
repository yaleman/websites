use super::assets::*;
use super::sites::parse_optional_datetime;
use super::state::*;
use super::*;

pub(crate) fn preview_asset_prefix(site_id: Uuid) -> String {
    format!("/admin/site/{site_id}/preview-assets")
}

pub(crate) fn rewrite_preview_asset_urls(content: &str, site_id: Uuid) -> String {
    content.replace("/assets/", &format!("{}/", preview_asset_prefix(site_id)))
}

pub(crate) fn should_rewrite_preview_asset_body(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/javascript"
        || mime == "application/json"
        || mime == "image/svg+xml"
        || mime.ends_with("+xml")
}

pub(crate) fn sanitize_preview_asset_path(asset_path: &str) -> Result<PathBuf, SiteError> {
    let mut sanitized = PathBuf::new();
    for component in StdPath::new(asset_path).components() {
        match component {
            Component::Normal(value) => sanitized.push(value),
            _ => return Err(SiteError::NotFound),
        }
    }

    if sanitized.as_os_str().is_empty() {
        return Err(SiteError::NotFound);
    }

    Ok(sanitized)
}

pub(crate) async fn admin_site_content_list(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AdminContentListQuery>,
) -> Result<AdminContentListTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let site_publish_config = crate::publish::get_site_publish_config(state.db.as_ref(), site_id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load publish config for site {site_id}: {error}"
            ))
        })?;
    let site_publish_configured = site_publish_config.as_ref().is_some_and(|config| {
        config.method != crate::entities::site_publish_config::PublishMethod::Disabled
    });

    let page_type_filter = ContentListPageTypeFilter::from_query(query.page_type.as_deref());
    let sort_by = ContentListSortBy::from_query(query.sort_by.as_deref());

    match list_content(state.db.as_ref(), site_id, page_type_filter.page_type()).await {
        Ok(mut pages) => {
            sort_content_items(&mut pages, sort_by);

            Ok(AdminContentListTemplate {
                template_shared: AdminTemplateData::new("Content")
                    .with_site_context(&site)
                    .with_site_publish_configured(site_publish_configured)
                    .with_links(vec![
                        AdminLink::new(
                            &format!("/admin/site/{site_id}/content/new"),
                            "New content",
                        ),
                        AdminLink::new(
                            &format!("/admin/site/{site_id}/memberships"),
                            "Memberships",
                        ),
                        AdminLink::new(&format!("/admin/site/{site_id}/tags"), "Tags"),
                        AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Assets"),
                        AdminLink::new(
                            &format!("/admin/site/{site_id}/settings#wordpress-import"),
                            "WordPress import",
                        ),
                        AdminLink::new(&format!("/admin/site/{site_id}/render"), "Render"),
                        AdminLink::new(&format!("/admin/site/{site_id}/settings"), "Site settings"),
                    ]),

                site_id,
                page_type_options: page_type_filter.options(),
                current_sort_by: sort_by.as_str(),
                sort_headers: build_content_list_sort_headers(site_id, page_type_filter, sort_by),
                content_rows: pages
                    .into_iter()
                    .map(|item| AdminContentListRow {
                        edit_href: format!("/admin/site/{}/content/{}/edit", site_id, item.id),
                        title: item.title,
                        page_type: item.page_type.to_string(),
                        created_at: item.created_at.to_rfc3339(),
                        updated_at: item
                            .last_updated
                            .map(|value| value.to_rfc3339())
                            .unwrap_or_else(|| "-".to_string()),
                    })
                    .collect(),
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load content for site {site_id}: {error}"
        ))),
    }
}

pub(crate) async fn admin_site_memberships(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminMembershipsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let viewer = current_user(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let memberships = list_memberships(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load memberships: {error}")))?;
    let user_ids = memberships
        .iter()
        .map(|membership| membership.user_id)
        .collect::<Vec<_>>();
    let users = list_users_by_ids(state.db.as_ref(), user_ids)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load users: {error}")))?;
    let user_map = users
        .into_iter()
        .map(|user| (user.id, (user.subject, user.email)))
        .collect::<HashMap<_, _>>();
    let membership_user_ids = user_map
        .keys()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let membership_rows = memberships
        .into_iter()
        .map(|membership| {
            let (subject, email) = user_map
                .get(&membership.user_id)
                .cloned()
                .unwrap_or_else(|| ("unknown".to_string(), None));
            AdminMembershipRow {
                subject,
                email,
                role: membership.role,
                profile_href: if viewer.admin || viewer.id == membership.user_id {
                    Some(format!("/admin/users/{}", membership.user_id))
                } else {
                    None
                },
                update_href: format!("/admin/site/{site_id}/memberships/{}/update", membership.id),
                remove_href: format!("/admin/site/{site_id}/memberships/{}/remove", membership.id),
            }
        })
        .collect();
    let membership_candidates = list_users(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load candidate users: {error}")))?
        .into_iter()
        .filter(|user| !membership_user_ids.contains(&user.id))
        .map(|user| {
            let search_value = match &user.email {
                Some(email) => format!("{email} ({})", user.subject),
                None => user.subject.clone(),
            };
            AdminMembershipCandidateRow {
                user_id: user.id,
                subject: user.subject,
                email: user.email,
                search_value,
            }
        })
        .collect();

    Ok(AdminMembershipsTemplate {
        template_shared: AdminTemplateData::new("Memberships")
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured)
            .with_links(vec![AdminLink::new(
                &format!("/admin/site/{site_id}/settings"),
                "Site settings",
            )]),
        site_id: site.id,
        site_full_title: site.full_title,
        memberships: membership_rows,
        membership_candidates,
        roles: SiteRole::all_without_admin(),
    })
}

pub(crate) async fn admin_user_profile_redirect(session: Session) -> Result<Redirect, SiteError> {
    let user = current_user(&session).await?;
    Ok(Redirect::to(&format!("/admin/users/{}", user.id)))
}

pub(crate) async fn admin_user_profile(
    State(state): State<AdminState>,
    session: Session,
    Path(user_id): Path<Uuid>,
    Query(query): Query<AdminUserProfileQuery>,
) -> Result<AdminUserProfileTemplate, SiteError> {
    let viewer = current_user(&session).await?;
    let target = get_user_by_id(state.db.as_ref(), user_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load user {user_id}: {error}")))?
        .ok_or(SiteError::NotFound)?;

    if !can_view_user_profile(&viewer, &target) {
        return Err(SiteError::UnAuthorized(
            "cannot view another user's profile".to_string(),
        ));
    }

    build_admin_user_profile_template(
        &state,
        &session,
        &viewer,
        target,
        AdminUserProfileViewState {
            page_message: query.revoked.as_ref().map(|_| "Token revoked.".to_string()),
            page_message_is_toast: query.revoked.is_some(),
            clear_query_param: Some("revoked".to_string()),
            ..Default::default()
        },
    )
    .await
}

pub(crate) async fn admin_user_token_issue(
    State(state): State<AdminState>,
    session: Session,
    Path(user_id): Path<Uuid>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response, SiteError> {
    let viewer = current_user(&session).await?;
    let target = get_user_by_id(state.db.as_ref(), user_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load user {user_id}: {error}")))?
        .ok_or(SiteError::NotFound)?;
    if !can_view_user_profile(&viewer, &target) {
        return Err(SiteError::UnAuthorized(
            "cannot manage another user's tokens".to_string(),
        ));
    }
    let csrf_token = form
        .get("csrf_token")
        .map(String::as_str)
        .ok_or_else(|| SiteError::BadRequest("missing csrf token".to_string()))?;
    session
        .validate_csrf_token(&user_token_issue_csrf_scope(user_id), csrf_token)
        .await?;

    let label = form
        .get("label")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SiteError::BadRequest("token label is required".to_string()))?;
    let grant_mode = form
        .get("grant_mode")
        .map(String::as_str)
        .unwrap_or("current");
    let memberships = list_memberships_for_user_id(state.db.as_ref(), target.id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load sites: {error}")))?;
    let grants = match grant_mode {
        "current" => None,
        "restricted" => {
            let mut restricted_sites = memberships
                .into_iter()
                .filter_map(|membership| {
                    let enabled_key = format!("site_{}_enabled", membership.site_id);
                    if !form.contains_key(&enabled_key) {
                        return None;
                    }

                    let role_key = format!("site_{}_role", membership.site_id);
                    Some((membership, role_key))
                })
                .map(
                    |(membership, role_key)| -> Result<TokenSiteGrant, SiteError> {
                        let selected_role = form
                            .get(&role_key)
                            .map(String::as_str)
                            .ok_or_else(|| {
                                SiteError::BadRequest("missing token grant role".to_string())
                            })
                            .and_then(|value| {
                                <SiteRole as std::str::FromStr>::from_str(value)
                                    .map_err(SiteError::BadRequest)
                            })?;
                        if selected_role.is_admin()
                            || !role_satisfies(membership.role, selected_role)
                        {
                            return Err(SiteError::BadRequest(
                                "requested token grant exceeds user permissions".to_string(),
                            ));
                        }
                        Ok(TokenSiteGrant {
                            site_id: membership.site_id,
                            role: selected_role,
                        })
                    },
                )
                .collect::<Result<Vec<_>, _>>()?;
            restricted_sites.sort_by_key(|grant| grant.site_id);
            Some(TokenGrantSet {
                admin: target.admin && form.contains_key("grant_admin"),
                sites: restricted_sites,
            })
        }
        _ => return Err(SiteError::BadRequest("invalid grant mode".to_string())),
    };

    let jwt_signer = state.jwt_signer.clone();
    let jwt_issuer = state.jwt_issuer.clone();
    let target_for_issue = target.clone();
    let viewer_for_issue = viewer.clone();
    let issued_result = state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            let grants = grants.clone();
            let jwt_signer = jwt_signer.clone();
            let jwt_issuer = jwt_issuer.clone();
            let target_for_issue = target_for_issue.clone();
            let viewer_for_issue = viewer_for_issue.clone();
            let label = label.to_string();
            Box::pin(async move {
                let issued = issue_user_api_token(
                    txn,
                    jwt_signer.as_ref(),
                    &jwt_issuer,
                    &target_for_issue,
                    &viewer_for_issue,
                    &label,
                    grants.clone(),
                )
                .await?;
                log_audit_event(
                    txn,
                    &viewer_for_issue.subject,
                    "issue_api_token",
                    "user_api_token",
                    issued.row.id,
                    None,
                    Some(json!({
                        "user_id": target_for_issue.id,
                        "label": issued.row.label,
                        "grants": grants
                    })),
                )
                .await?;
                Ok(issued)
            })
        })
        .await;
    let issued = map_transaction_error(issued_result)?;

    let template = build_admin_user_profile_template(
        &state,
        &session,
        &viewer,
        target,
        AdminUserProfileViewState {
            issued_token: Some(issued.token),
            page_message: Some("Token issued. Copy it now; it won't be shown again.".to_string()),
            ..Default::default()
        },
    )
    .await?;
    Ok(no_store_response(template))
}

pub(crate) async fn admin_user_token_revoke(
    State(state): State<AdminState>,
    session: Session,
    Path((user_id, token_id)): Path<(Uuid, Uuid)>,
    Form(form): Form<CsrfTokenForm>,
) -> Result<Redirect, SiteError> {
    let viewer = current_user(&session).await?;
    let target = get_user_by_id(state.db.as_ref(), user_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load user {user_id}: {error}")))?
        .ok_or(SiteError::NotFound)?;
    if !can_view_user_profile(&viewer, &target) {
        return Err(SiteError::UnAuthorized(
            "cannot manage another user's tokens".to_string(),
        ));
    }
    session
        .validate_csrf_token(&user_token_revoke_csrf_scope(user_id), &form.csrf_token)
        .await?;

    let revoke_result = state
        .db
        .transaction::<_, _, SiteError>(|txn| {
            let viewer = viewer.clone();
            Box::pin(async move {
                let token = token_auth::get_user_api_token_by_id(txn, token_id)
                    .await?
                    .ok_or(SiteError::NotFound)?;
                if token.user_id != user_id {
                    return Err(SiteError::NotFound);
                }
                let revoked = revoke_user_api_token(txn, token_id, viewer.id).await?;
                log_audit_event(
                    txn,
                    &viewer.subject,
                    "revoke_api_token",
                    "user_api_token",
                    revoked.id,
                    None,
                    Some(json!({
                        "user_id": user_id,
                        "label": revoked.label
                    })),
                )
                .await?;
                Ok(())
            })
        })
        .await;
    map_transaction_error(revoke_result)?;

    Ok(Redirect::to(&format!("/admin/users/{user_id}?revoked=1")))
}

pub(crate) async fn admin_site_membership_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<MembershipCreateForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let subject = form.subject.trim().to_string();
    let user = if let Some(user_id) = form.user_id {
        let user = get_user_by_id(state.db.as_ref(), user_id)
            .await
            .map_err(|error| SiteError::internal(format!("failed to load user: {error}")))?;
        user.ok_or_else(|| SiteError::BadRequest("unknown user".to_string()))?
    } else {
        let subject = subject.trim();
        if subject.is_empty() {
            return Err(SiteError::internal("missing subject".to_string()));
        }
        entities::user::Entity::find()
            .filter(
                Condition::any()
                    .add(entities::user::Column::Subject.eq(subject))
                    .add(entities::user::Column::Email.eq(subject)),
            )
            .one(state.db.as_ref())
            .await
            .map_err(|error| SiteError::internal(format!("failed to load user: {error}")))?
            .ok_or_else(|| {
                SiteError::BadRequest(
                    "user must log in before site access can be granted".to_string(),
                )
            })?
    };
    let actor = current_user(&session).await?.subject;
    let txn = state.db.begin().await?;
    let actor = actor.clone();
    let membership = create_membership(
        &txn,
        crate::NewMembership {
            site_id,
            user_id: user.id,
            role: form.role,
        },
    )
    .await?;
    log_audit_event(
        &txn,
        &actor,
        "create_membership",
        "site_membership",
        &membership.id,
        Some(membership.site_id),
        Some(json!({
            "user_id": membership.user_id,
            "role": membership.role
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log membership audit: {error}")))?;
    txn.commit().await.map_err(|error| {
        SiteError::internal(format!("failed to commit membership creation: {error}"))
    })?;
    Ok(Redirect::to(&format!("/admin/site/{site_id}/memberships")))
}

pub(crate) async fn admin_site_membership_update(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, membership_id)): Path<(Uuid, Uuid)>,
    Form(form): Form<MembershipUpdateForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;

    let membership = get_membership_by_id(state.db.as_ref(), membership_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;
    let membership =
        membership.ok_or_else(|| SiteError::internal("membership not found".to_string()))?;
    if membership.site_id != site_id {
        return Err(SiteError::UnAuthorized(
            "membership does not belong to site".to_string(),
        ));
    }

    let actor = current_user(&session).await?.subject;
    let txn = state.db.begin().await?;
    let actor = actor.clone();

    let updated = update_membership_role(&txn, membership.id, form.role)
        .await
        .map_err(|error| SiteError::internal(format!("failed to update membership: {error}")))?;
    log_audit_event(
        &txn,
        &actor,
        "update_membership",
        "site_membership",
        &updated.id,
        Some(updated.site_id),
        Some(json!({
            "site_id": updated.site_id,
            "user_id": updated.user_id,
            "role": updated.role
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log membership audit: {error}")))?;

    txn.commit().await.map_err(|error| {
        SiteError::internal(format!("failed to commit membership update: {error}"))
    })?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/memberships")))
}

pub(crate) async fn admin_site_membership_remove(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, membership_id)): Path<(Uuid, Uuid)>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Owner).await?;
    let membership = get_membership_by_id(state.db.as_ref(), membership_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;
    let membership = membership.ok_or(SiteError::internal("membership not found".to_string()))?;
    if membership.site_id != site_id {
        return Err(SiteError::UnAuthorized(
            "membership does not belong to site".to_string(),
        ));
    }
    let actor = current_user(&session).await?.subject;
    let txn = state.db.begin().await?;
    delete_membership(&txn, membership.id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to remove membership: {error}")))?;
    log_audit_event(
        &txn,
        &actor,
        "delete_membership",
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
    .map_err(|error| SiteError::internal(format!("failed to log membership audit: {error}")))?;
    txn.commit().await.map_err(|error| {
        SiteError::internal(format!("failed to commit membership removal: {error}"))
    })?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/memberships")))
}

pub(crate) async fn get_global_search(
    State(state): State<AdminState>,
    session: Session,
    Query(query): Query<SearchQuery>,
) -> Result<AdminSearchTemplate, SiteError> {
    let query_text = query.q.unwrap_or_default();
    let query_text = query_text.trim().to_string();
    let mut rows = Vec::new();
    let mut message = "Search content across all sites.".to_string();

    if !query_text.is_empty() {
        let items = search_all_content(state.db.as_ref(), &query_text).await?;
        let site_title_by_id = list_sites(state.db.as_ref())
            .await?
            .into_iter()
            .map(|site| (site.id, site.full_title))
            .collect::<HashMap<_, _>>();
        message = format!("Found {} result(s) for \"{}\".", items.len(), query_text);
        rows = build_search_rows(items, &site_title_by_id);
    }

    current_user(&session).await?;

    Ok(AdminSearchTemplate {
        template_shared: AdminTemplateData::new("Search Content")
            .with_message(message)
            .with_nav_search_value(&query_text),
        rows,
        show_site_column: true,
    })
}

pub(crate) async fn get_site_search(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<SearchQuery>,
) -> Result<AdminSearchTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = entities::site::Entity::find_by_id(site_id)
        .one(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?
        .ok_or_else(|| SiteError::NotFound)?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let query_text = query.q.unwrap_or_default();
    let query_text = query_text.trim().to_string();
    let mut rows = Vec::new();
    let mut message: String = "Search content by title, slug, or body text.".to_string();

    if !query_text.is_empty() {
        let items = search_content(state.db.as_ref(), site_id, &query_text).await?;
        let site_title_by_id = HashMap::from_iter([(site.id, site.full_title.clone())]);
        message = format!("Found {} result(s) for \"{}\".", items.len(), query_text);
        rows = build_search_rows(items, &site_title_by_id);
    }

    Ok(AdminSearchTemplate {
        template_shared: AdminTemplateData::new("Search Content")
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured)
            .with_message(message)
            .with_nav_search_value(&query_text)
            .with_links(vec![AdminLink::new(
                &format!("/admin/site/{site_id}/content"),
                "Back to site dashboard",
            )]),
        rows,
        show_site_column: false,
    })
}

pub(crate) async fn admin_site_content_scan(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminContentScanTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    Ok(build_content_scan_template(
        &site,
        site_publish_configured,
        String::new(),
        5,
        "all",
        Vec::new(),
        None,
    ))
}

pub(crate) async fn admin_site_content_scan_run(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<ContentScanForm>,
) -> Result<AdminContentScanTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let scan_limit = normalize_content_scan_limit(form.scan_limit);
    let scan_reports = load_content_scan_reports(
        state.db.as_ref(),
        site_id,
        form.content_id,
        &form.domains,
        scan_limit,
    )
    .await?;
    let summary = Some(build_scan_summary(&scan_reports, 0, Vec::new(), Vec::new()));
    Ok(build_content_scan_template(
        &site,
        site_publish_configured,
        form.domains,
        scan_limit,
        scan_filter_value(form.filter.as_deref()),
        scan_reports.reports,
        summary,
    ))
}

pub(crate) async fn admin_site_content_scan_apply(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    RawForm(raw_form): RawForm,
) -> Result<AdminContentScanTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let form = parse_content_scan_apply_form(&raw_form)?;
    let scan_limit = normalize_content_scan_limit(form.scan_limit);
    let mut selected_issue_ids = deserialize_string_set(form.selected_issue_ids_json.as_deref())?;
    let mut remote_import_issue_ids =
        deserialize_string_set(form.remote_import_issue_ids_json.as_deref())?;
    remote_import_issue_ids.extend(form.remote_import_issue_id);
    selected_issue_ids.extend(remote_import_issue_ids.iter().cloned());
    let manual_asset_map = deserialize_manual_asset_map(form.asset_selections_json.as_deref())?;
    let scan_reports =
        load_content_scan_reports(state.db.as_ref(), site_id, None, &form.domains, scan_limit)
            .await?;

    let mut updated_items = Vec::new();
    let mut skipped_messages = Vec::new();
    let mut applied_count = 0usize;
    let mut imported_assets: HashMap<String, AssetReference> = HashMap::new();
    let txn = state.db.begin().await?;

    for report in &scan_reports.reports {
        let selected_for_content = report
            .issues
            .iter()
            .filter(|issue| selected_issue_ids.contains(&issue.issue_id))
            .count();
        if selected_for_content == 0 {
            continue;
        }

        let mut asset_replacements = HashMap::new();
        let mut missing_remote_issues = Vec::new();
        for issue in &report.issues {
            if !selected_issue_ids.contains(&issue.issue_id) {
                continue;
            }
            if let Some(selection) = manual_asset_map.get(&issue.issue_id) {
                asset_replacements.insert(
                    issue.issue_id.clone(),
                    AssetReference {
                        asset_id: selection.asset_id,
                        variant: selection.variant.clone(),
                        asset_label: selection.asset_label.clone(),
                    },
                );
                continue;
            }
            if remote_import_issue_ids.contains(&issue.issue_id)
                && let ScanAction::ReplaceAsset {
                    remote_url: Some(remote_url),
                    alt,
                    title,
                    ..
                } = &issue.action
            {
                let asset_reference = if let Some(existing) = imported_assets.get(remote_url) {
                    existing.clone()
                } else {
                    let imported = import_remote_scan_asset(
                        &txn,
                        state.oidc_client.as_ref(),
                        state.upload_root.as_path(),
                        site_id,
                        &actor.subject,
                        remote_url,
                    )
                    .await?;
                    imported_assets.insert(remote_url.clone(), imported.clone());
                    imported
                };
                let shortcode = format_asset_shortcode(
                    asset_reference.asset_id,
                    &asset_reference.variant,
                    alt,
                    title.as_deref(),
                );
                asset_replacements.insert(
                    issue.issue_id.clone(),
                    AssetReference {
                        asset_id: asset_reference.asset_id,
                        variant: asset_reference.variant.clone(),
                        asset_label: shortcode,
                    },
                );
            } else if let ScanAction::ReplaceAsset {
                suggested_asset: None,
                remote_url: Some(_),
                ..
            } = &issue.action
            {
                missing_remote_issues.push(issue.label.clone());
            }
        }

        if !missing_remote_issues.is_empty() {
            skipped_messages.push(format!(
                "{} was skipped because some image issues still need an asset selection or remote import.",
                report.content.title
            ));
            continue;
        }

        let applied_issues = crate::content_scan::apply_issue_replacements(
            &report.content.page_content,
            &report.issues,
            &selected_issue_ids,
            &asset_replacements,
            &remote_import_issue_ids,
        );
        if applied_issues.is_empty() {
            skipped_messages.push(format!(
                "{} had no applicable fixes after rescanning.",
                report.content.title
            ));
            continue;
        }

        let mut page_content = report.content.page_content.clone();
        for applied in &applied_issues {
            if applied.kind == "__remote_import__" {
                skipped_messages.push(format!(
                    "{} still has an unresolved remote image replacement.",
                    report.content.title
                ));
                continue;
            }
            if applied.end <= page_content.len() && applied.start <= applied.end {
                page_content.replace_range(applied.start..applied.end, &applied.replacement);
            }
        }

        let content = update_content(
            &txn,
            crate::UpdateContent {
                content_id: report.content.id,
                page_type: None,
                title: Some(report.content.title.clone()),
                slug: Some(report.content.slug.clone()),
                page_content: Some(page_content),
                draft: Some(report.content.draft),
                published_at: report.content.published_at,
                editor_sub: actor.subject.clone(),
            },
        )
        .await
        .map_err(SiteError::internal)?;
        applied_count = applied_count.saturating_add(applied_issues.len());
        updated_items.push(AdminContentScanUpdatedItem {
            title: content.title,
            applied_count: applied_issues.len(),
        });
    }

    log_audit_event(
        &txn,
        &actor.subject,
        "content_scan_apply",
        "content_item",
        &site_id.to_string(),
        Some(site_id),
        Some(json!({
            "applied_count": applied_count,
            "updated_titles": updated_items.iter().map(|item| item.title.clone()).collect::<Vec<_>>(),
            "selected_issue_count": selected_issue_ids.len(),
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log scan apply audit: {error}")))?;
    txn.commit().await?;

    let refreshed_reports =
        load_content_scan_reports(state.db.as_ref(), site_id, None, &form.domains, scan_limit)
            .await?;
    let summary = Some(build_scan_summary(
        &refreshed_reports,
        applied_count,
        updated_items,
        skipped_messages,
    ));
    Ok(build_content_scan_template(
        &site,
        site_publish_configured,
        form.domains,
        scan_limit,
        scan_filter_value(form.filter.as_deref()),
        refreshed_reports.reports,
        summary,
    ))
}

pub(crate) async fn admin_site_content_new(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminContentNewTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let tags = list_tags(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load tags: {error}")))?;
    let tags = tags
        .into_iter()
        .map(|tag| AdminTagOption {
            name: tag.name,
            selected: false,
        })
        .collect();

    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    Ok(AdminContentNewTemplate {
        template_shared: AdminTemplateData::new("Create Content")
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured),
        page_content: String::new(), // empty page content for the editor
        tags,
        site_id: site.id,
        allow_external_image: false,
    })
}

pub(crate) async fn admin_site_content_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<CreateContentForm>,
) -> Result<Redirect, SiteError> {
    let page_type = PageType::from_str(&form.page_type)
        .map_err(|error| SiteError::internal(error.to_string()))?;
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let tag_names = parse_tag_list(form.tag_list);
    let title = form.title;
    let slug = form.slug;
    let page_content = form.page_content;
    let draft = form.draft.unwrap_or(false);

    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let site_id = site.id;

    let tag_names = tag_names.clone();
    let txn = state.db.begin().await?;
    let content = create_content(
        &txn,
        NewContent {
            site_id,
            page_type,
            title,
            slug,
            page_content,
            draft,
            creator_sub: actor.subject.clone(),
            published_at: None,
        },
    )
    .await
    .map_err(|error| {
        SiteError::internal(format!(
            "failed to create content for site {site_id}: {error}"
        ))
    })?;

    if !tag_names.is_empty() {
        let revision = get_revision_by_number(&txn, content.id, 1)
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to load revision for tags: {error}"))
            })?
            .ok_or_else(|| SiteError::internal("missing revision for new content".to_string()))?;
        crate::assign_tags_to_content(&txn, content.site_id, content.id, revision.id, tag_names)
            .await
            .map_err(|error| SiteError::internal(format!("failed to assign tags: {error}")))?;
    }

    log_audit_event(
        &txn,
        &actor.subject,
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
    .map_err(|error| SiteError::internal(format!("failed to log content audit: {error}")))?;

    txn.commit().await.map_err(|error| {
        SiteError::internal(format!("failed to commit content creation: {error}"))
    })?;

    Ok(Redirect::to(&format!(
        "/admin/site/{}/content/{}/edit?saved=1",
        content.site_id, content.id
    )))
}

pub(crate) async fn admin_site_content_detail(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<AdminContentDetailTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;

    let content = entities::content_item::Entity::find_by_id(content_id)
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .one(state.db.as_ref())
        .await
        .map_err(|err| SiteError::internal(format!("failed to load content {content_id}: {err}")))?
        .ok_or(SiteError::NotFound)?;
    let tags = list_content_tags(state.db.as_ref(), content.id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load tags for content {content_id}: {error}"
            ))
        })?
        .into_iter()
        .map(|tag| tag.name)
        .collect::<Vec<_>>();
    let aliases = list_aliases(state.db.as_ref(), content.site_id, Some(content.id))
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load aliases for content {content_id}: {error}"
            ))
        })?
        .into_iter()
        .map(|alias| AdminContentAliasRow {
            kind: alias.kind,
            path: alias.alias_path,
        })
        .collect::<Vec<_>>();
    let revisions = list_revisions(state.db.as_ref(), content.id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load revisions for content {content_id}: {error}"
            ))
        })?;
    let site = get_by_id(state.db.as_ref(), content.site_id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load site {} for content {content_id}: {error}",
                content.site_id
            ))
        })?;
    let creator_label = entities::user::Entity::find()
        .filter(entities::user::Column::Subject.eq(content.creator_sub.clone()))
        .one(state.db.as_ref())
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load creator {} for content {content_id}: {error}",
                content.creator_sub
            ))
        })?
        .map(|user| match (user.display_name, user.email) {
            (Some(display_name), Some(email)) if display_name != email => {
                format!("{display_name} ({email})")
            }
            (Some(display_name), _) => display_name,
            (None, Some(email)) => email,
            (None, None) => user.subject,
        })
        .unwrap_or_else(|| content.creator_sub.clone());

    let route = content_primary_route(&content);
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site.id).await?;
    Ok(AdminContentDetailTemplate {
        template_shared: AdminTemplateData::new(format!("Content: /{route}"))
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured)
            .with_links(vec![
                AdminLink::new(
                    &format!(
                        "/admin/site/{}/content/{}/edit",
                        content.site_id, content.id
                    ),
                    "Open in editor",
                ),
                AdminLink::new(
                    &format!(
                        "/admin/site/{}/content/{}/revisions",
                        content.site_id, content.id
                    ),
                    "Revisions",
                ),
                AdminLink::new(
                    &format!("/admin/site/{}/content", content.site_id),
                    "Back to site dashboard",
                ),
            ]),
        title: content.title,
        page_type: content.page_type,
        status: content_status_label(content.draft),
        primary_route: display_route_path(&route),
        revisions_summary: latest_revision_summary(&revisions),
        tags,
        aliases,
        creator_label,
        content_id: content.id,
        site_id: content.site_id,
        slug: content.slug,
        created_at: content.created_at.to_rfc3339(),
        updated_at: format_optional_datetime(content.last_updated),
        published_at: format_optional_datetime(content.published_at),
        page_content: content.page_content,
    })
}

#[axum::debug_handler]
pub(crate) async fn admin_site_content_source(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<SourceEditorQuery>,
) -> Result<AdminContentSourceTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;

    let content = entities::content_item::Entity::find_by_id(content_id)
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .one(state.db.as_ref())
        .await
        .map_err(|err| SiteError::internal(format!("failed to load content {content_id}: {err}")))?
        .ok_or(SiteError::NotFound)?;
    let preview_href = format!(
        "/admin/site/{}/content/{}/preview",
        content.site_id, content.id
    );
    let back_href = format!("/admin/site/{}/content/{}", content.site_id, content.id);
    let site_tags = list_tags(state.db.as_ref(), content.site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site tags: {error}")))?;
    let selected_tags = list_content_tags(state.db.as_ref(), content.id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load content tags: {error}")))?;
    let selected_tag_names = selected_tags
        .into_iter()
        .map(|tag| tag.name)
        .collect::<std::collections::HashSet<_>>();
    let tags = site_tags
        .into_iter()
        .map(|tag| AdminTagOption {
            selected: selected_tag_names.contains(&tag.name),
            name: tag.name,
        })
        .collect();

    let draft = content.draft;
    let published_at = content.content_publish_timestamp();
    let title = content.title;
    let slug = content.slug;
    let page_content = content.page_content;
    let site = get_by_id(state.db.as_ref(), content.site_id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load site {} for content {content_id}: {error}",
                content.site_id
            ))
        })?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site.id).await?;

    let template_shared = AdminTemplateData::new(format!("Editing: {}", title))
        .with_links(vec![
            AdminLink::new(&preview_href, "Preview").with_target_blank(),
            AdminLink::new(&back_href, "Back to site dashboard"),
        ])
        .with_site_context(&site)
        .with_site_publish_configured(site_publish_configured);
    let template_shared = if query.saved.is_some() {
        template_shared.with_toast_message(&"Content saved.", &"saved")
    } else {
        template_shared
    };

    Ok(AdminContentSourceTemplate {
        template_shared,
        tags,
        title,
        slug,
        page_type: content.page_type.to_string(),
        draft,
        published_at,
        page_content,
        site_id: content.site_id,
        allow_external_image: true,
    })
}

pub(crate) async fn admin_site_content_source_update(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
    Form(form): Form<UpdateContentForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let draft = matches!(form.draft.as_str(), "true" | "1" | "yes");
    let published_at =
        parse_optional_datetime(normalize_optional(form.published_at), "published_at")?;
    let page_type = PageType::from_str(&form.page_type)
        .map_err(|error| SiteError::internal(error.to_string()))?;
    let title = form.title;
    let slug = form.slug;
    let page_content = form.page_content;
    let tag_names = parse_tag_list(form.tag_list);

    let txn = state.db.begin().await?;

    let content = update_content(
        &txn,
        crate::UpdateContent {
            content_id,
            page_type: Some(page_type),
            title: Some(title),
            slug: Some(slug),
            page_content: Some(page_content),
            draft: Some(draft),
            published_at,
            editor_sub: actor.subject.clone(),
        },
    )
    .await
    .map_err(|error| {
        SiteError::internal(format!("failed to update content {content_id}: {error}"))
    })?;
    let revision = entities::content_revision::Entity::find()
        .filter(entities::content_revision::Column::ContentId.eq(content.id))
        .order_by_desc(entities::content_revision::Column::RevisionNumber)
        .one(&txn)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load latest revision: {error}")))?
        .ok_or_else(|| SiteError::internal("missing revision for updated content".to_string()))?;
    sync_tags_to_content(&txn, content.site_id, content.id, revision.id, tag_names)
        .await
        .map_err(|error| SiteError::internal(format!("failed to sync tags: {error}")))?;

    log_audit_event(
        &txn,
        &actor.subject,
        "update_content",
        "content_item",
        &content.id.to_string(),
        Some(content.site_id),
        Some(json!({
                "page_type": content.page_type.to_string(),
                "slug": &content.slug,
                "title": &content.title,
                "draft": content.draft
            }
        )),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log update audit: {error}")))?;
    txn.commit().await?;
    Ok(Redirect::to(&format!(
        "/admin/site/{}/content/{}/edit?saved=1",
        content.site_id, content.id
    )))
}

pub(crate) async fn admin_site_content_preview(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<Html<String>, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let rendered = render_content_preview(
        state.db.as_ref(),
        site_id,
        content_id,
        state.site_templates_root,
        state.upload_root.as_path(),
    )
    .await?;
    Ok(Html(rewrite_preview_asset_urls(&rendered, site_id)))
}

pub(crate) async fn admin_site_preview_asset(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, asset_path)): Path<(Uuid, String)>,
) -> Result<Response, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let safe_asset_path = sanitize_preview_asset_path(&asset_path)?;
    let file_path = state
        .site_templates_root
        .join(site.template_name)
        .join("assets")
        .join(safe_asset_path);
    let metadata = fs::metadata(&file_path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SiteError::NotFound
        } else {
            SiteError::internal(format!(
                "failed to inspect preview asset {}: {error}",
                file_path.display()
            ))
        }
    })?;
    if !metadata.is_file() {
        return Err(SiteError::NotFound);
    }

    let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
    let mut body = fs::read(&file_path).await.map_err(|error| {
        SiteError::internal(format!(
            "failed to read preview asset {}: {error}",
            file_path.display()
        ))
    })?;

    if should_rewrite_preview_asset_body(mime.essence_str())
        && let Ok(text) = String::from_utf8(body.clone())
    {
        body = rewrite_preview_asset_urls(&text, site_id).into_bytes();
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(Body::from(body))
        .map_err(|error| {
            SiteError::internal(format!("failed to build preview asset response: {error}"))
        })
}

pub(crate) async fn admin_site_content_advanced(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/content/{content_id}"
    )))
}

pub(crate) async fn admin_site_content_revisions(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<AdminContentRevisionsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;

    let revisions = list_revisions(state.db.as_ref(), content_id)
        .await
        .map_err(|error| {
            SiteError::internal(format!(
                "failed to load revisions for {content_id}: {error}"
            ))
        })?;

    let diff_links = revisions
        .iter()
        .filter(|revision| revision.revision_number > 1)
        .map(|revision| {
            format!(
                "<li><a href=\"/admin/site/{}/content/{}/revisions/{}\">Diff revision {}</a></li>",
                site_id, content_id, revision.id, revision.revision_number
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let rows = revisions
        .into_iter()
        .map(|revision| AdminRow {
            label: format!("rev-{}", revision.revision_number),
            value: format!(
                "{} updated {} by {} [{}]",
                revision.id, revision.created_at, revision.editor_sub, revision.page_type
            ),
        })
        .collect();

    Ok(AdminContentRevisionsTemplate {
        template_shared: AdminTemplateData::new(format!("Revisions for {content_id}"))
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured)
            .with_links(vec![AdminLink::new(
                &format!("/admin/site/{site_id}/content/{content_id}"),
                "Back to site dashboard",
            )]),
        rows,
        inline_body: if diff_links.is_empty() {
            "<p>No diffs available for the first revision.</p>".to_string()
        } else {
            format!(
                "<section class=\"revision-diffs\"><h2>Revision Diffs</h2><ul>{}</ul></section>",
                diff_links
            )
        },
    })
}

pub(crate) async fn admin_site_revision_diff(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id, revision_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<AdminRevisionDiffTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let revision = get_revision(state.db.as_ref(), revision_id)
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to load revision {revision_id}: {error}"))
        })?;
    if revision.content_id != content_id || revision.site_id != site_id {
        return Err(SiteError::internal(
            "revision does not belong to requested content".to_string(),
        ));
    }

    let previous = if revision.revision_number > 1 {
        match get_revision_by_number(
            state.db.as_ref(),
            revision.content_id,
            revision.revision_number - 1,
        )
        .await
        {
            Ok(previous) => previous,
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to load previous revision: {error}"
                )));
            }
        }
    } else {
        None
    };

    let diff_text = if let Some(previous) = previous {
        let previous_label = format!("rev-{}", previous.revision_number);
        let current_label = format!("rev-{}", revision.revision_number);
        TextDiff::from_lines(&previous.page_content, &revision.page_content)
            .unified_diff()
            .header(&previous_label, &current_label)
            .to_string()
    } else {
        "No previous revision available.".to_string()
    };

    Ok(AdminRevisionDiffTemplate {
        template_shared: AdminTemplateData::new(format!(
            "Diff for rev-{}",
            revision.revision_number
        ))
        .with_site_context(&site)
        .with_site_publish_configured(site_publish_configured)
        .with_message(format!(
            "Comparing revision {} for content {}.",
            revision.revision_number, revision.content_id
        ))
        .with_links(vec![
            AdminLink::new(
                &format!("/admin/site/{}/content/{}/revisions", site_id, content_id),
                "Back to revisions",
            ),
            AdminLink::new(
                &format!("/admin/site/{}/content/{}", site_id, content_id),
                "Back to site dashboard",
            ),
        ]),

        rows: vec![
            AdminRow {
                label: "revision_id".to_string(),
                value: revision.id.to_string(),
            },
            AdminRow {
                label: "created_at".to_string(),
                value: revision.created_at.to_rfc3339(),
            },
            AdminRow {
                label: "editor_sub".to_string(),
                value: revision.editor_sub,
            },
        ],

        pre_body: diff_text,
    })
}

pub(crate) async fn admin_site_tags(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
) -> Result<AdminTagsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let site = get_by_id(state.db.as_ref(), site_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    match list_tags(state.db.as_ref(), site_id).await {
        Ok(tags) => {
            let tags = tags
                .into_iter()
                .map(|tag| AdminSiteTagRow {
                    id: tag.id,
                    name: tag.name,
                    delete_href: format!("/admin/site/{site_id}/tags/{}/delete", tag.id),
                })
                .collect();

            Ok(AdminTagsTemplate {
                template_shared: AdminTemplateData::new("Tags")
                    .with_site_context(&site)
                    .with_site_publish_configured(site_publish_configured)
                    .with_links(vec![AdminLink::new(
                        &format!("/admin/site/{site_id}/content"),
                        "Back to site dashboard",
                    )]),

                site_id,
                tags,
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load tags for site {site_id}: {error}"
        ))),
    }
}

pub(crate) async fn admin_site_tag_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<CreateTagForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Editor).await?;
    let actor = current_user(&session).await?;
    let name = form.name.trim();
    if name.is_empty() {
        return Err(SiteError::BadRequest("missing tag name".to_string()));
    }

    let txn = state.db.begin().await?;
    let tag = create_tag(
        &txn,
        crate::NewTag {
            site_id,
            name: name.to_string(),
        },
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to create tag: {error}")))?;
    log_audit_event(
        &txn,
        &actor.subject,
        "create_tag",
        "tag",
        &tag.id,
        Some(site_id),
        Some(json!({
            "name": tag.name,
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log tag audit: {error}")))?;
    txn.commit()
        .await
        .map_err(|error| SiteError::internal(format!("failed to commit tag creation: {error}")))?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/tags")))
}

pub(crate) async fn admin_site_tag_delete(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Editor).await?;
    let actor = current_user(&session).await?;

    let txn = state.db.begin().await?;
    delete_tag(&txn, site_id, tag_id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to delete tag: {error}")))?;
    log_audit_event(
        &txn,
        &actor.subject,
        "delete_tag",
        "tag",
        &tag_id,
        Some(site_id),
        None,
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log tag audit: {error}")))?;
    txn.commit()
        .await
        .map_err(|error| SiteError::internal(format!("failed to commit tag delete: {error}")))?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/tags")))
}

pub(crate) fn format_optional_datetime(value: Option<DateTime<Utc>>) -> String {
    value
        .map(|timestamp| timestamp.to_rfc3339())
        .unwrap_or_else(|| "n/a".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_asset_prefix_formats_site_scoped_path() {
        let site_id = Uuid::nil();

        assert_eq!(
            preview_asset_prefix(site_id),
            "/admin/site/00000000-0000-0000-0000-000000000000/preview-assets"
        );
    }

    #[test]
    fn rewrite_preview_asset_urls_rewrites_root_relative_assets() {
        let site_id = Uuid::nil();
        let rendered = rewrite_preview_asset_urls(
            r#"<link href="/assets/style.css"><style>body{background:url('/assets/bg.png')}</style>"#,
            site_id,
        );

        assert!(
            rendered.contains(
                "/admin/site/00000000-0000-0000-0000-000000000000/preview-assets/style.css"
            )
        );
        assert!(
            rendered
                .contains("/admin/site/00000000-0000-0000-0000-000000000000/preview-assets/bg.png")
        );
    }

    #[test]
    fn should_rewrite_preview_asset_body_matches_text_and_xml_types() {
        assert!(should_rewrite_preview_asset_body("text/html"));
        assert!(should_rewrite_preview_asset_body("application/json"));
        assert!(should_rewrite_preview_asset_body("image/svg+xml"));
        assert!(should_rewrite_preview_asset_body("application/atom+xml"));
        assert!(should_rewrite_preview_asset_body("application/javascript"));

        assert!(!should_rewrite_preview_asset_body("image/png"));
        assert!(!should_rewrite_preview_asset_body(
            "application/octet-stream"
        ));
    }

    #[test]
    fn sanitize_preview_asset_path_rejects_traversal_and_absolute_paths() {
        assert!(sanitize_preview_asset_path("../secret.txt").is_err());
        assert!(sanitize_preview_asset_path("/etc/passwd").is_err());
        assert!(sanitize_preview_asset_path("").is_err());
    }

    #[test]
    fn sanitize_preview_asset_path_allows_nested_relative_paths() {
        let path = sanitize_preview_asset_path("css/site/style.css")
            .expect("expected nested preview asset path to be accepted");

        assert_eq!(path, PathBuf::from("css/site/style.css"));
    }

    #[test]
    fn parse_content_scan_apply_form_parses_repeated_and_optional_fields() {
        let raw = b"domains=example.com&scan_limit=12&filter=review&selected_issue_ids_json=%5B%22issue-1%22%5D&remote_import_issue_ids_json=%5B%22remote-1%22%5D&remote_import_issue_id=remote-a&remote_import_issue_id=remote-b&asset_selections_json=%7B%22issue-1%22%3A%7B%22asset_id%22%3A%22019d%22%7D%7D";

        let form =
            parse_content_scan_apply_form(raw).expect("expected content scan apply form to parse");

        assert_eq!(form.domains, "example.com");
        assert_eq!(form.scan_limit, Some(12));
        assert_eq!(form.filter.as_deref(), Some("review"));
        assert_eq!(
            form.selected_issue_ids_json.as_deref(),
            Some(r#"["issue-1"]"#)
        );
        assert_eq!(
            form.remote_import_issue_ids_json.as_deref(),
            Some(r#"["remote-1"]"#)
        );
        assert_eq!(
            form.remote_import_issue_id,
            vec!["remote-a".to_string(), "remote-b".to_string()]
        );
        assert_eq!(
            form.asset_selections_json.as_deref(),
            Some(r#"{"issue-1":{"asset_id":"019d"}}"#)
        );
    }

    #[test]
    fn parse_content_scan_apply_form_rejects_missing_domains() {
        let error = parse_content_scan_apply_form(b"scan_limit=12")
            .expect_err("missing domains should fail");

        match error {
            SiteError::BadRequest(message) => {
                assert!(message.contains("missing domains field"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
