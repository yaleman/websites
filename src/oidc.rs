
use axum::{
    extract::{Query, State},
    response::Redirect,
};
use openidconnect::{
    AuthorizationCode, EndpointMaybeSet, EndpointNotSet, EndpointSet, Nonce, PkceCodeVerifier,
    RedirectUrl, TokenResponse,
    core::{CoreClient, CoreProviderMetadata},
};
use reqwest::redirect::Policy;
use serde::Deserialize;
use tower_sessions::Session;

use crate::{entities::user::upsert_user_login, errors::SiteError, web::AdminState};

#[derive(Debug, Deserialize)]
pub(crate) struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

type OidcClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub(crate) static OIDC_SESSION_OIDC_PKCE_KEY: &str = "oidc_pkce";
pub(crate) static OIDC_SESSION_OIDC_STATE_KEY: &str = "oidc_state";
pub(crate) static OIDC_SESSION_OIDC_NONCE_KEY: &str = "oidc_nonce";

/// Builds a HTTP client for use with OIDC operations, configured to allow a limited number of redirects
pub(crate) fn build_http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::ClientBuilder::new()
        .redirect(Policy::limited(5))
        .build()
}

pub(crate) async fn build_oidc_client(
    state: &AdminState,
) -> Result<OidcClient, SiteError> {
    let frontend_url = state.oidc_frontend_url.clone();
    let oidc_client = state.oidc_client.clone();
    let provider_metadata =
        CoreProviderMetadata::discover_async(state.oidc_discovery_url.clone(), &*oidc_client)
            .await
            .map_err(|error| {
                SiteError::internal(format!("failed to discover provider metadata: {error}"))
            })?;

    let redirect_url = frontend_url
        .join("/oauth2/callback")
        .map_err(|error| SiteError::internal(format!("invalid redirect url: {error}")))?;
    let client = CoreClient::from_provider_metadata(
        provider_metadata,
        state.oidc_client_id.clone(),
        state.oidc_client_secret.clone(),
    )
    .set_redirect_uri(RedirectUrl::from_url(redirect_url));

    Ok(client)
}

pub(crate) async fn admin_login_callback(
    State(state): State<AdminState>,
    Query(query): Query<OidcCallbackQuery>,
    session: Session,
) -> Result<Redirect, SiteError> {
    if let Some(error) = query.error {
        let description = query.error_description.unwrap_or_default();
        return Err(SiteError::internal(format!(
            "OIDC error: {error} {description}"
        )));
    }

    let code = match query.code {
        Some(code) => code,
        None => {
            return Err(SiteError::UnAuthorized(
                "missing authorization code".to_string(),
            ));
        }
    };
    let state_value = match query.state {
        Some(state) => state,
        None => return Err(SiteError::UnAuthorized("missing state".to_string())),
    };

    let stored_state = session
        .get::<String>(OIDC_SESSION_OIDC_STATE_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or_default();
    if stored_state != state_value {
        return Err(SiteError::UnAuthorized("OIDC state mismatch".to_string()));
    }

    let pkce_verifier = session
        .get::<String>(OIDC_SESSION_OIDC_PKCE_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or_default();
    let nonce_value = session
        .get::<String>(OIDC_SESSION_OIDC_NONCE_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or_default();

    let client = match build_oidc_client(&state).await {
        Ok(client) => client,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to initialize OIDC client: {error}"
            )));
        }
    };

    let token_request = match client.exchange_code(AuthorizationCode::new(code)) {
        Ok(request) => request,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to build token request: {error}"
            )));
        }
    };
    let token_response = match token_request
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
        .request_async(&*state.oidc_client.clone())
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Err(SiteError::internal(format!(
                "failed to exchange code: {error}"
            )));
        }
    };

    let id_token = match token_response.id_token() {
        Some(token) => token,
        None => {
            return Err(SiteError::internal(
                "missing id_token in response".to_string(),
            ));
        }
    };

    let nonce = Nonce::new(nonce_value);
    let claims = match id_token.claims(&client.id_token_verifier(), &nonce) {
        Ok(claims) => claims,
        Err(error) => {
            return Err(SiteError::UnAuthorized(format!(
                "failed to verify id_token: {error}"
            )));
        }
    };

    let subject = claims.subject().as_str().to_string();
    if session.insert("user_sub", subject.clone()).await.is_err() {
        return Err(SiteError::internal("failed to store session".to_string()));
    }

    upsert_user_login(state.db.as_ref(), &subject).await?;

    Ok(Redirect::to("/admin"))
}
