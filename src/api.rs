use crate::entities::audit_event::log_audit_event;
use crate::entities::{self, PageType};
use crate::errors::SiteError;
use crate::token_auth::{self, authenticate_api_request};
use crate::web::{AdminState, SiteRole};
use crate::{
    NewContent, UpdateContent, collect_asset_filenames, create_content, delete_asset,
    delete_content, get_asset_for_site, get_content_for_site, list_asset_variants, list_assets,
    list_content, list_content_tags, search_content, store_uploaded_asset, sync_tags_to_content,
};
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path as StdPath;
use std::str::FromStr;
use tokio::fs;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub(crate) fn routes() -> Router<AdminState> {
    Router::new()
        .route(
            "/api/site/{site_id}/content",
            get(api_site_content_list).post(api_site_content_create),
        )
        .route(
            "/api/site/{site_id}/content/search",
            get(api_site_content_search),
        )
        .route(
            "/api/site/{site_id}/content/{content_id}",
            get(api_site_content_get)
                .patch(api_site_content_update)
                .delete(api_site_content_delete),
        )
        .route(
            "/api/site/{site_id}/assets",
            get(api_site_assets_list).post(api_site_asset_create),
        )
        .route(
            "/api/site/{site_id}/assets/{asset_id}",
            get(api_site_asset_get).delete(api_site_asset_delete),
        )
        .route(
            "/api/site/{site_id}/assets/library",
            get(api_site_assets_library),
        )
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ApiErrorResponse {
    message: String,
    #[schema(nullable = true)]
    details: Option<String>,
}

#[derive(Debug)]
pub(crate) enum ApiError {
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    Internal(String),
}

impl ApiError {
    fn response(status: StatusCode, message: &str, details: Option<String>) -> Response {
        (
            status,
            Json(ApiErrorResponse {
                message: message.to_string(),
                details,
            }),
        )
            .into_response()
    }

    fn bearer_response(
        status: StatusCode,
        authenticate: HeaderValue,
        message: &str,
        details: Option<String>,
    ) -> Response {
        let mut response = Self::response(status, message, details);
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, authenticate);
        response
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::BadRequest(details) => {
                Self::response(StatusCode::BAD_REQUEST, "Invalid Input", Some(details))
            }
            Self::Unauthorized(details) => Self::bearer_response(
                StatusCode::UNAUTHORIZED,
                HeaderValue::from_static("Bearer"),
                "Unauthorized",
                Some(details),
            ),
            Self::Forbidden(details) => Self::bearer_response(
                StatusCode::FORBIDDEN,
                HeaderValue::from_static("Bearer error=\"insufficient_scope\""),
                "Forbidden",
                Some(details),
            ),
            Self::NotFound(details) => {
                Self::response(StatusCode::NOT_FOUND, "Not Found", Some(details))
            }
            Self::Internal(details) => Self::response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Error",
                Some(details),
            ),
        }
    }
}

impl From<token_auth::ApiAuthError> for ApiError {
    fn from(value: token_auth::ApiAuthError) -> Self {
        match value {
            token_auth::ApiAuthError::Unauthorized(message) => Self::Unauthorized(message),
            token_auth::ApiAuthError::Forbidden(message) => Self::Forbidden(message),
            token_auth::ApiAuthError::Site(error) => Self::from(error),
        }
    }
}

impl From<SiteError> for ApiError {
    fn from(value: SiteError) -> Self {
        match value {
            SiteError::NotFound => Self::NotFound("resource not found".to_string()),
            SiteError::SiteNotFound(identifier) => {
                Self::NotFound(format!("site not found: {identifier}"))
            }
            SiteError::ContentNotFound(identifier) => {
                Self::NotFound(format!("content not found: {identifier}"))
            }
            SiteError::MembershipNotFound(identifier) => {
                Self::NotFound(format!("membership not found: {identifier}"))
            }
            SiteError::BadRequest(message) => Self::BadRequest(message),
            SiteError::UnAuthorized(message) => Self::Unauthorized(message),
            SiteError::Internal(message)
            | SiteError::Database(message)
            | SiteError::Io(message)
            | SiteError::XmlParsing(message) => Self::Internal(message),
            SiteError::TeraTemplate(error) => Self::Internal(error.to_string()),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ContentListQuery {
    #[param(value_type = Option<String>)]
    page_type: Option<String>,
    limit: Option<u64>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ContentSearchQuery {
    q: String,
    #[param(value_type = Option<String>)]
    page_type: Option<String>,
    limit: Option<u64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct ApiCreateContentRequest {
    page_type: String,
    title: String,
    slug: String,
    page_content: String,
    draft: bool,
    published_at: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct ApiUpdateContentRequest {
    page_type: Option<String>,
    title: Option<String>,
    slug: Option<String>,
    page_content: Option<String>,
    draft: Option<bool>,
    published_at: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ApiContentListResponse {
    items: Vec<ContentItemWithTags>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ContentItemWithTags {
    #[serde(flatten)]
    entity: entities::content_item::Model,
    tags: Vec<String>,
}

impl ContentItemWithTags {
    async fn from_content_item(
        entity: entities::content_item::Model,
        db: &DatabaseConnection,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            tags: content_tags_for(db, entity.id).await?,
            entity,
        })
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AssetListQuery {
    q: Option<String>,
    limit: Option<u64>,
    r#type: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AssetLibraryQuery {
    q: Option<String>,
    limit: Option<u64>,
    r#type: Option<String>,
}

#[derive(Debug, ToSchema)]
#[allow(dead_code)]
pub(crate) struct AssetUploadRequest {
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ApiAssetListResponse {
    assets: Vec<ApiAssetSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ApiAssetResponse {
    asset: ApiAssetDetail,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ApiAssetSummary {
    #[serde(flatten)]
    entity: entities::asset::Model,
    original_url: String,
    thumbnail_url: Option<String>,
    has_thumbnail: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ApiAssetDetail {
    #[serde(flatten)]
    entity: entities::asset::Model,
    original_url: String,
    thumbnail_url: Option<String>,
    has_thumbnail: bool,
    variants: Vec<ApiAssetVariant>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ApiAssetVariant {
    variant_kind: String,
    filename: String,
    mime_type: String,
    byte_length: i32,
    width: Option<i32>,
    height: Option<i32>,
    url: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AssetLibraryResponse {
    assets: Vec<AssetLibraryItem>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AssetLibraryItem {
    id: Uuid,
    original_filename: String,
    mime_type: String,
    width: Option<i32>,
    height: Option<i32>,
    created_at: String,
    original_url: String,
    thumbnail_url: Option<String>,
    has_thumbnail: bool,
}

fn parse_page_type(value: Option<&str>) -> Result<Option<PageType>, ApiError> {
    value
        .map(PageType::from_str)
        .transpose()
        .map_err(ApiError::BadRequest)
}

fn parse_optional_datetime(value: Option<&str>) -> Result<Option<DateTime<Utc>>, ApiError> {
    value
        .map(|raw| {
            DateTime::parse_from_rfc3339(raw)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|error| ApiError::BadRequest(format!("invalid published_at: {error}")))
        })
        .transpose()
}

fn normalize_tag_names(tags: Vec<String>) -> Vec<String> {
    tags.into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn normalize_asset_mime_filter(value: &str) -> Option<&'static str> {
    match value {
        "jpeg" | "jpg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "svg" => Some("image/svg+xml"),
        "webp" => Some("image/webp"),
        "all" => None,
        _ => None,
    }
}

async fn content_tags_for(
    db: &sea_orm::DatabaseConnection,
    content_id: Uuid,
) -> Result<Vec<String>, ApiError> {
    let mut tags = list_content_tags(db, content_id)
        .await
        .map_err(ApiError::Internal)?
        .into_iter()
        .map(|tag| tag.name)
        .collect::<Vec<_>>();
    tags.sort();
    Ok(tags)
}

async fn to_content_list_item(
    db: &sea_orm::DatabaseConnection,
    content: entities::content_item::Model,
) -> Result<ContentItemWithTags, ApiError> {
    Ok(ContentItemWithTags {
        tags: content_tags_for(db, content.id).await?,
        entity: content,
    })
}

async fn load_thumbnail_urls(
    db: &sea_orm::DatabaseConnection,
    asset_ids: &[Uuid],
) -> Result<HashMap<Uuid, String>, ApiError> {
    if asset_ids.is_empty() {
        return Ok(HashMap::new());
    }

    Ok(entities::asset_variant::Entity::find()
        .filter(entities::asset_variant::Column::AssetId.is_in(asset_ids.to_vec()))
        .filter(entities::asset_variant::Column::VariantKind.eq("thumbnail"))
        .all(db)
        .await
        .map_err(SiteError::from)?
        .into_iter()
        .map(|variant| {
            (
                variant.asset_id,
                format!("/media/images/{}", variant.filename),
            )
        })
        .collect())
}

async fn to_asset_summary(
    _db: &sea_orm::DatabaseConnection,
    asset: entities::asset::Model,
    thumbnails: &HashMap<Uuid, String>,
) -> Result<ApiAssetSummary, ApiError> {
    let thumbnail_url = thumbnails.get(&asset.id).cloned();
    Ok(ApiAssetSummary {
        entity: asset.clone(),
        original_url: format!("/media/images/{}", asset.storage_basename),
        has_thumbnail: thumbnail_url.is_some(),
        thumbnail_url,
    })
}

async fn to_asset_detail(
    db: &sea_orm::DatabaseConnection,
    asset: entities::asset::Model,
) -> Result<ApiAssetDetail, ApiError> {
    let variants = list_asset_variants(db, asset.id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let thumbnail_url = variants
        .iter()
        .find(|variant| variant.variant_kind == "thumbnail")
        .map(|variant| format!("/media/images/{}", variant.filename));
    let has_thumbnail = thumbnail_url.is_some();
    Ok(ApiAssetDetail {
        entity: asset.clone(),
        original_url: format!("/media/images/{}", asset.storage_basename),
        thumbnail_url,
        has_thumbnail,
        variants: variants
            .into_iter()
            .map(|variant| ApiAssetVariant {
                variant_kind: variant.variant_kind,
                filename: variant.filename.clone(),
                mime_type: variant.mime_type,
                byte_length: variant.byte_length,
                width: variant.width,
                height: variant.height,
                url: format!("/media/images/{}", variant.filename),
            })
            .collect(),
    })
}

async fn authenticate_and_require(
    state: &AdminState,
    headers: &HeaderMap,
    session: &Session,
    site_id: Uuid,
    role: SiteRole,
) -> Result<token_auth::ApiPrincipal, ApiError> {
    let principal = authenticate_api_request(
        state.db.as_ref(),
        state.jwt_signer.as_ref(),
        &state.jwt_issuer,
        headers,
        session,
    )
    .await
    .map_err(ApiError::from)?;
    principal
        .require_site_role(state.db.as_ref(), site_id, role)
        .await
        .map_err(ApiError::from)?;
    Ok(principal)
}

async fn remove_uploaded_files(
    upload_root: &StdPath,
    filenames: &[String],
) -> Result<(), ApiError> {
    for filename in filenames {
        let path = upload_root.join(filename);
        match fs::remove_file(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "failed to remove uploaded file {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/site/{site_id}/content",
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site to list content for"),
        ContentListQuery
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "A list of content items", body = ApiContentListResponse),
        (status = 400, description = "Invalid request parameters", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_content_list(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<ContentListQuery>,
) -> Result<Json<ApiContentListResponse>, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Viewer).await?;
    let page_type = parse_page_type(query.page_type.as_deref())?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200) as usize;

    let mut items = list_content(state.db.as_ref(), site_id, page_type)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    items.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    items.truncate(limit);

    let mut response_items = Vec::with_capacity(items.len());
    for item in items {
        response_items.push(to_content_list_item(state.db.as_ref(), item).await?);
    }
    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(Json(ApiContentListResponse {
        items: response_items,
    }))
}

#[utoipa::path(
    get,
    path = "/api/site/{site_id}/content/search",
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site to search content for"),
        ContentSearchQuery
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "A list of matching content items", body = ApiContentListResponse),
        (status = 400, description = "Invalid request parameters", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_content_search(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<ContentSearchQuery>,
) -> Result<Json<ApiContentListResponse>, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Viewer).await?;
    let query_text = query.q.trim();
    if query_text.is_empty() {
        return Err(ApiError::BadRequest(
            "missing q query parameter".to_string(),
        ));
    }
    let page_type = parse_page_type(query.page_type.as_deref())?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200) as usize;

    let mut items = search_content(state.db.as_ref(), site_id, query_text).await?;
    if let Some(page_type) = page_type {
        items.retain(|item| item.page_type == page_type);
    }
    items.truncate(limit);

    let mut response_items = Vec::with_capacity(items.len());
    for item in items {
        response_items.push(to_content_list_item(state.db.as_ref(), item).await?);
    }
    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(Json(ApiContentListResponse {
        items: response_items,
    }))
}

#[utoipa::path(
    post,
    path = "/api/site/{site_id}/content",
    request_body = ApiCreateContentRequest,
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site to create content for")
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "The created content item", body = ContentItemWithTags),
        (status = 400, description = "Invalid request body", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_content_create(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path(site_id): Path<Uuid>,
    Json(request): Json<ApiCreateContentRequest>,
) -> Result<Json<ContentItemWithTags>, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Author).await?;
    let page_type = PageType::from_str(&request.page_type).map_err(ApiError::BadRequest)?;
    let published_at = parse_optional_datetime(request.published_at.as_deref())?;
    let tags = normalize_tag_names(request.tags);
    let actor = principal.user.subject.clone();

    let txn = state.db.begin().await.map_err(SiteError::from)?;
    let content = create_content(
        &txn,
        NewContent {
            site_id,
            page_type,
            title: request.title,
            slug: request.slug,
            page_content: request.page_content,
            draft: request.draft,
            creator_sub: actor.clone(),
            published_at,
        },
    )
    .await?;

    if !tags.is_empty() {
        let revision = crate::get_revision_by_number(&txn, content.id, 1)
            .await
            .map_err(ApiError::Internal)?
            .ok_or_else(|| ApiError::Internal("missing first revision for content".to_string()))?;
        crate::assign_tags_to_content(&txn, site_id, content.id, revision.id, tags)
            .await
            .map_err(|err| ApiError::Internal(err.to_string()))?;
    }

    log_audit_event(
        &txn,
        &actor,
        "create_content_api",
        "content_item",
        &content.id.to_string(),
        Some(content.site_id),
        Some(json!({
            "page_type": content.page_type.to_string(),
            "slug": content.slug,
            "title": content.title,
            "draft": content.draft
        })),
    )
    .await
    .map_err(|error| ApiError::Internal(format!("failed to log content audit: {error}")))?;
    txn.commit().await.map_err(SiteError::from)?;

    let content = get_content_for_site(state.db.as_ref(), site_id, content.id)
        .await?
        .ok_or_else(|| {
            ApiError::NotFound(format!("content not found after create: {}", content.id))
        })?;
    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(Json(
        ContentItemWithTags::from_content_item(content, state.db.as_ref()).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/site/{site_id}/content/{content_id}",
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site"),
        ("content_id" = Uuid, Path, description = "The ID of the content item")
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "The requested content item", body = ContentItemWithTags),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 404, description = "Content not found", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_content_get(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ContentItemWithTags>, ApiError> {
    let principal: token_auth::ApiPrincipal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Viewer).await?;
    let content = get_content_for_site(state.db.as_ref(), site_id, content_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("content not found: {content_id}")))?;
    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(Json(
        ContentItemWithTags::from_content_item(content, state.db.as_ref()).await?,
    ))
}

#[utoipa::path(
    patch,
    path = "/api/site/{site_id}/content/{content_id}",
    request_body = ApiUpdateContentRequest,
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site"),
        ("content_id" = Uuid, Path, description = "The ID of the content item")
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "The updated content item", body = ContentItemWithTags),
        (status = 400, description = "Invalid request body", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 404, description = "Content not found", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_content_update(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ApiUpdateContentRequest>,
) -> Result<Json<ContentItemWithTags>, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Author).await?;
    if get_content_for_site(state.db.as_ref(), site_id, content_id)
        .await?
        .is_none()
    {
        return Err(ApiError::NotFound(format!(
            "content not found: {content_id}"
        )));
    }
    if request.page_type.is_none()
        && request.title.is_none()
        && request.slug.is_none()
        && request.page_content.is_none()
        && request.draft.is_none()
        && request.published_at.is_none()
        && request.tags.is_none()
    {
        return Err(ApiError::BadRequest(
            "request body did not include any updatable fields".to_string(),
        ));
    }

    let actor = principal.user.subject.clone();
    let page_type = request
        .page_type
        .as_deref()
        .map(PageType::from_str)
        .transpose()
        .map_err(ApiError::BadRequest)?;
    let published_at = parse_optional_datetime(request.published_at.as_deref())?;
    let tags = request.tags.map(normalize_tag_names);

    let txn = state.db.begin().await.map_err(SiteError::from)?;
    let content = crate::update_content(
        &txn,
        UpdateContent {
            content_id,
            page_type,
            title: request.title,
            slug: request.slug,
            page_content: request.page_content,
            draft: request.draft,
            published_at,
            editor_sub: actor.clone(),
        },
    )
    .await
    .map_err(|err| ApiError::Internal(err.to_string()))?;

    if let Some(tags) = tags {
        let revision = entities::content_revision::Entity::find()
            .filter(entities::content_revision::Column::ContentId.eq(content.id))
            .order_by_desc(entities::content_revision::Column::RevisionNumber)
            .one(&txn)
            .await
            .map_err(SiteError::from)?
            .ok_or_else(|| ApiError::Internal("missing latest revision for content".to_string()))?;
        sync_tags_to_content(&txn, site_id, content.id, revision.id, tags)
            .await
            .map_err(|err| ApiError::Internal(err.to_string()))?;
    }

    log_audit_event(
        &txn,
        &actor,
        "update_content_api",
        "content_item",
        &content.id.to_string(),
        Some(content.site_id),
        Some(json!({
            "page_type": content.page_type.to_string(),
            "slug": content.slug,
            "title": content.title,
            "draft": content.draft
        })),
    )
    .await
    .map_err(|error| ApiError::Internal(format!("failed to log content audit: {error}")))?;
    txn.commit().await.map_err(SiteError::from)?;

    let content = get_content_for_site(state.db.as_ref(), site_id, content_id)
        .await?
        .ok_or_else(|| {
            ApiError::NotFound(format!("content not found after update: {content_id}"))
        })?;
    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(Json(
        ContentItemWithTags::from_content_item(content, state.db.as_ref()).await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/api/site/{site_id}/content/{content_id}",
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site"),
        ("content_id" = Uuid, Path, description = "The ID of the content item")
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 204, description = "The content item was deleted"),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 404, description = "Content not found", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_content_delete(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path((site_id, content_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Author).await?;
    let content = get_content_for_site(state.db.as_ref(), site_id, content_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("content not found: {content_id}")))?;
    let actor = principal.user.subject.clone();

    let txn = state.db.begin().await.map_err(SiteError::from)?;
    log_audit_event(
        &txn,
        &actor,
        "delete_content_api",
        "content_item",
        &content.id.to_string(),
        Some(content.site_id),
        Some(json!({
            "slug": content.slug,
            "title": content.title
        })),
    )
    .await
    .map_err(|error| ApiError::Internal(format!("failed to log content audit: {error}")))?;
    delete_content(&txn, site_id, content_id).await?;
    txn.commit().await.map_err(SiteError::from)?;
    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/site/{site_id}/assets",
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site to list assets for"),
        AssetListQuery
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "A list of assets", body = ApiAssetListResponse),
        (status = 400, description = "Invalid request parameters", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_assets_list(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AssetListQuery>,
) -> Result<Json<ApiAssetListResponse>, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Viewer).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200) as usize;
    let mut assets = list_assets(state.db.as_ref(), site_id).await?;
    if let Some(query_text) = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        assets.retain(|asset| {
            asset.original_filename.contains(query_text)
                || asset.storage_basename.contains(query_text)
        });
    }
    if let Some(type_filter) = query
        .r#type
        .as_deref()
        .and_then(normalize_asset_mime_filter)
    {
        assets.retain(|asset| asset.mime_type == type_filter);
    }
    assets.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    assets.truncate(limit);

    let asset_ids = assets.iter().map(|asset| asset.id).collect::<Vec<_>>();
    let thumbnails = load_thumbnail_urls(state.db.as_ref(), &asset_ids).await?;
    let mut response_assets = Vec::with_capacity(assets.len());
    for asset in assets {
        response_assets.push(to_asset_summary(state.db.as_ref(), asset, &thumbnails).await?);
    }

    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(Json(ApiAssetListResponse {
        assets: response_assets,
    }))
}

#[utoipa::path(
    post,
    path = "/api/site/{site_id}/assets",
    request_body(content = AssetUploadRequest, content_type = "multipart/form-data"),
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site to create an asset for")
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "The created asset", body = ApiAssetResponse),
        (status = 400, description = "Invalid request body", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_asset_create(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path(site_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<ApiAssetResponse>, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Author).await?;
    let actor = principal.user.subject.clone();
    let mut upload_bytes: Option<Vec<u8>> = None;
    let mut original_filename: Option<String> = None;
    let mut mime_type: Option<String> = None;

    loop {
        let field = multipart.next_field().await.map_err(|error| {
            ApiError::BadRequest(format!("failed to parse multipart body: {error}"))
        })?;
        let Some(field) = field else { break };
        if field.name() != Some("file") {
            continue;
        }
        original_filename = field.file_name().map(|value| value.to_string());
        mime_type = field.content_type().map(|value| value.to_string());
        let bytes = field.bytes().await.map_err(|error| {
            ApiError::BadRequest(format!("failed to read uploaded file: {error}"))
        })?;
        if bytes.is_empty() {
            continue;
        }
        upload_bytes = Some(bytes.to_vec());
        break;
    }

    let bytes =
        upload_bytes.ok_or_else(|| ApiError::BadRequest("missing file upload".to_string()))?;
    let original_filename = original_filename
        .ok_or_else(|| ApiError::BadRequest("missing original filename".to_string()))?;

    let txn = state.db.begin().await.map_err(SiteError::from)?;
    let asset = store_uploaded_asset(
        &txn,
        &state.upload_root,
        site_id,
        &actor,
        bytes,
        original_filename,
        mime_type,
    )
    .await?;
    log_audit_event(
        &txn,
        &actor,
        "create_asset_api",
        "asset",
        &asset.id.to_string(),
        Some(asset.site_id),
        Some(json!({
            "original_filename": asset.original_filename,
            "storage_basename": asset.storage_basename,
            "mime_type": asset.mime_type
        })),
    )
    .await
    .map_err(|error| ApiError::Internal(format!("failed to log asset audit: {error}")))?;
    txn.commit().await.map_err(SiteError::from)?;

    let asset = get_asset_for_site(state.db.as_ref(), site_id, asset.id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("asset not found after create: {}", asset.id)))?;
    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(Json(ApiAssetResponse {
        asset: to_asset_detail(state.db.as_ref(), asset).await?,
    }))
}

#[utoipa::path(
    get,
    path = "/api/site/{site_id}/assets/{asset_id}",
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site"),
        ("asset_id" = Uuid, Path, description = "The ID of the asset")
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "The requested asset", body = ApiAssetResponse),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 404, description = "Asset not found", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_asset_get(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path((site_id, asset_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ApiAssetResponse>, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Viewer).await?;
    let asset = get_asset_for_site(state.db.as_ref(), site_id, asset_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("asset not found: {asset_id}")))?;
    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(Json(ApiAssetResponse {
        asset: to_asset_detail(state.db.as_ref(), asset).await?,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/site/{site_id}/assets/{asset_id}",
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site"),
        ("asset_id" = Uuid, Path, description = "The ID of the asset")
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 204, description = "The asset was deleted"),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 404, description = "Asset not found", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_asset_delete(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path((site_id, asset_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Author).await?;
    let asset = get_asset_for_site(state.db.as_ref(), site_id, asset_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("asset not found: {asset_id}")))?;
    let actor = principal.user.subject.clone();
    let filenames = collect_asset_filenames(state.db.as_ref(), asset_id).await?;

    let txn = state.db.begin().await.map_err(SiteError::from)?;
    log_audit_event(
        &txn,
        &actor,
        "delete_asset_api",
        "asset",
        &asset.id.to_string(),
        Some(asset.site_id),
        Some(json!({
            "original_filename": asset.original_filename,
            "storage_basename": asset.storage_basename
        })),
    )
    .await
    .map_err(|error| ApiError::Internal(format!("failed to log asset audit: {error}")))?;
    delete_asset(&txn, site_id, asset_id).await?;
    txn.commit().await.map_err(SiteError::from)?;
    remove_uploaded_files(&state.upload_root, &filenames).await?;
    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/site/{site_id}/assets/library",
    params(
        ("site_id" = Uuid, Path, description = "The ID of the site to list assets for"),
        AssetLibraryQuery
    ),
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "A list of assets matching the query", body = AssetLibraryResponse),
        (status = 400, description = "Invalid request parameters", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized access", body = ApiErrorResponse),
        (status = 403, description = "Forbidden - insufficient permissions", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    ),
)]
pub(crate) async fn api_site_assets_library(
    State(state): State<AdminState>,
    headers: HeaderMap,
    session: Session,
    Path(site_id): Path<Uuid>,
    Query(query): Query<AssetLibraryQuery>,
) -> Result<Json<AssetLibraryResponse>, ApiError> {
    let principal =
        authenticate_and_require(&state, &headers, &session, site_id, SiteRole::Viewer).await?;
    let query_text = query.q.unwrap_or_default();
    let query_text = query_text.trim();
    let has_query = !query_text.is_empty();
    let default_limit = if has_query { 50 } else { 12 };
    let limit = query.limit.unwrap_or(default_limit).clamp(1, 200) as usize;

    let mut asset_query = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .filter(entities::asset::Column::MimeType.like("image/%"));

    if let Some(type_filter) = query
        .r#type
        .as_deref()
        .and_then(normalize_asset_mime_filter)
    {
        asset_query = asset_query.filter(entities::asset::Column::MimeType.eq(type_filter));
    }

    if has_query {
        let condition = Condition::any()
            .add(entities::asset::Column::OriginalFilename.contains(query_text))
            .add(entities::asset::Column::StorageBasename.contains(query_text));
        asset_query = asset_query.filter(condition);
    }

    let assets = asset_query
        .order_by_desc(entities::asset::Column::CreatedAt)
        .limit(limit as u64)
        .all(state.db.as_ref())
        .await
        .map_err(SiteError::from)?;

    let asset_ids = assets.iter().map(|asset| asset.id).collect::<Vec<_>>();
    let thumbnails = load_thumbnail_urls(state.db.as_ref(), &asset_ids).await?;
    let items = assets
        .into_iter()
        .map(|asset| {
            let thumbnail_url = thumbnails.get(&asset.id).cloned();
            AssetLibraryItem {
                id: asset.id,
                original_filename: asset.original_filename,
                mime_type: asset.mime_type,
                width: asset.width,
                height: asset.height,
                created_at: asset.created_at.to_rfc3339(),
                original_url: format!("/media/images/{}", asset.storage_basename),
                has_thumbnail: thumbnail_url.is_some(),
                thumbnail_url,
            }
        })
        .collect();

    principal.record_successful_use(state.db.as_ref()).await?;
    Ok(Json(AssetLibraryResponse { assets: items }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::SESSION_USER;
    use crate::db::test_db_start;
    use crate::entities::user::create_user;
    use crate::resolve_log_path;
    use crate::token_auth::{JwtHs256SecretSetting, signer_from_secret};
    use axum::body::{Body, to_bytes};
    use axum::extract::Path;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::get;
    use openidconnect::{ClientId, IssuerUrl};
    use reqwest::Url;
    use sea_orm::{EntityTrait, PaginatorTrait, QueryFilter};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tower::ServiceExt;
    use tower_sessions::{Expiry, MemoryStore, Session, SessionManagerLayer};

    const TINY_PNG_BYTES: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xff, 0xff, 0xff, 0x7f, 0x00, 0x09, 0xfb, 0x03, 0xfd, 0x28, 0xa6, 0xe3, 0x8a, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    async fn test_login(
        State(state): State<AdminState>,
        session: Session,
        Path(user_id): Path<Uuid>,
    ) -> Result<StatusCode, SiteError> {
        let user = crate::get_user_by_id(state.db.as_ref(), user_id)
            .await?
            .ok_or(SiteError::NotFound)?;
        session
            .insert(SESSION_USER, user)
            .await
            .map_err(|_| SiteError::internal("failed to seed session"))?;
        Ok(StatusCode::NO_CONTENT)
    }

    fn test_admin_state(
        db: Arc<sea_orm::DatabaseConnection>,
        upload_root: &StdPath,
        site_templates_root: &StdPath,
    ) -> AdminState {
        let jwt_signer = signer_from_secret(&JwtHs256SecretSetting {
            secret_bytes: vec![7; 32],
        })
        .expect("failed to build jwt signer");
        AdminState {
            db,
            oidc_client_id: ClientId::new("client".to_string()),
            oidc_client_secret: None,
            oidc_frontend_url: Url::parse("https://example.com").expect("invalid frontend url"),
            oidc_discovery_url: IssuerUrl::new("https://example.com".to_string())
                .expect("invalid discovery url"),
            oidc_client: Arc::new(reqwest::Client::new()),
            jwt_signer: Arc::new(jwt_signer),
            jwt_issuer: "https://example.com".to_string(),
            upload_root: upload_root.to_path_buf(),
            log_path: resolve_log_path(),
            site_templates_root: site_templates_root.to_path_buf(),
            rendered_root: std::env::temp_dir()
                .join(format!("websites-rendered-test-{}", Uuid::now_v7())),
        }
    }

    fn test_router(state: AdminState) -> Router {
        let session_layer = SessionManagerLayer::new(MemoryStore::default())
            .with_secure(false)
            .with_expiry(Expiry::OnSessionEnd);
        Router::new()
            .route("/test-login/{user_id}", get(test_login))
            .merge(routes())
            .layer(session_layer)
            .with_state(state)
    }

    async fn seed_session_cookie(router: Router, user_id: Uuid) -> String {
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/test-login/{user_id}"))
                    .body(Body::empty())
                    .expect("failed to build login request"),
            )
            .await
            .expect("failed to perform login request");
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .next()
            .expect("missing set-cookie header")
            .to_str()
            .expect("invalid cookie")
            .split(';')
            .next()
            .expect("missing cookie pair")
            .to_string()
    }

    async fn json_body(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("failed to read body");
        serde_json::from_slice(&body).expect("failed to parse json body")
    }

    fn multipart_body(filename: &str, mime_type: &str, bytes: &[u8]) -> (String, Vec<u8>) {
        let boundary = "api-test-boundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (boundary.to_string(), body)
    }

    fn assert_exists(path: &StdPath) {
        assert!(path.exists(), "expected path to exist: {}", path.display());
    }

    fn assert_missing(path: &StdPath) {
        assert!(
            !path.exists(),
            "expected path to be absent: {}",
            path.display()
        );
    }

    #[tokio::test]
    async fn content_api_supports_crud_and_search() {
        let db = Arc::new(test_db_start().await);
        let upload_root = TempDir::new().expect("failed to create upload root");
        let site_templates_root = TempDir::new().expect("failed to create template root");
        let router = test_router(test_admin_state(
            db.clone(),
            upload_root.path(),
            site_templates_root.path(),
        ));

        let site = crate::create_site(
            db.as_ref(),
            "content-api".to_string(),
            "Content API".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create site");
        let author = create_user(db.as_ref(), "content-author", None, None, false)
            .await
            .expect("failed to create author");
        crate::create_membership(
            db.as_ref(),
            crate::NewMembership {
                site_id: site.id,
                user_id: author.id,
                role: SiteRole::Author,
            },
        )
        .await
        .expect("failed to create author membership");

        let cookie = seed_session_cookie(router.clone(), author.id).await;
        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/site/{}/content", site.id))
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "page_type": "page",
                            "title": "API Draft",
                            "slug": "api-draft",
                            "page_content": "Initial body",
                            "draft": true,
                            "tags": ["alpha", "beta"]
                        })
                        .to_string(),
                    ))
                    .expect("failed to build create request"),
            )
            .await
            .expect("failed to call create route");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_body = json_body(create_response).await;
        let content_id = create_body["id"].as_str().expect("missing content id");

        let get_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/site/{}/content/{}", site.id, content_id))
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .expect("failed to build get request"),
            )
            .await
            .expect("failed to call get route");
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_body = json_body(get_response).await;
        assert_eq!(get_body["title"], "API Draft");
        assert_eq!(get_body["tags"][0], "alpha");

        let update_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/site/{}/content/{}", site.id, content_id))
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "API Published",
                            "draft": false,
                            "tags": ["gamma"]
                        })
                        .to_string(),
                    ))
                    .expect("failed to build update request"),
            )
            .await
            .expect("failed to call update route");
        assert_eq!(update_response.status(), StatusCode::OK);
        let update_body = json_body(update_response).await;
        assert_eq!(update_body["title"], "API Published");
        assert_eq!(update_body["draft"], false);
        assert_eq!(update_body["tags"][0], "gamma");

        let search_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/site/{}/content/search?q=Published", site.id))
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .expect("failed to build search request"),
            )
            .await
            .expect("failed to call search route");
        assert_eq!(search_response.status(), StatusCode::OK);
        let search_body = json_body(search_response).await;
        assert_eq!(
            search_body["items"]
                .as_array()
                .expect("items should be array")
                .len(),
            1
        );

        let delete_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/site/{}/content/{}", site.id, content_id))
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .expect("failed to build delete request"),
            )
            .await
            .expect("failed to call delete route");
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let content_uuid = Uuid::parse_str(content_id).expect("invalid content id");
        assert!(
            crate::get_content_for_site(db.as_ref(), site.id, content_uuid)
                .await
                .expect("failed to fetch content")
                .is_none()
        );
        let revision_count = entities::content_revision::Entity::find()
            .filter(entities::content_revision::Column::ContentId.eq(content_uuid))
            .count(db.as_ref())
            .await
            .expect("failed to count revisions");
        assert_eq!(revision_count, 0);
    }

    #[tokio::test]
    async fn content_api_enforces_viewer_and_author_roles() {
        let db = Arc::new(test_db_start().await);
        let upload_root = TempDir::new().expect("failed to create upload root");
        let site_templates_root = TempDir::new().expect("failed to create template root");
        let router = test_router(test_admin_state(
            db.clone(),
            upload_root.path(),
            site_templates_root.path(),
        ));

        let site = crate::create_site(
            db.as_ref(),
            "content-auth".to_string(),
            "Content Auth".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create site");
        let creator = create_user(db.as_ref(), "creator", None, None, false)
            .await
            .expect("failed to create creator");
        crate::create_membership(
            db.as_ref(),
            crate::NewMembership {
                site_id: site.id,
                user_id: creator.id,
                role: SiteRole::Author,
            },
        )
        .await
        .expect("failed to create creator membership");
        let content = create_content(
            db.as_ref(),
            NewContent {
                site_id: site.id,
                page_type: PageType::Page,
                title: "Role content".to_string(),
                slug: "role-content".to_string(),
                page_content: "Body".to_string(),
                draft: false,
                creator_sub: creator.subject.clone(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create content");

        let viewer = create_user(db.as_ref(), "viewer", None, None, false)
            .await
            .expect("failed to create viewer");
        crate::create_membership(
            db.as_ref(),
            crate::NewMembership {
                site_id: site.id,
                user_id: viewer.id,
                role: SiteRole::Viewer,
            },
        )
        .await
        .expect("failed to create viewer membership");
        let outsider = create_user(db.as_ref(), "outsider", None, None, false)
            .await
            .expect("failed to create outsider");

        let viewer_cookie = seed_session_cookie(router.clone(), viewer.id).await;
        let outsider_cookie = seed_session_cookie(router.clone(), outsider.id).await;

        let viewer_get = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/site/{}/content/{}", site.id, content.id))
                    .header(header::COOKIE, &viewer_cookie)
                    .body(Body::empty())
                    .expect("failed to build viewer get request"),
            )
            .await
            .expect("failed to call viewer get");
        assert_eq!(viewer_get.status(), StatusCode::OK);

        let viewer_post = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/site/{}/content", site.id))
                    .header(header::COOKIE, &viewer_cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "page_type": "page",
                            "title": "Viewer blocked",
                            "slug": "viewer-blocked",
                            "page_content": "Body",
                            "draft": true,
                            "tags": []
                        })
                        .to_string(),
                    ))
                    .expect("failed to build viewer post request"),
            )
            .await
            .expect("failed to call viewer post");
        assert_eq!(viewer_post.status(), StatusCode::UNAUTHORIZED);

        let outsider_get = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/site/{}/content/{}", site.id, content.id))
                    .header(header::COOKIE, &outsider_cookie)
                    .body(Body::empty())
                    .expect("failed to build outsider get request"),
            )
            .await
            .expect("failed to call outsider get");
        assert_eq!(outsider_get.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn asset_api_supports_upload_list_get_and_delete_with_file_cleanup() {
        let db = Arc::new(test_db_start().await);
        let upload_root = TempDir::new().expect("failed to create upload root");
        let site_templates_root = TempDir::new().expect("failed to create template root");
        let router = test_router(test_admin_state(
            db.clone(),
            upload_root.path(),
            site_templates_root.path(),
        ));

        let site = crate::create_site(
            db.as_ref(),
            "asset-api".to_string(),
            "Asset API".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create site");
        let author = create_user(db.as_ref(), "asset-author", None, None, false)
            .await
            .expect("failed to create author");
        crate::create_membership(
            db.as_ref(),
            crate::NewMembership {
                site_id: site.id,
                user_id: author.id,
                role: SiteRole::Author,
            },
        )
        .await
        .expect("failed to create author membership");
        let viewer = create_user(db.as_ref(), "asset-viewer", None, None, false)
            .await
            .expect("failed to create viewer");
        crate::create_membership(
            db.as_ref(),
            crate::NewMembership {
                site_id: site.id,
                user_id: viewer.id,
                role: SiteRole::Viewer,
            },
        )
        .await
        .expect("failed to create viewer membership");

        let author_cookie = seed_session_cookie(router.clone(), author.id).await;
        let viewer_cookie = seed_session_cookie(router.clone(), viewer.id).await;
        let (boundary, body) = multipart_body("tiny.png", "image/png", TINY_PNG_BYTES);

        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/site/{}/assets", site.id))
                    .header(header::COOKIE, &author_cookie)
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .expect("failed to build asset upload request"),
            )
            .await
            .expect("failed to call asset upload");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_body = json_body(create_response).await;
        let asset_id = create_body["asset"]["id"]
            .as_str()
            .expect("missing asset id");
        let asset_uuid = Uuid::parse_str(asset_id).expect("invalid asset id");

        let stored_filenames = collect_asset_filenames(db.as_ref(), asset_uuid)
            .await
            .expect("failed to collect filenames");
        for filename in &stored_filenames {
            assert_exists(&upload_root.path().join(filename));
        }

        let list_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/site/{}/assets", site.id))
                    .header(header::COOKIE, &viewer_cookie)
                    .body(Body::empty())
                    .expect("failed to build asset list request"),
            )
            .await
            .expect("failed to call asset list");
        assert_eq!(list_response.status(), StatusCode::OK);
        let list_body = json_body(list_response).await;
        assert_eq!(
            list_body["assets"]
                .as_array()
                .expect("assets should be array")
                .len(),
            1
        );

        let detail_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/site/{}/assets/{}", site.id, asset_id))
                    .header(header::COOKIE, &viewer_cookie)
                    .body(Body::empty())
                    .expect("failed to build asset get request"),
            )
            .await
            .expect("failed to call asset get");
        assert_eq!(detail_response.status(), StatusCode::OK);
        let detail_body = json_body(detail_response).await;
        assert_eq!(detail_body["asset"]["original_filename"], "tiny.png");
        assert!(
            !detail_body["asset"]["variants"]
                .as_array()
                .expect("variants should be array")
                .is_empty()
        );

        let library_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/site/{}/assets/library", site.id))
                    .header(header::COOKIE, &viewer_cookie)
                    .body(Body::empty())
                    .expect("failed to build library request"),
            )
            .await
            .expect("failed to call asset library");
        assert_eq!(library_response.status(), StatusCode::OK);

        let delete_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/site/{}/assets/{}", site.id, asset_id))
                    .header(header::COOKIE, &author_cookie)
                    .body(Body::empty())
                    .expect("failed to build asset delete request"),
            )
            .await
            .expect("failed to call asset delete");
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        assert!(
            get_asset_for_site(db.as_ref(), site.id, asset_uuid)
                .await
                .expect("failed to reload asset")
                .is_none()
        );
        for filename in &stored_filenames {
            assert_missing(&upload_root.path().join(filename));
        }
    }
}
