use std::str::FromStr;

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use compact_jwt::{JwsHs256Signer, JwsSigner, JwsVerifier, Jwt, JwtUnverified};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tower_sessions::Session;
use uuid::Uuid;

use crate::constants::SESSION_USER;
use crate::entities;
use crate::entities::setting::SettingKey;
use crate::errors::SiteError;
use crate::web::SiteRole;

pub const API_JWT_AUDIENCE: &str = "websites-api";
pub const API_TOKEN_IDLE_DAYS: i64 = 7;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JwtHs256SecretSetting {
    pub secret_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenGrantSet {
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub sites: Vec<TokenSiteGrant>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenSiteGrant {
    pub site_id: Uuid,
    pub role: SiteRole,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiTokenClaims {
    pub token_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grants: Option<TokenGrantSet>,
}

#[derive(Clone, Debug)]
pub struct IssuedApiToken {
    pub row: entities::user_api_token::Model,
    pub token: String,
}

#[derive(Clone, Debug)]
pub enum ApiPrincipalKind {
    Session,
    Bearer {
        grants: Option<TokenGrantSet>,
        token: Box<entities::user_api_token::Model>,
    },
}

#[derive(Clone, Debug)]
pub struct ApiPrincipal {
    pub user: entities::user::Model,
    pub kind: ApiPrincipalKind,
}

#[derive(Debug)]
pub enum ApiAuthError {
    Unauthorized(String),
    Forbidden(String),
    Site(SiteError),
}

impl From<SiteError> for ApiAuthError {
    fn from(value: SiteError) -> Self {
        Self::Site(value)
    }
}

impl From<sea_orm::DbErr> for ApiAuthError {
    fn from(value: sea_orm::DbErr) -> Self {
        Self::Site(SiteError::from(value))
    }
}

impl IntoResponse for ApiAuthError {
    fn into_response(self) -> Response {
        match self {
            ApiAuthError::Unauthorized(message) => bearer_response(
                StatusCode::UNAUTHORIZED,
                HeaderValue::from_static("Bearer"),
                message,
            ),
            ApiAuthError::Forbidden(message) => bearer_response(
                StatusCode::FORBIDDEN,
                HeaderValue::from_static("Bearer error=\"insufficient_scope\""),
                message,
            ),
            ApiAuthError::Site(error) => error.into_response(),
        }
    }
}

fn bearer_response(status: StatusCode, authenticate: HeaderValue, message: String) -> Response {
    let mut response = (status, message).into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, authenticate);
    response
}

pub fn next_inactive_expiry(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::days(API_TOKEN_IDLE_DAYS)
}

pub fn summarize_grants(grants: Option<&TokenGrantSet>) -> String {
    match grants {
        None => "Current user access".to_string(),
        Some(grants) => {
            let mut parts = Vec::new();
            if grants.admin {
                parts.push("admin".to_string());
            }
            for site in &grants.sites {
                parts.push(format!("{}:{}", site.site_id, site.role.label()));
            }
            if parts.is_empty() {
                "No access".to_string()
            } else {
                parts.join(", ")
            }
        }
    }
}

impl TokenGrantSet {
    pub fn site_role(&self, site_id: Uuid) -> Option<SiteRole> {
        self.sites
            .iter()
            .find(|grant| grant.site_id == site_id)
            .map(|grant| grant.role)
    }
}

impl ApiPrincipal {
    pub fn is_bearer(&self) -> bool {
        matches!(self.kind, ApiPrincipalKind::Bearer { .. })
    }

    pub async fn record_successful_use<C: ConnectionTrait>(
        &self,
        db: &C,
    ) -> Result<(), ApiAuthError> {
        if let ApiPrincipalKind::Bearer { token, .. } = &self.kind {
            touch_user_api_token(db, token.id, Utc::now()).await?;
        }
        Ok(())
    }

    pub async fn require_site_role<C: ConnectionTrait>(
        &self,
        db: &C,
        site_id: Uuid,
        required: SiteRole,
    ) -> Result<(), ApiAuthError> {
        let (effective_role, forbid_on_failure) = match &self.kind {
            ApiPrincipalKind::Session => {
                if self.user.admin {
                    (Some(SiteRole::Admin), false)
                } else {
                    (
                        crate::get_membership_for_subject(db, site_id, &self.user.subject)
                            .await?
                            .map(|membership| membership.role),
                        false,
                    )
                }
            }
            ApiPrincipalKind::Bearer { grants, .. } => (
                effective_bearer_role(db, &self.user, grants.as_ref(), site_id).await?,
                true,
            ),
        };

        let Some(role) = effective_role else {
            return if forbid_on_failure {
                Err(ApiAuthError::Forbidden(format!(
                    "missing membership for site {site_id}"
                )))
            } else {
                Err(ApiAuthError::Unauthorized(format!(
                    "missing membership for site {site_id}"
                )))
            };
        };

        if role_satisfies(role, required) {
            Ok(())
        } else {
            let message = format!(
                "site role {} does not satisfy required role {}",
                role.label(),
                required.label()
            );
            if forbid_on_failure {
                Err(ApiAuthError::Forbidden(message))
            } else {
                Err(ApiAuthError::Unauthorized(message))
            }
        }
    }
}

pub async fn authenticate_api_request(
    db: &DatabaseConnection,
    signer: &JwsHs256Signer,
    issuer: &str,
    headers: &HeaderMap,
    session: &Session,
) -> Result<ApiPrincipal, ApiAuthError> {
    match extract_bearer_token(headers)? {
        Some(token) => authenticate_bearer_token(db, signer, issuer, token).await,
        None => authenticate_session(session).await,
    }
}

pub async fn authenticate_api_request_from_parts(
    db: &DatabaseConnection,
    signer: &JwsHs256Signer,
    issuer: &str,
    request: &Request,
    session: &Session,
) -> Result<ApiPrincipal, ApiAuthError> {
    authenticate_api_request(db, signer, issuer, request.headers(), session).await
}

pub async fn ensure_jwt_hs256_secret<C: ConnectionTrait>(
    db: &C,
) -> Result<JwtHs256SecretSetting, SiteError> {
    if let Some(value) = get_setting_json(db, SettingKey::JwtHs256Secret).await? {
        return Ok(value);
    }

    let value = JwtHs256SecretSetting {
        secret_bytes: rand::random::<[u8; 32]>().to_vec(),
    };
    upsert_setting_json(db, SettingKey::JwtHs256Secret, &value).await?;
    Ok(value)
}

pub fn signer_from_secret(secret: &JwtHs256SecretSetting) -> Result<JwsHs256Signer, SiteError> {
    JwsHs256Signer::try_from(secret.secret_bytes.as_slice())
        .map_err(|error| SiteError::internal(format!("failed to build jwt signer: {error:?}")))
}

pub async fn get_setting_json<C, T>(db: &C, key: SettingKey) -> Result<Option<T>, SiteError>
where
    C: ConnectionTrait,
    T: DeserializeOwned,
{
    let value = entities::setting::Entity::find_by_id(key).one(db).await?;
    value
        .map(|model| serde_json::from_value(model.value_json))
        .transpose()
        .map_err(|error| SiteError::internal(format!("failed to deserialize setting: {error}")))
}

pub async fn upsert_setting_json<C, T>(
    db: &C,
    key: SettingKey,
    value: &T,
) -> Result<entities::setting::Model, SiteError>
where
    C: ConnectionTrait,
    T: Serialize,
{
    let value_json = serde_json::to_value(value)
        .map_err(|error| SiteError::internal(format!("failed to serialize setting: {error}")))?;

    if let Some(existing) = entities::setting::Entity::find_by_id(key).one(db).await? {
        let mut active = existing.into_active_model();
        active.value_json = Set(value_json);
        active.updated_at = Set(Utc::now());
        active.update(db).await.map_err(SiteError::from)
    } else {
        entities::setting::ActiveModel {
            key: Set(key),
            value_json: Set(value_json),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .map_err(SiteError::from)
    }
}

pub async fn issue_user_api_token<C: ConnectionTrait>(
    db: &C,
    signer: &JwsHs256Signer,
    issuer: &str,
    user: &entities::user::Model,
    issued_by: &entities::user::Model,
    label: &str,
    grants: Option<TokenGrantSet>,
) -> Result<IssuedApiToken, SiteError> {
    let now = Utc::now();
    let token_id = Uuid::now_v7();
    let jwt_id = Uuid::now_v7().to_string();
    let claims = ApiTokenClaims {
        token_id,
        grants: grants.clone(),
    };
    let jwt = Jwt {
        iss: Some(issuer.to_string()),
        sub: Some(user.subject.clone()),
        aud: Some(API_JWT_AUDIENCE.to_string()),
        iat: Some(now.timestamp()),
        jti: Some(jwt_id.clone()),
        extensions: claims,
        ..Default::default()
    };
    let token = signer
        .sign(&jwt)
        .map_err(|error| SiteError::internal(format!("failed to sign jwt: {error:?}")))?
        .to_string();
    let grants_json = serialize_grants_json(grants.as_ref())?;
    let row = entities::user_api_token::ActiveModel {
        id: Set(token_id),
        user_id: Set(user.id),
        issued_by_user_id: Set(issued_by.id),
        label: Set(label.to_string()),
        jwt_id: Set(jwt_id),
        grants_json: Set(grants_json),
        created_at: Set(now),
        last_used_at: Set(None),
        inactive_expires_at: Set(next_inactive_expiry(now)),
        revoked_at: Set(None),
        revoked_by_user_id: Set(None),
    }
    .insert(db)
    .await?;

    Ok(IssuedApiToken { row, token })
}

pub async fn list_user_api_tokens<C: ConnectionTrait>(
    db: &C,
    user_id: Uuid,
) -> Result<Vec<entities::user_api_token::Model>, SiteError> {
    entities::user_api_token::Entity::find()
        .filter(entities::user_api_token::Column::UserId.eq(user_id))
        .order_by_desc(entities::user_api_token::Column::CreatedAt)
        .all(db)
        .await
        .map_err(SiteError::from)
}

pub async fn get_user_api_token_by_id<C: ConnectionTrait>(
    db: &C,
    token_id: Uuid,
) -> Result<Option<entities::user_api_token::Model>, SiteError> {
    entities::user_api_token::Entity::find_by_id(token_id)
        .one(db)
        .await
        .map_err(SiteError::from)
}

pub async fn revoke_user_api_token<C: ConnectionTrait>(
    db: &C,
    token_id: Uuid,
    revoked_by_user_id: Uuid,
) -> Result<entities::user_api_token::Model, SiteError> {
    let Some(existing) = get_user_api_token_by_id(db, token_id).await? else {
        return Err(SiteError::NotFound);
    };

    let mut active = existing.into_active_model();
    active.revoked_at = Set(Some(Utc::now()));
    active.revoked_by_user_id = Set(Some(revoked_by_user_id));
    active.update(db).await.map_err(SiteError::from)
}

pub fn deserialize_grants_json(
    value: Option<&serde_json::Value>,
) -> Result<Option<TokenGrantSet>, SiteError> {
    value
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| {
            SiteError::internal(format!("failed to deserialize token grants: {error}"))
        })
}

pub fn serialize_grants_json(
    value: Option<&TokenGrantSet>,
) -> Result<Option<serde_json::Value>, SiteError> {
    value
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| SiteError::internal(format!("failed to serialize token grants: {error}")))
}

fn extract_bearer_token(headers: &HeaderMap) -> Result<Option<&str>, ApiAuthError> {
    let Some(header_value) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let header_value = header_value
        .to_str()
        .map_err(|_| ApiAuthError::Unauthorized("invalid authorization header".to_string()))?;
    let Some(token) = header_value.strip_prefix("Bearer ") else {
        return Err(ApiAuthError::Unauthorized(
            "invalid authorization scheme".to_string(),
        ));
    };
    if token.trim().is_empty() {
        return Err(ApiAuthError::Unauthorized(
            "missing bearer token".to_string(),
        ));
    }
    Ok(Some(token))
}

async fn authenticate_session(session: &Session) -> Result<ApiPrincipal, ApiAuthError> {
    let user = session
        .get::<entities::user::Model>(SESSION_USER)
        .await
        .map_err(|_| ApiAuthError::Unauthorized("failed to read session".to_string()))?
        .ok_or_else(|| ApiAuthError::Unauthorized("missing bearer token".to_string()))?;

    Ok(ApiPrincipal {
        user,
        kind: ApiPrincipalKind::Session,
    })
}

async fn authenticate_bearer_token(
    db: &DatabaseConnection,
    signer: &JwsHs256Signer,
    issuer: &str,
    token: &str,
) -> Result<ApiPrincipal, ApiAuthError> {
    let unverified = JwtUnverified::<ApiTokenClaims>::from_str(token)
        .map_err(|_| ApiAuthError::Unauthorized("invalid bearer token".to_string()))?;
    let jwt = signer
        .verify(&unverified)
        .map_err(|_| ApiAuthError::Unauthorized("invalid bearer token".to_string()))?;
    if jwt.iss.as_deref() != Some(issuer) {
        return Err(ApiAuthError::Unauthorized(
            "invalid token issuer".to_string(),
        ));
    }
    if jwt.aud.as_deref() != Some(API_JWT_AUDIENCE) {
        return Err(ApiAuthError::Unauthorized(
            "invalid token audience".to_string(),
        ));
    }
    let subject = jwt
        .sub
        .as_deref()
        .ok_or_else(|| ApiAuthError::Unauthorized("missing token subject".to_string()))?;
    let jwt_id = jwt
        .jti
        .as_deref()
        .ok_or_else(|| ApiAuthError::Unauthorized("missing token id".to_string()))?;

    let Some(token_row) = entities::user_api_token::Entity::find()
        .filter(entities::user_api_token::Column::JwtId.eq(jwt_id.to_string()))
        .one(db)
        .await?
    else {
        return Err(ApiAuthError::Unauthorized(
            "unknown bearer token".to_string(),
        ));
    };

    if token_row.id != jwt.extensions.token_id {
        return Err(ApiAuthError::Unauthorized(
            "invalid bearer token".to_string(),
        ));
    }

    let stored_grants = deserialize_grants_json(token_row.grants_json.as_ref())?;
    if stored_grants != jwt.extensions.grants {
        return Err(ApiAuthError::Unauthorized(
            "invalid bearer token".to_string(),
        ));
    }

    if token_row.revoked_at.is_some() {
        return Err(ApiAuthError::Unauthorized("token revoked".to_string()));
    }

    let now = Utc::now();
    if token_row.inactive_expires_at < now {
        return Err(ApiAuthError::Unauthorized("token expired".to_string()));
    }

    let user = crate::get_user_by_id(db, token_row.user_id)
        .await?
        .ok_or_else(|| ApiAuthError::Unauthorized("missing token user".to_string()))?;
    if user.subject != subject {
        return Err(ApiAuthError::Unauthorized(
            "invalid bearer token".to_string(),
        ));
    }

    Ok(ApiPrincipal {
        user,
        kind: ApiPrincipalKind::Bearer {
            grants: stored_grants,
            token: Box::new(token_row),
        },
    })
}

async fn touch_user_api_token<C: ConnectionTrait>(
    db: &C,
    token_id: Uuid,
    used_at: DateTime<Utc>,
) -> Result<(), SiteError> {
    entities::user_api_token::Entity::update_many()
        .col_expr(
            entities::user_api_token::Column::LastUsedAt,
            Expr::value(Some(used_at)),
        )
        .col_expr(
            entities::user_api_token::Column::InactiveExpiresAt,
            Expr::value(next_inactive_expiry(used_at)),
        )
        .filter(entities::user_api_token::Column::Id.eq(token_id))
        .filter(entities::user_api_token::Column::RevokedAt.is_null())
        .exec(db)
        .await
        .map(|_| ())
        .map_err(SiteError::from)
}

async fn effective_bearer_role<C: ConnectionTrait>(
    db: &C,
    user: &entities::user::Model,
    grants: Option<&TokenGrantSet>,
    site_id: Uuid,
) -> Result<Option<SiteRole>, ApiAuthError> {
    if user.admin {
        match grants {
            None => return Ok(Some(SiteRole::Admin)),
            Some(grants) if grants.admin => return Ok(Some(SiteRole::Admin)),
            Some(grants) => return Ok(grants.site_role(site_id)),
        }
    }

    let live_role = crate::get_membership_for_subject(db, site_id, &user.subject)
        .await?
        .map(|membership| membership.role);
    let Some(live_role) = live_role else {
        return Ok(None);
    };

    Ok(match grants {
        None => Some(live_role),
        Some(grants) => grants
            .site_role(site_id)
            .map(|grant_role| min_role(live_role, grant_role)),
    })
}

fn role_satisfies(actual: SiteRole, required: SiteRole) -> bool {
    role_rank(actual) >= role_rank(required)
}

fn min_role(left: SiteRole, right: SiteRole) -> SiteRole {
    if role_rank(left) <= role_rank(right) {
        left
    } else {
        right
    }
}

fn role_rank(role: SiteRole) -> u8 {
    match role {
        SiteRole::Viewer => 0,
        SiteRole::Author => 1,
        SiteRole::Editor => 2,
        SiteRole::Owner => 3,
        SiteRole::Admin => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::EntityTrait;

    #[test]
    fn summarize_current_access_when_unrestricted() {
        assert_eq!(summarize_grants(None), "Current user access");
    }

    #[test]
    fn restricted_bearer_role_uses_lower_role() {
        assert_eq!(
            min_role(SiteRole::Owner, SiteRole::Author),
            SiteRole::Author
        );
        assert_eq!(
            min_role(SiteRole::Viewer, SiteRole::Editor),
            SiteRole::Viewer
        );
    }

    #[tokio::test]
    async fn ensure_secret_is_created_once() {
        let db = crate::db::test_db_start().await;

        let first = ensure_jwt_hs256_secret(&db)
            .await
            .expect("failed to create secret");
        let second = ensure_jwt_hs256_secret(&db)
            .await
            .expect("failed to reuse secret");

        assert_eq!(first, second);
        let rows = entities::setting::Entity::find()
            .all(&db)
            .await
            .expect("failed to load settings");
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn issued_token_only_refreshes_idle_expiry_after_successful_use() {
        let db = crate::db::test_db_start().await;
        let secret = ensure_jwt_hs256_secret(&db)
            .await
            .expect("failed to create secret");
        let signer = signer_from_secret(&secret).expect("failed to build signer");
        let user = entities::user::create_user(&db, "token-user", None, None, false)
            .await
            .expect("failed to create user");
        let site = crate::create_site(
            &db,
            "token-site".to_string(),
            "Token Site".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create site");
        crate::create_membership(
            &db,
            crate::NewMembership {
                site_id: site.id,
                user_id: user.id,
                role: SiteRole::Author,
            },
        )
        .await
        .expect("failed to create membership");
        let issued = issue_user_api_token(
            &db,
            &signer,
            "https://example.test",
            &user,
            &user,
            "author token",
            None,
        )
        .await
        .expect("failed to issue token");

        let principal =
            authenticate_bearer_token(&db, &signer, "https://example.test", &issued.token)
                .await
                .expect("failed to authenticate token");

        let untouched = get_user_api_token_by_id(&db, issued.row.id)
            .await
            .expect("failed to load untouched token")
            .expect("missing untouched token");
        assert!(untouched.last_used_at.is_none());
        assert_eq!(
            untouched.inactive_expires_at,
            issued.row.inactive_expires_at
        );

        principal
            .require_site_role(&db, site.id, SiteRole::Author)
            .await
            .expect("expected author access");
        principal
            .record_successful_use(&db)
            .await
            .expect("expected successful use to refresh expiry");

        let refreshed = get_user_api_token_by_id(&db, issued.row.id)
            .await
            .expect("failed to load refreshed token")
            .expect("missing refreshed token");
        assert!(refreshed.last_used_at.is_some());
        assert!(refreshed.inactive_expires_at > issued.row.inactive_expires_at);
    }

    #[tokio::test]
    async fn restricted_grants_limit_access() {
        let db = crate::db::test_db_start().await;
        let signer = signer_from_secret(
            &ensure_jwt_hs256_secret(&db)
                .await
                .expect("failed to create secret"),
        )
        .expect("failed to build signer");
        let user = entities::user::create_user(&db, "restricted-user", None, None, false)
            .await
            .expect("failed to create user");
        let site = crate::create_site(
            &db,
            "restricted-site".to_string(),
            "Restricted Site".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create site");
        crate::create_membership(
            &db,
            crate::NewMembership {
                site_id: site.id,
                user_id: user.id,
                role: SiteRole::Owner,
            },
        )
        .await
        .expect("failed to create membership");
        let issued = issue_user_api_token(
            &db,
            &signer,
            "https://example.test",
            &user,
            &user,
            "restricted token",
            Some(TokenGrantSet {
                admin: false,
                sites: vec![TokenSiteGrant {
                    site_id: site.id,
                    role: SiteRole::Viewer,
                }],
            }),
        )
        .await
        .expect("failed to issue restricted token");

        let principal =
            authenticate_bearer_token(&db, &signer, "https://example.test", &issued.token)
                .await
                .expect("failed to authenticate token");
        let error = principal
            .require_site_role(&db, site.id, SiteRole::Author)
            .await
            .expect_err("expected restricted token to fail author access");
        assert!(matches!(error, ApiAuthError::Forbidden(_)));
        principal
            .require_site_role(&db, site.id, SiteRole::Viewer)
            .await
            .expect("expected viewer access");
    }

    #[tokio::test]
    async fn unrestricted_tokens_follow_current_memberships() {
        let db = crate::db::test_db_start().await;
        let signer = signer_from_secret(
            &ensure_jwt_hs256_secret(&db)
                .await
                .expect("failed to create secret"),
        )
        .expect("failed to build signer");
        let user = entities::user::create_user(&db, "live-user", None, None, false)
            .await
            .expect("failed to create user");
        let site = crate::create_site(
            &db,
            "live-site".to_string(),
            "Live Site".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create site");
        let membership = crate::create_membership(
            &db,
            crate::NewMembership {
                site_id: site.id,
                user_id: user.id,
                role: SiteRole::Author,
            },
        )
        .await
        .expect("failed to create membership");
        let issued = issue_user_api_token(
            &db,
            &signer,
            "https://example.test",
            &user,
            &user,
            "live token",
            None,
        )
        .await
        .expect("failed to issue token");

        crate::update_membership_role(&db, membership.id, SiteRole::Viewer)
            .await
            .expect("failed to downgrade membership");

        let principal =
            authenticate_bearer_token(&db, &signer, "https://example.test", &issued.token)
                .await
                .expect("failed to authenticate token");
        let error = principal
            .require_site_role(&db, site.id, SiteRole::Author)
            .await
            .expect_err("expected downgraded membership to remove author access");
        assert!(matches!(error, ApiAuthError::Forbidden(_)));

        let untouched = get_user_api_token_by_id(&db, issued.row.id)
            .await
            .expect("failed to load token after forbidden access")
            .expect("missing token after forbidden access");
        assert!(untouched.last_used_at.is_none());
        assert_eq!(
            untouched.inactive_expires_at,
            issued.row.inactive_expires_at
        );
    }

    #[tokio::test]
    async fn expired_and_revoked_tokens_are_rejected() {
        let db = crate::db::test_db_start().await;
        let signer = signer_from_secret(
            &ensure_jwt_hs256_secret(&db)
                .await
                .expect("failed to create secret"),
        )
        .expect("failed to build signer");
        let user = entities::user::create_user(&db, "reject-user", None, None, false)
            .await
            .expect("failed to create user");
        let issued = issue_user_api_token(
            &db,
            &signer,
            "https://example.test",
            &user,
            &user,
            "reject token",
            None,
        )
        .await
        .expect("failed to issue token");

        let mut expired = issued.row.clone().into_active_model();
        expired.inactive_expires_at = Set(Utc::now() - Duration::days(1));
        expired.update(&db).await.expect("failed to expire token");

        let expired_error =
            authenticate_bearer_token(&db, &signer, "https://example.test", &issued.token)
                .await
                .expect_err("expected expired token to fail");
        assert!(matches!(expired_error, ApiAuthError::Unauthorized(_)));

        let fresh = issue_user_api_token(
            &db,
            &signer,
            "https://example.test",
            &user,
            &user,
            "revoked token",
            None,
        )
        .await
        .expect("failed to issue second token");
        revoke_user_api_token(&db, fresh.row.id, user.id)
            .await
            .expect("failed to revoke token");

        let revoked_error =
            authenticate_bearer_token(&db, &signer, "https://example.test", &fresh.token)
                .await
                .expect_err("expected revoked token to fail");
        assert!(matches!(revoked_error, ApiAuthError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn successful_use_does_not_restore_a_revoked_token() {
        let db = crate::db::test_db_start().await;
        let signer = signer_from_secret(
            &ensure_jwt_hs256_secret(&db)
                .await
                .expect("failed to create secret"),
        )
        .expect("failed to build signer");
        let user = entities::user::create_user(&db, "stale-principal-user", None, None, false)
            .await
            .expect("failed to create user");
        let issued = issue_user_api_token(
            &db,
            &signer,
            "https://example.test",
            &user,
            &user,
            "stale principal token",
            None,
        )
        .await
        .expect("failed to issue token");

        let principal =
            authenticate_bearer_token(&db, &signer, "https://example.test", &issued.token)
                .await
                .expect("failed to authenticate token");
        revoke_user_api_token(&db, issued.row.id, user.id)
            .await
            .expect("failed to revoke token");

        principal
            .record_successful_use(&db)
            .await
            .expect("recording successful use should be a no-op for revoked tokens");

        let revoked = get_user_api_token_by_id(&db, issued.row.id)
            .await
            .expect("failed to reload revoked token")
            .expect("missing revoked token");
        assert!(revoked.revoked_at.is_some());
        assert_eq!(revoked.revoked_by_user_id, Some(user.id));
    }
}
