use axum::{extract::Request, middleware::Next, response::Response};
use reqwest::header;
use tracing::info;

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

pub(crate) async fn dont_cache_me(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store, no-cache, must-revalidate, proxy-revalidate"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, header::HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(header::EXPIRES, header::HeaderValue::from_static("0"));
    response
}
