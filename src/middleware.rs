use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use reqwest::header;
use tower_sessions::Session;
use tracing::info;

use crate::{constants::SESSION_USER, entities};

/// Handles request/response logging for all requests. Should be the outermost middleware so that it can log all requests, including those that fail authentication.
pub async fn log_requests(request: Request, next: Next) -> Response {
    let http_method = request.method().clone();
    let uri = request.uri().clone();
    let http_path = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_default();
    let res = next.run(request).await;
    info!(
        http_method=http_method.as_str(),
        http_path=http_path,
        status=?res.status(),
        ""
    );
    res
}

/// If you're in test/debug mode, then don't cache responses. Release mode might.
///
/// Relevant MDN links:
///
/// - [Cache-Control](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Cache-Control)
/// - [Pragma](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Pragma)
/// - [Expires](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Expires)
///
pub(crate) async fn set_cache(request: Request, next: Next) -> Response {
    #[cfg(any(test, debug_assertions))]
    {
        let mut response = next.run(request).await;
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static(
                "no-store, no-transform, no-cache, must-revalidate, proxy-revalidate",
            ),
        );
        response
            .headers_mut()
            .insert(header::PRAGMA, header::HeaderValue::from_static("no-cache"));
        response
            .headers_mut()
            .insert(header::EXPIRES, header::HeaderValue::from_static("0"));
        response
    }
    #[cfg(not(any(test, debug_assertions)))]
    {
        let request_uri = request.uri().clone();
        let mut response = next.run(request).await;
        // because 404's might be fixed quick
        if request_uri.path().contains("/assets/") && response.status().is_success() {
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("private, max-age=300"),
            );
        }
        response
    }
}

/// Ensures that the user is authenticated before allowing access to the site. If not authenticated, redirects to the login page.
pub async fn require_session(session: Session, request: Request, next: Next) -> Response {
    let is_authenticated = session
        .get::<entities::user::Model>(SESSION_USER)
        .await
        .unwrap_or(None)
        .is_some();
    if is_authenticated {
        next.run(request).await
    } else {
        Redirect::to("/admin/login").into_response()
    }
}
