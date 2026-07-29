use super::state::*;
use super::*;
use crate::collect_asset_filenames;
use crate::mass_asset_import::{
    LocalAssetCandidate, LocalAssetCandidateRank, MissingAssetGroup,
    find_exact_local_asset_candidates_for_paths, find_local_asset_candidates,
    find_local_asset_candidates_for_paths, find_missing_asset_groups, validate_import_candidate,
};
use sea_orm::QuerySelect;
use sea_orm::sea_query::{Expr, Func};

const MASS_IMPORT_IMAGE_EXTENSIONS: [&str; 6] = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".svg"];

fn multipart_site_error(
    action: &str,
    error: &axum::extract::multipart::MultipartError,
) -> SiteError {
    let details = error.body_text();
    match error.status() {
        StatusCode::BAD_REQUEST => {
            SiteError::BadRequest(format!("failed to parse {action}: {details}"))
        }
        StatusCode::PAYLOAD_TOO_LARGE => {
            SiteError::PayloadTooLarge(format!("{action} exceeded the 50 MB upload limit"))
        }
        _ => SiteError::internal(format!("failed to parse {action}: {details}")),
    }
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

#[derive(Clone, Debug)]
pub(crate) struct UploadedAssetFile {
    pub(crate) bytes: Vec<u8>,
    pub(crate) original_filename: String,
    pub(crate) mime_type: Option<String>,
}

pub(crate) struct AssetUploadAuditContext<'a> {
    pub(crate) upload_root: &'a StdPath,
    pub(crate) site_id: Uuid,
    pub(crate) actor_sub: &'a str,
    pub(crate) event_type: &'a str,
}

pub(crate) enum ParsedAssetCreateUpload {
    Files(Vec<UploadedAssetFile>),
    SourceUrl(Url),
}

pub(crate) async fn parse_asset_create_upload(
    mut multipart: Multipart,
) -> Result<ParsedAssetCreateUpload, SiteError> {
    let mut uploaded_files = Vec::new();
    let mut source_url: Option<Url> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(field) => field,
            Err(error) => return Err(multipart_site_error("asset upload", &error)),
        };

        let Some(field) = field else { break };
        match field.name() {
            Some("file") => {
                let original_filename = field
                    .file_name()
                    .map(|value| value.to_string())
                    .ok_or_else(|| {
                        SiteError::BadRequest(
                            "uploaded file is missing its original filename".to_string(),
                        )
                    })?;
                let mime_type = field.content_type().map(|value| value.to_string());
                let bytes = match field.bytes().await {
                    Ok(bytes) => bytes,
                    Err(error) => return Err(multipart_site_error("asset upload", &error)),
                };
                if bytes.is_empty() {
                    continue;
                }

                uploaded_files.push(UploadedAssetFile {
                    bytes: bytes.to_vec(),
                    original_filename,
                    mime_type,
                });
            }
            Some("source_url") => {
                let value = match field.text().await {
                    Ok(value) => value,
                    Err(error) => return Err(multipart_site_error("asset import url", &error)),
                };
                let parsed = normalize_remote_asset_url(&value)?;
                if let Some(parsed) = parsed {
                    if source_url.is_some() {
                        return Err(SiteError::BadRequest(
                            "provide only one image url per upload".to_string(),
                        ));
                    }
                    source_url = Some(parsed);
                }
            }
            _ => continue,
        }
    }

    if !uploaded_files.is_empty() && source_url.is_some() {
        return Err(SiteError::BadRequest(
            "provide uploaded files or an image url, not both".to_string(),
        ));
    }

    if !uploaded_files.is_empty() {
        return Ok(ParsedAssetCreateUpload::Files(uploaded_files));
    }

    if let Some(source_url) = source_url {
        return Ok(ParsedAssetCreateUpload::SourceUrl(source_url));
    }

    Err(SiteError::BadRequest(
        "provide at least one uploaded file or an image url".to_string(),
    ))
}

pub(crate) async fn cleanup_uploaded_files(
    upload_root: &StdPath,
    filenames: &HashSet<String>,
) -> Result<(), SiteError> {
    for filename in filenames {
        remove_file_if_exists(&upload_root.join(filename)).await?;
    }

    Ok(())
}

pub(crate) async fn store_uploaded_asset_with_audit<C: ConnectionTrait>(
    db: &C,
    context: AssetUploadAuditContext<'_>,
    upload: UploadedAssetFile,
    cleanup_filenames: &mut HashSet<String>,
) -> Result<entities::asset::Model, SiteError> {
    let asset = store_uploaded_asset(
        db,
        context.upload_root,
        context.site_id,
        context.actor_sub,
        upload.bytes,
        upload.original_filename,
        upload.mime_type,
    )
    .await?;

    cleanup_filenames.insert(asset.storage_basename.clone());
    cleanup_filenames.extend(collect_asset_filenames(db, asset.id).await?);

    log_audit_event(
        db,
        context.actor_sub,
        context.event_type,
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

    Ok(asset)
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
            Err(error) => return Err(multipart_site_error("asset upload", &error)),
        };

        let Some(field) = field else { break };
        match field.name() {
            Some("file") => {
                let field_filename = field.file_name().map(|value| value.to_string());
                let field_mime_type = field.content_type().map(|value| value.to_string());
                let bytes = match field.bytes().await {
                    Ok(bytes) => bytes,
                    Err(error) => return Err(multipart_site_error("asset upload", &error)),
                };
                if bytes.is_empty() {
                    continue;
                }

                upload_bytes = Some((bytes.to_vec(), field_filename, field_mime_type));
            }
            Some("source_url") => {
                let value = match field.text().await {
                    Ok(value) => value,
                    Err(error) => return Err(multipart_site_error("asset import url", &error)),
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

    Ok(AdminAssetsTemplate {
        template_shared: AdminTemplateData::new("Assets")
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured)
            .with_links(vec![
                AdminLink::new(&format!("/admin/site/{site_id}/assets/new"), "Upload"),
                AdminLink::new(
                    &format!("/admin/site/{site_id}/assets/mass-import"),
                    "Mass import",
                ),
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content"),
                    "Back to site dashboard",
                ),
            ]),
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
                template_shared: AdminTemplateData::new("Upload Asset")
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
    let uploaded_files = match parse_asset_create_upload(multipart).await? {
        ParsedAssetCreateUpload::Files(uploaded_files) => uploaded_files,
        ParsedAssetCreateUpload::SourceUrl(source_url) => {
            let (bytes, original_filename, mime_type) =
                fetch_remote_asset(state.oidc_client.as_ref(), source_url).await?;
            vec![UploadedAssetFile {
                bytes,
                original_filename,
                mime_type,
            }]
        }
    };
    let db_txn = state.db.begin().await?;
    let mut cleanup_filenames = HashSet::new();
    let upload_result: Result<(), SiteError> = async {
        for uploaded_file in uploaded_files {
            store_uploaded_asset_with_audit(
                &db_txn,
                AssetUploadAuditContext {
                    upload_root: &state.upload_root,
                    site_id: site.id,
                    actor_sub: &actor.subject,
                    event_type: "create_asset",
                },
                uploaded_file,
                &mut cleanup_filenames,
            )
            .await?;
        }

        db_txn.commit().await.map_err(|error| {
            SiteError::internal(format!("failed to commit asset transaction: {error}"))
        })?;

        Ok(())
    }
    .await;

    if let Err(error) = upload_result {
        if !cleanup_filenames.is_empty() {
            cleanup_uploaded_files(&state.upload_root, &cleanup_filenames)
                .await
                .map_err(|cleanup_error| {
                    SiteError::internal(format!(
                        "asset upload failed: {error}; cleanup failed: {cleanup_error}"
                    ))
                })?;
        }
        return Err(error);
    }

    Ok(Redirect::to(&format!("/admin/site/{site_id}/assets")))
}

pub(crate) async fn admin_site_assets_mass_import(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AdminMassAssetImportQuery>,
) -> Result<AdminMassAssetImportTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let scan_limit = query.limit.unwrap_or(20).clamp(1, 100);
    let import_path = site.mass_import_assets.clone().unwrap_or_default();
    let mut message = None;
    let mut warnings = Vec::new();
    let mut page_groups = Vec::<AdminMassAssetImportPageGroup>::new();
    let import_root = if import_path.trim().is_empty() {
        warnings.push("Mass import assets path is not configured.".to_string());
        None
    } else {
        match fs::metadata(&import_path).await {
            Ok(metadata) if metadata.is_dir() => Some(PathBuf::from(&import_path)),
            Ok(_) => {
                warnings
                    .push("The configured mass import assets path is not a directory.".to_string());
                None
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                warnings
                    .push("The configured mass import assets path could not be found.".to_string());
                None
            }
            Err(error) => {
                warnings.push(format!(
                    "The configured mass import assets path could not be read: {error}."
                ));
                None
            }
        }
    };

    if site.internal_domains.is_empty() {
        warnings.push("Internal domains are not configured.".to_string());
    }

    if import_path.trim().is_empty() {
        message = Some("Configure a mass import assets path in site settings first.".to_string());
    } else {
        let groups = load_mass_import_listing_groups(
            state.db.as_ref(),
            site_id,
            &site.internal_domains,
            scan_limit,
        )
        .await?;
        let mut candidate_map = if let Some(import_root) = import_root.as_ref()
            && !groups.is_empty()
        {
            let normalized_paths = groups
                .iter()
                .map(|group| group.normalized_path.clone())
                .collect::<Vec<_>>();
            find_exact_local_asset_candidates_for_paths(import_root.as_path(), &normalized_paths, 5)
                .await
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        for group in groups {
            let first_content_title = group
                .affected_content
                .first()
                .map(|row| row.title.clone())
                .unwrap_or_else(|| "Unknown content".to_string());
            let first_content_href = group
                .affected_content
                .first()
                .map(|row| {
                    format!(
                        "/admin/site/{site_id}/assets/mass-import/content/{}",
                        row.content_id
                    )
                })
                .unwrap_or_else(|| format!("/admin/site/{site_id}/assets/mass-import"));
            let candidates = candidate_map
                .remove(&group.normalized_path)
                .unwrap_or_default();
            let row = AdminMassAssetImportRow {
                import_href: format!(
                    "/admin/site/{site_id}/assets/mass-import/missing?path={}",
                    encode_query_value(&group.normalized_path)
                ),
                normalized_path: group.normalized_path,
                affected_post_count: group.affected_content.len(),
                occurrence_count: group.occurrence_count,
                candidate_paths: candidates
                    .into_iter()
                    .map(|candidate| candidate.relative_path.to_string_lossy().to_string())
                    .collect(),
            };
            if let Some(page_group) = page_groups
                .last_mut()
                .filter(|page_group| page_group.first_content_href == first_content_href)
            {
                page_group.rows.push(row);
            } else {
                page_groups.push(AdminMassAssetImportPageGroup {
                    first_content_title,
                    first_content_href,
                    rows: vec![row],
                });
            }
        }
    }

    Ok(AdminMassAssetImportTemplate {
        template_shared: AdminTemplateData::new("Mass Asset Import")
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured)
            .with_links(vec![
                AdminLink::new(&format!("/admin/site/{site_id}/settings"), "Site settings"),
                AdminLink::new(&format!("/admin/site/{site_id}/assets"), "Assets"),
            ]),
        site_id,
        import_path,
        scan_limit,
        page_groups,
        message,
        warnings,
    })
}

pub(crate) async fn admin_site_assets_mass_import_missing(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<MissingImageImportQuery>,
) -> Result<AdminMissingImageImportTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let import_root = configured_import_root(&site)?;
    let content =
        load_mass_import_content_for_path(state.db.as_ref(), site_id, &query.path).await?;
    let group = find_group_for_path(content, &site.internal_domains, &query.path);
    let affected_content = group
        .as_ref()
        .map(|group| {
            group
                .affected_content
                .iter()
                .map(|row| AdminMassAssetAffectedContentRow {
                    title: row.title.clone(),
                    occurrence_count: row.occurrence_count,
                    edit_href: format!("/admin/site/{site_id}/content/{}/edit", row.content_id),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let candidates = find_local_asset_candidates(import_root.as_path(), &query.path, 20)
        .await
        .unwrap_or_default();
    let candidate_rows = build_candidate_rows(site_id, candidates).await;
    let message = if group.is_none() {
        Some("This missing asset path no longer appears in current content.".to_string())
    } else if candidate_rows.is_empty() {
        Some("No local file candidates were found for this path.".to_string())
    } else {
        None
    };

    Ok(AdminMissingImageImportTemplate {
        template_shared: AdminTemplateData::new("Missing Image Import")
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured)
            .with_links(vec![AdminLink::new(
                &format!("/admin/site/{site_id}/assets/mass-import"),
                "Mass asset import",
            )]),
        site_id,
        normalized_path: query.path,
        affected_content,
        candidates: candidate_rows,
        message,
    })
}

pub(crate) async fn admin_site_assets_mass_import_content(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<AdminMassAssetImportContentTemplate, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let site_publish_configured = site_has_publish_config(state.db.as_ref(), site_id).await?;
    let import_root = configured_import_root(&site)?;
    let content = get_content_for_site(state.db.as_ref(), site_id, content_id)
        .await?
        .ok_or(SiteError::NotFound)?;
    let rows = build_mass_import_content_rows(
        state.db.as_ref(),
        site_id,
        &site.internal_domains,
        import_root.as_path(),
        &content,
    )
    .await?;
    let message = if rows.is_empty() {
        Some("No missing asset paths were found in this content item.".to_string())
    } else {
        None
    };

    Ok(AdminMassAssetImportContentTemplate {
        template_shared: AdminTemplateData::new("Mass Asset Import")
            .with_site_context(&site)
            .with_site_publish_configured(site_publish_configured)
            .with_links(vec![
                AdminLink::new(
                    &format!("/admin/site/{site_id}/assets/mass-import"),
                    "Mass asset import",
                ),
                AdminLink::new(
                    &format!("/admin/site/{site_id}/content/{content_id}/edit"),
                    "Open editor",
                ),
            ]),
        site_id,
        content_id,
        content_title: content.title,
        edit_href: format!("/admin/site/{site_id}/content/{content_id}/edit"),
        rows,
        message,
    })
}

pub(crate) async fn admin_site_assets_mass_import_preview(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<MissingImagePreviewQuery>,
) -> Result<Response, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let import_root = configured_import_root(&site)?;
    let candidate =
        validate_import_candidate(import_root.as_path(), &import_root.join(&query.candidate))
            .await?;
    let bytes = fs::read(&candidate).await.map_err(SiteError::from)?;
    let mime_type = mime_guess::from_path(&candidate)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string();
    Ok(([(header::CONTENT_TYPE, mime_type)], bytes).into_response())
}

pub(crate) async fn admin_site_assets_mass_import_content_apply(
    State(state): State<AdminState>,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
    RawForm(raw_form): RawForm,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let import_root = configured_import_root(&site)?;
    get_content_for_site(state.db.as_ref(), site_id, content_id)
        .await?
        .ok_or(SiteError::NotFound)?;
    let form = parse_mass_import_content_apply_form(&raw_form)?;
    apply_mass_import_selections(
        &state,
        site_id,
        Some(content_id),
        &site,
        import_root.as_path(),
        &actor.subject,
        &form.selections,
    )
    .await?;

    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/assets/mass-import/content/{content_id}"
    )))
}

pub(crate) async fn admin_site_assets_mass_import_apply(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Form(form): Form<MissingImageImportForm>,
) -> Result<Redirect, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let actor = current_user(&session).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let import_root = configured_import_root(&site)?;
    let candidate_path =
        validate_import_candidate(import_root.as_path(), &import_root.join(&form.candidate))
            .await?;
    let bytes = fs::read(&candidate_path).await.map_err(SiteError::from)?;
    let original_filename = candidate_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .ok_or_else(|| SiteError::BadRequest("candidate is missing a filename".to_string()))?;
    let mime_type = mime_guess::from_path(&candidate_path)
        .first_raw()
        .map(|value| value.to_string());
    let content =
        load_mass_import_content_for_path(state.db.as_ref(), site_id, &form.normalized_path)
            .await?;
    let group = find_group_for_path(content, &site.internal_domains, &form.normalized_path)
        .ok_or_else(|| SiteError::BadRequest("missing asset path no longer appears".to_string()))?;
    let txn = state.db.begin().await?;
    let mut cleanup_filenames = HashSet::new();
    let result: Result<Uuid, SiteError> = async {
        let asset = store_uploaded_asset_with_audit(
            &txn,
            AssetUploadAuditContext {
                upload_root: &state.upload_root,
                site_id,
                actor_sub: &actor.subject,
                event_type: "mass_import_asset",
            },
            UploadedAssetFile {
                bytes,
                original_filename,
                mime_type,
            },
            &mut cleanup_filenames,
        )
        .await?;
        let shortcode = format_asset_shortcode(
            asset.id,
            "original",
            &asset_alt_text(&form.normalized_path),
            None,
        );
        apply_group_replacements(&txn, site_id, &actor.subject, &group, &shortcode).await?;
        let refreshed =
            load_mass_import_content_for_path(&txn, site_id, &form.normalized_path).await?;
        if find_group_for_path(refreshed, &site.internal_domains, &form.normalized_path).is_some() {
            return Err(SiteError::internal(
                "mass asset import did not clear the missing path".to_string(),
            ));
        }
        log_audit_event(
            &txn,
            &actor.subject,
            "mass_asset_import_apply",
            "content_item",
            &site_id.to_string(),
            Some(site_id),
            Some(json!({
                "normalized_path": form.normalized_path,
                "asset_id": asset.id,
                "affected_content_count": group.affected_content.len(),
                "occurrence_count": group.occurrence_count,
            })),
        )
        .await
        .map_err(|error| {
            SiteError::internal(format!("failed to log mass import audit: {error}"))
        })?;
        Ok(asset.id)
    }
    .await;

    if let Err(error) = result {
        cleanup_uploaded_files(&state.upload_root, &cleanup_filenames).await?;
        return Err(error);
    }
    txn.commit().await?;

    Ok(Redirect::to(&format!(
        "/admin/site/{site_id}/assets/mass-import"
    )))
}

pub(crate) async fn admin_site_assets_mass_import_recheck(
    State(state): State<AdminState>,
    session: Session,
    Path(site_id): Path<Uuid>,
    Json(payload): Json<MassAssetRecheckRequest>,
) -> Result<Json<MassAssetRecheckResponse>, SiteError> {
    require_site_role(&state, &session, site_id, SiteRole::Author).await?;
    let site = get_by_id(state.db.as_ref(), site_id).await?;
    let content = if let Some(content_id) = payload.content_id {
        vec![
            get_content_for_site(state.db.as_ref(), site_id, content_id)
                .await?
                .ok_or(SiteError::NotFound)?,
        ]
    } else {
        load_mass_import_content_for_path(state.db.as_ref(), site_id, &payload.path).await?
    };
    let group = find_group_for_path(content, &site.internal_domains, &payload.path);
    let occurrence_count = group
        .as_ref()
        .map(|group| group.occurrence_count)
        .unwrap_or(0);
    Ok(Json(MassAssetRecheckResponse {
        complete: occurrence_count == 0,
        occurrence_count,
    }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MassImportContentApplySelection {
    normalized_path: String,
    candidate: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MassImportContentApplyForm {
    selections: Vec<MassImportContentApplySelection>,
}

fn parse_mass_import_content_apply_form(
    raw: &[u8],
) -> Result<MassImportContentApplyForm, SiteError> {
    let values = collect_form_values(raw);
    let raw_selections = if let Some(single_selection) =
        first_form_value(&values, "single_selection").filter(|value| !value.trim().is_empty())
    {
        vec![single_selection]
    } else {
        values
            .iter()
            .filter(|(key, _)| key.starts_with("selection_"))
            .flat_map(|(_, values)| values.iter())
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
    };

    if raw_selections.is_empty() {
        return Err(SiteError::BadRequest(
            "select at least one import candidate".to_string(),
        ));
    }

    let mut selections = Vec::with_capacity(raw_selections.len());
    let mut seen_paths = HashSet::new();
    for raw_selection in raw_selections {
        let selection = parse_mass_import_selection(&raw_selection)?;
        if !seen_paths.insert(selection.normalized_path.clone()) {
            return Err(SiteError::BadRequest(
                "duplicate import selection for the same missing path".to_string(),
            ));
        }
        selections.push(selection);
    }
    Ok(MassImportContentApplyForm { selections })
}

fn parse_mass_import_selection(raw: &str) -> Result<MassImportContentApplySelection, SiteError> {
    let values = collect_form_values(raw.as_bytes());
    let normalized_path = first_form_value(&values, "normalized_path")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SiteError::BadRequest("missing normalized path selection".to_string()))?;
    let candidate = first_form_value(&values, "candidate")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SiteError::BadRequest("missing candidate selection".to_string()))?;
    Ok(MassImportContentApplySelection {
        normalized_path,
        candidate,
    })
}

fn encode_mass_import_selection(normalized_path: &str, candidate: &str) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("normalized_path", normalized_path);
    serializer.append_pair("candidate", candidate);
    serializer.finish()
}

async fn build_mass_import_content_rows<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    internal_domains: &[String],
    import_root: &StdPath,
    content: &entities::content_item::Model,
) -> Result<Vec<AdminMassAssetImportContentPathRow>, SiteError> {
    let post_groups =
        find_missing_asset_groups(vec![content.clone()], internal_domains, usize::MAX);
    if post_groups.is_empty() {
        return Ok(Vec::new());
    }
    let normalized_paths = post_groups
        .iter()
        .map(|group| group.normalized_path.clone())
        .collect::<Vec<_>>();
    let mut candidate_map =
        find_local_asset_candidates_for_paths(import_root, &normalized_paths, 20)
            .await
            .unwrap_or_default();
    let mut rows = Vec::with_capacity(post_groups.len());
    for post_group in post_groups {
        let site_content =
            load_mass_import_content_for_path(db, site_id, &post_group.normalized_path).await?;
        let site_group =
            find_group_for_path(site_content, internal_domains, &post_group.normalized_path)
                .unwrap_or_else(|| post_group.clone());
        let candidates = candidate_map
            .remove(&post_group.normalized_path)
            .unwrap_or_default();
        rows.push(AdminMassAssetImportContentPathRow {
            normalized_path: post_group.normalized_path.clone(),
            post_occurrence_count: post_group.occurrence_count,
            affected_post_count: site_group.affected_content.len(),
            site_occurrence_count: site_group.occurrence_count,
            candidates: build_content_candidate_rows(
                site_id,
                &post_group.normalized_path,
                candidates,
            )
            .await,
        });
    }
    Ok(rows)
}

async fn build_content_candidate_rows(
    site_id: Uuid,
    normalized_path: &str,
    candidates: Vec<LocalAssetCandidate>,
) -> Vec<AdminMassAssetImportContentCandidateRow> {
    let mut rows = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let relative_path = candidate.relative_path.to_string_lossy().to_string();
        rows.push(AdminMassAssetImportContentCandidateRow {
            preview_url: format!(
                "/admin/site/{site_id}/assets/mass-import/preview?candidate={}",
                encode_query_value(&relative_path)
            ),
            byte_length: format_byte_length(candidate.byte_length),
            dimensions: candidate_dimensions(candidate.absolute_path.clone())
                .await
                .unwrap_or_else(|| "n/a".to_string()),
            rank_label: match candidate.rank {
                LocalAssetCandidateRank::PathSuffix => "Path match".to_string(),
                LocalAssetCandidateRank::Filename => "Filename match".to_string(),
            },
            selection_value: encode_mass_import_selection(normalized_path, &relative_path),
            relative_path,
        });
    }
    rows
}

async fn apply_mass_import_selections(
    state: &AdminState,
    site_id: Uuid,
    source_content_id: Option<Uuid>,
    site: &entities::site::Model,
    import_root: &StdPath,
    actor_sub: &str,
    selections: &[MassImportContentApplySelection],
) -> Result<(), SiteError> {
    let txn = state.db.begin().await?;
    let mut cleanup_filenames = HashSet::new();
    let result: Result<(), SiteError> = async {
        for selection in selections {
            let candidate_path =
                validate_import_candidate(import_root, &import_root.join(&selection.candidate))
                    .await?;
            let bytes = fs::read(&candidate_path).await.map_err(SiteError::from)?;
            let original_filename = candidate_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_string())
                .ok_or_else(|| {
                    SiteError::BadRequest("candidate is missing a filename".to_string())
                })?;
            let mime_type = mime_guess::from_path(&candidate_path)
                .first_raw()
                .map(|value| value.to_string());
            let content =
                load_mass_import_content_for_path(&txn, site_id, &selection.normalized_path)
                    .await?;
            let group =
                find_group_for_path(content, &site.internal_domains, &selection.normalized_path)
                    .ok_or_else(|| {
                        SiteError::BadRequest("missing asset path no longer appears".to_string())
                    })?;
            if let Some(source_content_id) = source_content_id
                && !group
                    .affected_content
                    .iter()
                    .any(|row| row.content_id == source_content_id)
            {
                return Err(SiteError::BadRequest(
                    "missing asset path no longer appears in this content item".to_string(),
                ));
            }
            let asset = store_uploaded_asset_with_audit(
                &txn,
                AssetUploadAuditContext {
                    upload_root: &state.upload_root,
                    site_id,
                    actor_sub,
                    event_type: "mass_import_asset",
                },
                UploadedAssetFile {
                    bytes,
                    original_filename,
                    mime_type,
                },
                &mut cleanup_filenames,
            )
            .await?;
            let shortcode = format_asset_shortcode(
                asset.id,
                "original",
                &asset_alt_text(&selection.normalized_path),
                None,
            );
            apply_group_replacements(&txn, site_id, actor_sub, &group, &shortcode).await?;
            let refreshed =
                load_mass_import_content_for_path(&txn, site_id, &selection.normalized_path)
                    .await?;
            if find_group_for_path(
                refreshed,
                &site.internal_domains,
                &selection.normalized_path,
            )
            .is_some()
            {
                return Err(SiteError::internal(
                    "mass asset import did not clear the missing path".to_string(),
                ));
            }
            log_audit_event(
                &txn,
                actor_sub,
                "mass_asset_import_content_apply",
                "content_item",
                &source_content_id.unwrap_or(site_id).to_string(),
                Some(site_id),
                Some(json!({
                    "source_content_id": source_content_id,
                    "normalized_path": selection.normalized_path,
                    "asset_id": asset.id,
                    "affected_content_count": group.affected_content.len(),
                    "occurrence_count": group.occurrence_count,
                })),
            )
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to log mass import audit: {error}"))
            })?;
        }
        Ok(())
    }
    .await;

    if let Err(error) = result {
        cleanup_uploaded_files(&state.upload_root, &cleanup_filenames).await?;
        return Err(error);
    }
    txn.commit().await?;
    Ok(())
}

fn configured_import_root(site: &entities::site::Model) -> Result<PathBuf, SiteError> {
    let Some(import_path) = site.mass_import_assets.as_ref() else {
        return Err(SiteError::BadRequest(
            "mass import assets path is not configured".to_string(),
        ));
    };
    if import_path.trim().is_empty() {
        return Err(SiteError::BadRequest(
            "mass import assets path is not configured".to_string(),
        ));
    }
    Ok(PathBuf::from(import_path))
}

fn find_group_for_path(
    content: Vec<entities::content_item::Model>,
    internal_domains: &[String],
    normalized_path: &str,
) -> Option<MissingAssetGroup> {
    find_missing_asset_groups(content, internal_domains, usize::MAX)
        .into_iter()
        .find(|group| group.normalized_path == normalized_path)
}

async fn load_mass_import_listing_groups<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    internal_domains: &[String],
    scan_limit: usize,
) -> Result<Vec<MissingAssetGroup>, SiteError> {
    let batch_size = scan_limit.saturating_mul(25).clamp(100, 1000);
    let mut offset = 0u64;
    let mut discovered_paths = Vec::<String>::new();
    let mut discovered_path_set = HashSet::<String>::new();

    while discovered_paths.len() < scan_limit {
        let content = load_mass_import_candidate_content(db, site_id, batch_size, offset).await?;
        if content.is_empty() {
            break;
        }
        let groups = find_missing_asset_groups(content, internal_domains, scan_limit);
        for group in groups {
            if discovered_path_set.insert(group.normalized_path.clone()) {
                discovered_paths.push(group.normalized_path);
                if discovered_paths.len() >= scan_limit {
                    break;
                }
            }
        }
        offset = offset.saturating_add(batch_size as u64);
    }

    let mut groups = Vec::with_capacity(discovered_paths.len());
    for normalized_path in discovered_paths {
        let content = load_mass_import_content_for_path(db, site_id, &normalized_path).await?;
        if let Some(group) = find_group_for_path(content, internal_domains, &normalized_path) {
            groups.push(group);
        }
    }
    Ok(groups)
}

async fn load_mass_import_content_for_path<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    normalized_path: &str,
) -> Result<Vec<entities::content_item::Model>, SiteError> {
    let terms = mass_import_target_path_terms(normalized_path);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let mut condition = Condition::any();
    for term in terms {
        condition = condition.add(entities::content_item::Column::PageContent.contains(&term));
    }

    entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .filter(condition)
        .order_by_desc(Expr::cust("COALESCE(last_updated, created_at)"))
        .order_by_desc(entities::content_item::Column::CreatedAt)
        .order_by_asc(entities::content_item::Column::Title)
        .all(db)
        .await
        .map_err(SiteError::from)
}

async fn load_mass_import_candidate_content<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    limit: usize,
    offset: u64,
) -> Result<Vec<entities::content_item::Model>, SiteError> {
    let mut condition = Condition::any();
    for extension in MASS_IMPORT_IMAGE_EXTENSIONS {
        condition = condition.add(
            Expr::expr(Func::lower(Expr::col(
                entities::content_item::Column::PageContent,
            )))
            .like(format!("%{extension}%")),
        );
    }

    entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .filter(condition)
        .order_by_desc(Expr::cust("COALESCE(last_updated, created_at)"))
        .order_by_desc(entities::content_item::Column::CreatedAt)
        .order_by_asc(entities::content_item::Column::Title)
        .limit(limit as u64)
        .offset(offset)
        .all(db)
        .await
        .map_err(SiteError::from)
}

fn mass_import_target_path_terms(normalized_path: &str) -> Vec<String> {
    let trimmed = normalized_path.trim();
    let relative = trimmed.trim_start_matches('/');
    if relative.is_empty() {
        return Vec::new();
    }
    vec![format!("/{relative}"), relative.to_string()]
}

async fn build_candidate_rows(
    site_id: Uuid,
    candidates: Vec<LocalAssetCandidate>,
) -> Vec<AdminLocalAssetCandidateRow> {
    let mut rows = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let relative_path = candidate.relative_path.to_string_lossy().to_string();
        rows.push(AdminLocalAssetCandidateRow {
            preview_url: format!(
                "/admin/site/{site_id}/assets/mass-import/preview?candidate={}",
                encode_query_value(&relative_path)
            ),
            byte_length: format_byte_length(candidate.byte_length),
            dimensions: candidate_dimensions(candidate.absolute_path.clone())
                .await
                .unwrap_or_else(|| "n/a".to_string()),
            rank_label: match candidate.rank {
                LocalAssetCandidateRank::PathSuffix => "Path match".to_string(),
                LocalAssetCandidateRank::Filename => "Filename match".to_string(),
            },
            relative_path,
        });
    }
    rows
}

async fn candidate_dimensions(path: PathBuf) -> Option<String> {
    tokio::task::spawn_blocking(move || image::image_dimensions(path).ok())
        .await
        .ok()
        .flatten()
        .map(|(width, height)| format!("{width} x {height}"))
}

async fn apply_group_replacements<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    actor_sub: &str,
    group: &MissingAssetGroup,
    shortcode: &str,
) -> Result<(), SiteError> {
    for affected in &group.affected_content {
        let content = entities::content_item::Entity::find_by_id(affected.content_id)
            .filter(entities::content_item::Column::SiteId.eq(site_id))
            .one(db)
            .await
            .map_err(SiteError::from)?
            .ok_or(SiteError::NotFound)?;
        let mut page_content = content.page_content.clone();
        let mut occurrences = group
            .occurrences
            .iter()
            .filter(|occurrence| occurrence.content_id == content.id)
            .collect::<Vec<_>>();
        occurrences.sort_by_key(|occurrence| std::cmp::Reverse(occurrence.start));
        for occurrence in occurrences {
            let Some(current) = page_content.get(occurrence.start..occurrence.end) else {
                return Err(SiteError::BadRequest(
                    "content changed before mass import could apply".to_string(),
                ));
            };
            if !current.contains(&occurrence.original_link) {
                return Err(SiteError::BadRequest(
                    "content changed before mass import could apply".to_string(),
                ));
            }
            page_content.replace_range(occurrence.start..occurrence.end, shortcode);
        }
        update_content(
            db,
            crate::UpdateContent {
                content_id: content.id,
                page_type: None,
                title: Some(content.title),
                slug: Some(content.slug),
                page_content: Some(page_content),
                draft: Some(content.draft),
                published_at: content.published_at,
                editor_sub: actor_sub.to_string(),
            },
        )
        .await
        .map_err(SiteError::internal)?;
    }
    Ok(())
}

fn asset_alt_text(normalized_path: &str) -> String {
    StdPath::new(normalized_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.replace(['-', '_'], " "))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Imported image".to_string())
}

fn format_byte_length(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    if bytes < 1024 * 1024 {
        return format!("{:.1} KB", bytes as f64 / 1024.0);
    }
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn encode_query_value(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
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

#[cfg(test)]
mod tests {
    use crate::constants::DEFAULT_TEMPLATE_NAME;
    use crate::db::test_db_start;
    use crate::entities::PageType;

    use super::*;

    #[tokio::test]
    async fn load_mass_import_content_for_path_prefilters_matching_spellings() {
        let db = test_db_start().await;
        let site = crate::create_site(
            &db,
            "mass-import-prefilter".to_string(),
            "Mass Import Prefilter".to_string(),
            DEFAULT_TEMPLATE_NAME.to_string(),
        )
        .await
        .expect("failed to create site");
        let other_site = crate::create_site(
            &db,
            "other-prefilter-site".to_string(),
            "Other Prefilter Site".to_string(),
            DEFAULT_TEMPLATE_NAME.to_string(),
        )
        .await
        .expect("failed to create other site");

        let full_url = crate::create_content(
            &db,
            crate::NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: "Full URL".to_string(),
                slug: "full-url".to_string(),
                page_content:
                    r#"<img src="https://example.com/wp-content/uploads/2020/hero.png" />"#
                        .to_string(),
                draft: false,
                creator_sub: "tester".to_string(),
                created_at: None,
                published_at: None,
            },
        )
        .await
        .expect("failed to create full-url content");
        let bare_relative = crate::create_content(
            &db,
            crate::NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: "Bare Relative".to_string(),
                slug: "bare-relative".to_string(),
                page_content: "![Hero](wp-content/uploads/2020/hero.png)".to_string(),
                draft: false,
                creator_sub: "tester".to_string(),
                created_at: None,
                published_at: None,
            },
        )
        .await
        .expect("failed to create bare-relative content");
        crate::create_content(
            &db,
            crate::NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: "Other Asset".to_string(),
                slug: "other-asset".to_string(),
                page_content: "![Other](/wp-content/uploads/2020/other.png)".to_string(),
                draft: false,
                creator_sub: "tester".to_string(),
                created_at: None,
                published_at: None,
            },
        )
        .await
        .expect("failed to create unrelated content");
        crate::create_content(
            &db,
            crate::NewContent {
                site_id: other_site.id,
                page_type: PageType::Post,
                title: "Other Site".to_string(),
                slug: "other-site".to_string(),
                page_content: "![Hero](/wp-content/uploads/2020/hero.png)".to_string(),
                draft: false,
                creator_sub: "tester".to_string(),
                created_at: None,
                published_at: None,
            },
        )
        .await
        .expect("failed to create other-site content");

        let content =
            load_mass_import_content_for_path(&db, site.id, "/wp-content/uploads/2020/hero.png")
                .await
                .expect("failed to load target content");
        let ids = content
            .iter()
            .map(|content| content.id)
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&full_url.id));
        assert!(ids.contains(&bare_relative.id));
    }

    #[tokio::test]
    async fn load_mass_import_candidate_content_prefilters_image_links() {
        let db = test_db_start().await;
        let site = crate::create_site(
            &db,
            "mass-import-candidates".to_string(),
            "Mass Import Candidates".to_string(),
            DEFAULT_TEMPLATE_NAME.to_string(),
        )
        .await
        .expect("failed to create site");
        let image_content = crate::create_content(
            &db,
            crate::NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: "Image Candidate".to_string(),
                slug: "image-candidate".to_string(),
                page_content: r#"<a href="/wp-content/uploads/2020/hero.PNG">hero</a>"#.to_string(),
                draft: false,
                creator_sub: "tester".to_string(),
                created_at: None,
                published_at: None,
            },
        )
        .await
        .expect("failed to create image content");
        crate::create_content(
            &db,
            crate::NewContent {
                site_id: site.id,
                page_type: PageType::Post,
                title: "Plain Text".to_string(),
                slug: "plain-text".to_string(),
                page_content: "No image references here.".to_string(),
                draft: false,
                creator_sub: "tester".to_string(),
                created_at: None,
                published_at: None,
            },
        )
        .await
        .expect("failed to create plain content");

        let content = load_mass_import_candidate_content(&db, site.id, 100, 0)
            .await
            .expect("failed to load candidate content");

        assert_eq!(content.len(), 1);
        assert_eq!(content[0].id, image_content.id);
    }

    #[test]
    fn parse_mass_import_content_apply_form_collects_single_and_batch_selections() {
        let hero_path = "/wp-content/uploads/2020/hero.png";
        let hero_candidate = "wp-content/uploads/2020/hero.png";
        let hero = encode_mass_import_selection(hero_path, hero_candidate);
        let other = encode_mass_import_selection(
            "/wp-content/uploads/2020/other.png",
            "wp-content/uploads/2020/other.png",
        );
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("single_selection", &hero);
        form.append_pair("selection_other", &other);

        let parsed = parse_mass_import_content_apply_form(form.finish().as_bytes())
            .expect("expected form to parse");

        assert_eq!(parsed.selections.len(), 1);
        assert_eq!(parsed.selections[0].normalized_path, hero_path);
        assert_eq!(parsed.selections[0].candidate, hero_candidate);
    }

    #[test]
    fn parse_mass_import_content_apply_form_rejects_duplicate_paths() {
        let first = encode_mass_import_selection(
            "/wp-content/uploads/2020/hero.png",
            "wp-content/uploads/2020/hero.png",
        );
        let second =
            encode_mass_import_selection("/wp-content/uploads/2020/hero.png", "elsewhere/hero.png");
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("selection_first", &first);
        form.append_pair("selection_second", &second);

        let error = parse_mass_import_content_apply_form(form.finish().as_bytes())
            .expect_err("duplicate normalized paths should be rejected");

        assert!(error.to_string().contains("duplicate"));
    }
}
