use axum::{Json, response::IntoResponse};
use sea_orm::DbErr;

#[derive(Debug)]
pub enum SiteError {
    NotFound,
    Internal(String),
    UnAuthorized(String),
    Database(String),
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
            SiteError::Database(error) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
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

            SiteError::Database(msg) => write!(f, "database error: {msg}"),
        }
    }
}

impl From<sqlx::Error> for SiteError {
    fn from(err: sqlx::Error) -> Self {
        SiteError::Database(err.to_string())
    }
}

impl From<DbErr> for SiteError {
    fn from(err: DbErr) -> Self {
        SiteError::Database(err.to_string())
    }
}
