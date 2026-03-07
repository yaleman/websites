use axum::response::IntoResponse;

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
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
            SiteError::UnAuthorized(msg) => {
                (axum::http::StatusCode::UNAUTHORIZED, msg).into_response()
            }
        }
    }
}
