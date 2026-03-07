use axum::{Json, response::IntoResponse};

#[derive(Debug)]
pub enum SiteError {
    NotFound,
    Internal(String),
    UnAuthorized(String),
}

impl SiteError {
    pub fn internal(msg: String) -> Self {
        SiteError::Internal(msg)
    }
}

impl IntoResponse for SiteError {
    fn into_response(self) -> axum::response::Response {
        match self {
            SiteError::NotFound => (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response(),
            SiteError::Internal(msg) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(msg)).into_response()
            }
            SiteError::UnAuthorized(msg) => {
                (axum::http::StatusCode::UNAUTHORIZED, msg).into_response()
            }
        }
    }
}

impl std::fmt::Display for SiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SiteError::NotFound => write!(f, "not found"),
            SiteError::Internal(msg) => write!(f, "internal error: {msg}"),
            SiteError::UnAuthorized(msg) => write!(f, "unauthorized: {msg}"),
        }
    }
}
