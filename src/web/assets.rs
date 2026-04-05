use super::state::*;
use super::*;

pub(crate) const ASSET_UPLOAD_TOAST_QUERY_PARAM: &str = "uploaded";

pub(crate) struct UploadedAssetInput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) original_filename: String,
    pub(crate) mime_type: Option<String>,
}

pub(crate) struct ParsedAssetCreateRequest {
    pub(crate) uploads: Vec<UploadedAssetInput>,
    pub(crate) source_url: Option<Url>,
}

pub(crate) struct StoredAssetBatch {
    pub(crate) assets: Vec<entities::asset::Model>,
    pub(crate) stored_filenames: Vec<String>,
}

pub(crate) fn display_route_path(route: &str) -> String {
    format!("/{}", route.trim_matches('/'))
}

pub(crate) fn content_status_label(draft: bool) -> String {
    if draft {
        "Draft".to_string()
    } else {
        "Published".to_string()
    }
}

pub(crate) fn latest_revision_summary(revisions: &[entities::content_revision::Model]) -> String {
    revisions
        .first()
        .map(|revision| {
            format!(
                "{} revisions, latest is revision {} by {} on {}",
                revisions.len(),
                revision.revision_number,
                revision.editor_sub,
                revision.created_at.to_rfc3339()
            )
        })
        .unwrap_or_else(|| "No revisions recorded.".to_string())
}

pub(crate) fn normalize_remote_asset_url(value: &str) -> Result<Option<Url>, SiteError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let parsed = Url::parse(trimmed)
        .map_err(|error| SiteError::internal(format!("invalid asset import url: {error}")))?;
    match parsed.scheme() {
        "http" | "https" => Ok(Some(parsed)),
        scheme => Err(SiteError::internal(format!(
            "unsupported asset import url scheme: {scheme}"
        ))),
    }
}

pub(crate) fn extension_from_mime_type(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "image/webp" => "webp",
        _ => "bin",
    }
}

pub(crate) async fn fetch_remote_asset(
    client: &reqwest::Client,
    source_url: Url,
) -> Result<(Vec<u8>, String, Option<String>), SiteError> {
    let response = client
        .get(source_url.clone())
        .send()
        .await
        .map_err(|error| SiteError::internal(format!("failed to fetch asset url: {error}")))?;
    if !response.status().is_success() {
        return Err(SiteError::internal(format!(
            "asset import request failed with status {}",
            response.status()
        )));
    }

    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_string());
    if let Some(mime_type) = mime_type.as_deref()
        && !mime_type.starts_with("image/")
    {
        return Err(SiteError::internal(format!(
            "asset import url did not return an image: {mime_type}"
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|error| SiteError::internal(format!("failed to read imported asset: {error}")))?;
    if bytes.is_empty() {
        return Err(SiteError::internal(
            "asset import returned an empty response body".to_string(),
        ));
    }

    let path_filename = source_url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .map(|segment| segment.to_string())
        .filter(|segment| !segment.is_empty());
    let original_filename = match path_filename {
        Some(filename) => filename,
        None => format!(
            "remote-image.{}",
            extension_from_mime_type(mime_type.as_deref().unwrap_or("application/octet-stream"))
        ),
    };

    Ok((bytes.to_vec(), original_filename, mime_type))
}

pub(crate) async fn import_remote_scan_asset<C: ConnectionTrait>(
    db: &C,
    client: &reqwest::Client,
    upload_root: &StdPath,
    site_id: Uuid,
    uploader_sub: &str,
    remote_url: &str,
) -> Result<AssetReference, SiteError> {
    let url = normalize_remote_asset_url(remote_url)?
        .ok_or_else(|| SiteError::BadRequest("missing remote asset url".to_string()))?;
    let (bytes, original_filename, mime_type) = fetch_remote_asset(client, url).await?;
    let asset = store_uploaded_asset(
        db,
        upload_root,
        site_id,
        uploader_sub,
        bytes,
        original_filename,
        mime_type,
    )
    .await?;
    Ok(AssetReference {
        asset_id: asset.id,
        variant: "original".to_string(),
        asset_label: asset.original_filename,
    })
}

pub(crate) fn format_asset_dimensions(width: Option<i32>, height: Option<i32>) -> String {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => format!("{width} x {height}"),
        (Some(width), None) if width > 0 => format!("{width}w"),
        (None, Some(height)) if height > 0 => format!("{height}h"),
        _ => "n/a".to_string(),
    }
}

pub(crate) fn asset_upload_message(uploaded_count: usize) -> String {
    match uploaded_count {
        1 => "Uploaded 1 asset.".to_string(),
        count => format!("Uploaded {count} assets."),
    }
}

pub(crate) async fn parse_asset_create_request(
    mut multipart: Multipart,
) -> Result<ParsedAssetCreateRequest, SiteError> {
    let mut uploads = Vec::new();
    let mut source_url: Option<Url> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(field) => field,
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to parse upload: {error}"
                )));
            }
        };

        let Some(field) = field else { break };
        match field.name() {
            Some("files") => {
                let original_filename = field
                    .file_name()
                    .map(|value| value.to_string())
                    .ok_or_else(|| SiteError::internal("missing original filename".to_string()))?;
                let mime_type = field.content_type().map(|value| value.to_string());
                let bytes = match field.bytes().await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        return Err(SiteError::internal(format!(
                            "failed to read upload: {error}"
                        )));
                    }
                };
                if bytes.is_empty() {
                    continue;
                }

                uploads.push(UploadedAssetInput {
                    bytes: bytes.to_vec(),
                    original_filename,
                    mime_type,
                });
            }
            Some("source_url") => {
                let value = match field.text().await {
                    Ok(value) => value,
                    Err(error) => {
                        return Err(SiteError::internal(format!(
                            "failed to read asset import url: {error}"
                        )));
                    }
                };
                source_url = normalize_remote_asset_url(&value)?;
            }
            _ => continue,
        }
    }

    if !uploads.is_empty() && source_url.is_some() {
        return Err(SiteError::BadRequest(
            "provide uploaded files or an image url, not both".to_string(),
        ));
    }

    Ok(ParsedAssetCreateRequest {
        uploads,
        source_url,
    })
}

pub(crate) async fn resolve_asset_create_request(
    client: &reqwest::Client,
    parsed: ParsedAssetCreateRequest,
) -> Result<Vec<UploadedAssetInput>, SiteError> {
    if !parsed.uploads.is_empty() {
        return Ok(parsed.uploads);
    }

    if let Some(source_url) = parsed.source_url {
        let (bytes, original_filename, mime_type) = fetch_remote_asset(client, source_url).await?;
        return Ok(vec![UploadedAssetInput {
            bytes,
            original_filename,
            mime_type,
        }]);
    }

    Err(SiteError::BadRequest(
        "provide uploaded files or an image url".to_string(),
    ))
}

#[allow(dead_code)]
pub(crate) async fn parse_asset_upload(
    mut multipart: Multipart,
) -> Result<
    (
        Option<(Vec<u8>, Option<String>, Option<String>)>,
        Option<Url>,
    ),
    SiteError,
> {
    let mut upload_bytes: Option<(Vec<u8>, Option<String>, Option<String>)> = None;
    let mut source_url: Option<Url> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(field) => field,
            Err(error) => {
                return Err(SiteError::internal(format!(
                    "failed to parse upload: {error}"
                )));
            }
        };

        let Some(field) = field else { break };
        match field.name() {
            Some("file") => {
                let field_filename = field.file_name().map(|value| value.to_string());
                let field_mime_type = field.content_type().map(|value| value.to_string());
                let bytes = match field.bytes().await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        return Err(SiteError::internal(format!(
                            "failed to read upload: {error}"
                        )));
                    }
                };
                if bytes.is_empty() {
                    continue;
                }

                upload_bytes = Some((bytes.to_vec(), field_filename, field_mime_type));
            }
            Some("source_url") => {
                let value = match field.text().await {
                    Ok(value) => value,
                    Err(error) => {
                        return Err(SiteError::internal(format!(
                            "failed to read asset import url: {error}"
                        )));
                    }
                };
                source_url = normalize_remote_asset_url(&value)?;
            }
            _ => continue,
        }
    }

    Ok((upload_bytes, source_url))
}

#[allow(dead_code)]
pub(crate) async fn resolve_asset_upload(
    client: &reqwest::Client,
    upload_bytes: Option<(Vec<u8>, Option<String>, Option<String>)>,
    source_url: Option<Url>,
) -> Result<(Vec<u8>, String, Option<String>), SiteError> {
    if let Some((bytes, original_filename, mime_type)) = upload_bytes {
        let Some(original_filename) = original_filename else {
            return Err(SiteError::internal("missing original filename".to_string()));
        };
        return Ok((bytes, original_filename, mime_type));
    }

    if let Some(source_url) = source_url {
        return fetch_remote_asset(client, source_url).await;
    }

    Err(SiteError::internal(
        "provide a file upload or an image url".to_string(),
    ))
}

pub(crate) async fn create_uploaded_asset_batch<C: ConnectionTrait>(
    db: &C,
    upload_root: &StdPath,
    site_id: Uuid,
    uploader_sub: &str,
    uploads: Vec<UploadedAssetInput>,
) -> Result<StoredAssetBatch, SiteError> {
    let mut assets = Vec::with_capacity(uploads.len());
    let mut stored_filenames = Vec::new();

    for upload in uploads {
        let stored_asset = match crate::store_uploaded_asset_with_filenames(
            db,
            upload_root,
            site_id,
            uploader_sub,
            upload.bytes,
            upload.original_filename,
            upload.mime_type,
        )
        .await
        {
            Ok(stored_asset) => stored_asset,
            Err(error) => {
                crate::cleanup_uploaded_asset_files(upload_root, &stored_filenames)
                    .await
                    .map_err(|cleanup_error| {
                        SiteError::internal(format!(
                            "failed to clean up batch upload files: {cleanup_error}"
                        ))
                    })?;
                return Err(error);
            }
        };

        stored_filenames.extend(stored_asset.stored_filenames.clone());
        assets.push(stored_asset.asset);
    }

    Ok(StoredAssetBatch {
        assets,
        stored_filenames,
    })
}

pub(crate) async fn log_asset_create_events<C: ConnectionTrait>(
    db: &C,
    actor_sub: &str,
    event_type: &str,
    assets: &[entities::asset::Model],
) -> Result<(), SiteError> {
    for asset in assets {
        log_audit_event(
            db,
            actor_sub,
            event_type,
            "asset",
            &asset.id.to_string(),
            Some(asset.site_id),
            Some(json!({
                "original_filename": &asset.original_filename,
                "storage_basename": &asset.storage_basename,
                "mime_type": &asset.mime_type
            })),
        )
        .await
        .map_err(|error| SiteError::internal(format!("failed to log asset audit: {error}")))?;
    }

    Ok(())
}

pub(crate) async fn build_admin_asset_rows<C: ConnectionTrait>(
    db: &C,
    upload_root: &StdPath,
    assets: Vec<entities::asset::Model>,
) -> Result<Vec<AdminAssetRow>, SiteError> {
    if assets.is_empty() {
        return Ok(Vec::new());
    }

    let asset_ids = assets.iter().map(|asset| asset.id).collect::<Vec<_>>();
    let thumbnails = entities::asset_variant::Entity::find()
        .filter(entities::asset_variant::Column::AssetId.is_in(asset_ids))
        .filter(entities::asset_variant::Column::VariantKind.eq("thumbnail"))
        .all(db)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load asset variants: {error}")))?;

    let thumbnails_by_asset = thumbnails
        .into_iter()
        .map(|variant| (variant.asset_id, variant))
        .collect::<HashMap<_, _>>();

    let mut rows = Vec::with_capacity(assets.len());
    for asset in assets {
        let original_exists = fs::try_exists(upload_root.join(&asset.storage_basename))
            .await
            .unwrap_or(false);
        let thumbnail_url = if let Some(variant) = thumbnails_by_asset.get(&asset.id) {
            if fs::try_exists(upload_root.join(&variant.filename))
                .await
                .unwrap_or(false)
            {
                Some(format!("/media/images/{}", variant.filename))
            } else {
                None
            }
        } else {
            None
        };

        let thumbnail_exists = thumbnail_url.is_some();

        rows.push(AdminAssetRow {
            id: asset.id,
            original_filename: asset.original_filename,
            storage_basename: asset.storage_basename.clone(),
            uploader_sub: asset.uploader_sub,
            mime_type: asset.mime_type,
            byte_length: asset.byte_length,
            dimensions: format_asset_dimensions(asset.width, asset.height),
            created_at: asset.created_at.to_rfc3339(),
            original_url: format!("/media/images/{}", asset.storage_basename),
            thumbnail_url,
            original_exists,
            thumbnail_exists,
        });
    }

    Ok(rows)
}

pub(crate) async fn admin_site_assets(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AdminAssetListQuery>,
) -> Result<AdminAssetsTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Viewer).await?;
    let sort_by = AssetSortBy::from_query(query.sort_by.as_deref());
    let sort_dir = AssetSortDirection::from_query(query.sort_dir.as_deref());
    let mut assets = list_assets(state.db.as_ref(), site_id).await?;
    sort_assets(&mut assets, sort_by, sort_dir);
    let site = entities::site::Entity::find_by_id(site_id)
        .one(state.db.as_ref())
        .await
        .map_err(|error| SiteError::internal(format!("failed to load site {site_id}: {error}")))?
        .ok_or(SiteError::SiteNotFound(site_id.to_string()))?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let asset_rows = build_admin_asset_rows(state.db.as_ref(), &state.upload_root, assets).await?;
    let template_shared = AdminTemplateData::new("Assets")
        .with_site_context(&site)
        .with_site_publish_configured(site_publish_configured)
        .with_links(vec![
            AdminLink::new(&format!("/admin/site/{site_id}/assets/new"), "Upload"),
            AdminLink::new(
                &format!("/admin/site/{site_id}/content"),
                "Back to site dashboard",
            ),
        ]);
    let template_shared = if let Some(uploaded_count) = query.uploaded {
        template_shared.with_toast_message(
            &asset_upload_message(uploaded_count),
            &ASSET_UPLOAD_TOAST_QUERY_PARAM,
        )
    } else {
        template_shared
    };

    Ok(AdminAssetsTemplate {
        template_shared,
        site_id,
        site_full_title: site.full_title,
        sort_by_options: sort_by.options(),
        sort_dir_options: sort_dir.options(),
        assets: asset_rows,
    })
}

pub(crate) async fn admin_site_assets_new(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AdminAssetListQuery>,
) -> Result<AdminAssetsNewTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let sort_by = AssetSortBy::from_query(query.sort_by.as_deref());
    let sort_dir = AssetSortDirection::from_query(query.sort_dir.as_deref());
    match get_by_id(state.db.as_ref(), site_id).await {
        Ok(site) => {
            let mut assets = list_assets(state.db.as_ref(), site_id).await?;
            sort_assets(&mut assets, sort_by, sort_dir);
            let assets =
                build_admin_asset_rows(state.db.as_ref(), &state.upload_root, assets).await?;
            let site_publish_configured =
                site_has_publish_config(state.db.as_ref(), site_id).await?;

            Ok(AdminAssetsNewTemplate {
                template_shared: AdminTemplateData::new("Upload Assets")
                    .with_site_context(&site)
                    .with_site_publish_configured(site_publish_configured)
                    .with_links(vec![
                        AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Back to assets"),
                        AdminLink::new(
                            &format!("/admin/site/{site_id}/content"),
                            "Back to site dashboard",
                        ),
                    ]),
                site_id: site.id,
                site_full_title: site.full_title,
                sort_by_options: sort_by.options(),
                sort_dir_options: sort_dir.options(),
                assets,
            })
        }
        Err(error) => Err(SiteError::internal(format!(
            "failed to load site {site_id}: {error}"
        ))),
    }
}

pub(crate) async fn admin_site_assets_create(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let site = match get_by_id(state.db.as_ref(), site_id).await {
        Ok(site) => site,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to load site {site_id}: {error}"
            )));
        }
    };

    let parsed_request = parse_asset_create_request(multipart).await?;
    let uploads = resolve_asset_create_request(state.oidc_client.as_ref(), parsed_request).await?;
    let uploaded_count = uploads.len();

    let db_txn = state.db.begin().await?;
    let batch = create_uploaded_asset_batch(
        &db_txn,
        &state.upload_root,
        site.id,
        &actor.subject,
        uploads,
    )
    .await?;
    log_asset_create_events(&db_txn, &actor.subject, "create_asset", &batch.assets).await?;

    if let Err(error) = db_txn.commit().await {
        crate::cleanup_uploaded_asset_files(&state.upload_root, &batch.stored_filenames)
            .await
            .map_err(|cleanup_error| {
                SiteError::internal(format!(
                    "failed to clean up uploaded files after commit error: {cleanup_error}"
                ))
            })?;
        return Err(SiteError::internal(format!(
            "failed to commit asset transaction: {error}"
        )));
    }

    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/assets?{ASSET_UPLOAD_TOAST_QUERY_PARAM}={uploaded_count}"
    )))
}

pub(crate) async fn remove_file_if_exists(path: &StdPath) -> Result<(), SiteError> {
    match fs::remove_file(path).await {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SiteError::from(error)),
    }
}

pub(crate) async fn replace_asset_variant<C: ConnectionTrait>(
    db: &C,
    input: NewAssetVariant,
) -> Result<entities::asset_variant::Model, SiteError> {
    let existing = entities::asset_variant::Entity::find()
        .filter(entities::asset_variant::Column::AssetId.eq(input.asset_id))
        .filter(entities::asset_variant::Column::VariantKind.eq(&input.variant_kind))
        .one(db)
        .await
        .map_err(SiteError::from)?;

    if let Some(existing) = existing {
        let mut active = existing.into_active_model();
        active.filename = Set(input.filename);
        active.mime_type = Set(input.mime_type);
        active.byte_length = Set(input.byte_length);
        active.width = Set(input.width);
        active.height = Set(input.height);
        active.update(db).await.map_err(SiteError::from)
    } else {
        crate::create_asset_variant(db, input)
            .await
            .map_err(SiteError::internal)
    }
}

pub(crate) async fn replace_asset_files<C: ConnectionTrait>(
    db: &C,
    upload_root: &StdPath,
    site_id: Uuid,
    asset_id: Uuid,
    bytes: Vec<u8>,
    original_filename: String,
    mime_type: Option<String>,
) -> Result<entities::asset::Model, SiteError> {
    let existing_asset = entities::asset::Entity::find_by_id(asset_id)
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .one(db)
        .await
        .map_err(SiteError::from)?
        .ok_or(SiteError::NotFound)?;

    let existing_variants = crate::list_asset_variants(db, asset_id)
        .await
        .map_err(SiteError::internal)?;
    let existing_thumbnail_filename = existing_variants
        .iter()
        .find(|variant| variant.variant_kind == "thumbnail")
        .map(|variant| variant.filename.clone());

    let file_details = persist_asset_files(
        upload_root,
        &existing_asset.storage_basename,
        bytes,
        &original_filename,
        mime_type,
    )
    .await?;

    let mut asset = existing_asset.clone().into_active_model();
    asset.original_filename = Set(original_filename);
    asset.mime_type = Set(file_details.mime_type.clone());
    asset.byte_length = Set(file_details.byte_length);
    asset.width = Set(file_details.width);
    asset.height = Set(file_details.height);
    let updated_asset = asset.update(db).await.map_err(SiteError::from)?;

    replace_asset_variant(
        db,
        NewAssetVariant {
            asset_id,
            variant_kind: "original".to_string(),
            filename: existing_asset.storage_basename.clone(),
            mime_type: file_details.mime_type.clone(),
            byte_length: file_details.byte_length,
            width: file_details.width,
            height: file_details.height,
        },
    )
    .await?;

    match (
        existing_thumbnail_filename,
        file_details.thumbnail_filename.clone(),
    ) {
        (Some(existing_thumbnail), Some(new_thumbnail)) => {
            replace_asset_variant(
                db,
                NewAssetVariant {
                    asset_id,
                    variant_kind: "thumbnail".to_string(),
                    filename: new_thumbnail.clone(),
                    mime_type: file_details.mime_type.clone(),
                    byte_length: file_details.byte_length,
                    width: file_details.width,
                    height: file_details.height,
                },
            )
            .await?;

            if existing_thumbnail != new_thumbnail {
                remove_file_if_exists(&upload_root.join(existing_thumbnail)).await?;
            }
        }
        (None, Some(new_thumbnail)) => {
            replace_asset_variant(
                db,
                NewAssetVariant {
                    asset_id,
                    variant_kind: "thumbnail".to_string(),
                    filename: new_thumbnail,
                    mime_type: file_details.mime_type.clone(),
                    byte_length: file_details.byte_length,
                    width: file_details.width,
                    height: file_details.height,
                },
            )
            .await?;
        }
        (Some(existing_thumbnail), None) => {
            if let Some(existing_variant) = existing_variants
                .iter()
                .find(|variant| variant.variant_kind == "thumbnail")
            {
                entities::asset_variant::Entity::delete_by_id(existing_variant.id)
                    .exec(db)
                    .await
                    .map_err(SiteError::from)?;
            }
            remove_file_if_exists(&upload_root.join(existing_thumbnail)).await?;
        }
        (None, None) => {}
    }

    Ok(updated_asset)
}

pub(crate) async fn admin_site_asset_replace(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, asset_id)): Path<(Uuid, Uuid)>,
) -> Result<AdminAssetReplaceTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let asset = get_asset_for_site(state.db.as_ref(), site_id, asset_id)
        .await?
        .ok_or(SiteError::NotFound)?;
    let asset_row =
        build_admin_asset_rows(state.db.as_ref(), &state.upload_root, vec![asset.clone()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| SiteError::internal("missing asset row".to_string()))?;

    Ok(AdminAssetReplaceTemplate {
        template_shared: AdminTemplateData::new("Replace Asset")
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured)
            .with_links(vec![
                AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Back to assets"),
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
            ]),
        site_id: site.id,
        site_full_title: site.full_title,
        asset: asset_row,
    })
}

pub(crate) async fn admin_site_asset_replace_update(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, asset_id)): Path<(Uuid, Uuid)>,
    multipart: Multipart,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let (upload_bytes, source_url) = parse_asset_upload(multipart).await?;
    let (bytes, original_filename, mime_type) =
        resolve_asset_upload(state.oidc_client.as_ref(), upload_bytes, source_url).await?;

    let txn = state.db.begin().await?;
    let asset = replace_asset_files(
        &txn,
        &state.upload_root,
        site_id,
        asset_id,
        bytes,
        original_filename,
        mime_type,
    )
    .await?;

    log_audit_event(
        &txn,
        &actor.subject,
        "replace_asset",
        "asset",
        &asset.id.to_string(),
        Some(asset.site_id),
        Some(json!({
            "original_filename": &asset.original_filename,
            "storage_basename": &asset.storage_basename,
            "mime_type": &asset.mime_type
        })),
    )
    .await
    .map_err(|error| SiteError::internal(format!("failed to log asset audit: {error}")))?;

    txn.commit().await?;

    Ok(Redirect::to(&format!("/admin/site/{site_id}/assets")))
}

pub(crate) fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}
