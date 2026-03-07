use axum::response::IntoResponse;

pub enum SiteError {
    NotFound,
    InternalError(String),
}

impl IntoResponse for SiteError {
    fn into_response(self) -> axum::response::Response {
        match self {
            SiteError::NotFound => (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response(),
            SiteError::InternalError(msg) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
