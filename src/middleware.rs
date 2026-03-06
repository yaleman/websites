use axum::{extract::Request, middleware::Next, response::Response};
use tracing::info;

/// Handles request/response logging for all requests. Should be the outermost middleware so that it can log all requests, including those that fail authentication.
pub async fn log_requests(request: Request, next: Next) -> Response {
    let http_method = request.method().clone();
    let uri = request.uri().clone();
    let http_host = uri.host().unwrap_or_default().to_string();
    let http_path = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_default();
    let res = next.run(request).await;
    info!(
        http_method=http_method.as_str(),
        http_host=http_host,
        http_path=http_path,
        status=?res.status(),
        ""
    );
    res
}
