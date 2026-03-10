use axum::{Json, response::IntoResponse};
use sea_orm::DbErr;
use serde_json::json;
use uuid::Uuid;

#[derive(Debug)]
pub enum SiteError {
    NotFound,
    /// The inner is the short_name or id of the site
    SiteNotFound(String),
    /// The inner is the id of the content
    ContentNotFound(Uuid),
    Internal(String),
    UnAuthorized(String),
    Database(String),
    Io(String),
    TeraTemplate(tera::Error),
    XmlParsing(String),
    BadRequest(String),
    MembershipNotFound(Uuid),
}

impl SiteError {
    pub fn internal(msg: impl ToString) -> Self {
        SiteError::Internal(msg.to_string())
    }
}

impl IntoResponse for SiteError {
    fn into_response(self) -> axum::response::Response {
        match self {
            SiteError::NotFound => (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response(),
            SiteError::Internal(msg) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(msg)).into_response()
            }
            SiteError::UnAuthorized(msg) => (
                axum::http::StatusCode::UNAUTHORIZED,
                format!("{}<br /><a href=\"/\">Login</a>", msg),
            )
                .into_response(),
            SiteError::Database(error) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
            SiteError::Io(error) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
            SiteError::TeraTemplate(err) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Template rendering error: {:?}", err),
            )
                .into_response(),
            SiteError::SiteNotFound(identifier) => (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"message" :  "Site Not Found", "identifier" : identifier})),
            )
                .into_response(),
            SiteError::ContentNotFound(identifier) => (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"message" :  "Content Not Found", "identifier" : identifier})),
            )
                .into_response(),
            SiteError::XmlParsing(msg) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"message" :  "XML Parsing Error", "details" : msg})),
            )
                .into_response(),
            SiteError::BadRequest(msg) => (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({"message" :  "Invalid Input", "details" : msg})),
            )
                .into_response(),
            SiteError::MembershipNotFound(identifier) => (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"message" :  "Site Permission Membership Not Found", "identifier" : identifier})),
            )
                .into_response(),
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
            SiteError::Io(msg) => write!(f, "I/O error: {msg}"),
            SiteError::TeraTemplate(err) => write!(f, "template rendering error: {err}"),
            SiteError::SiteNotFound(identifier) => write!(f, "site not found: {identifier}"),
            SiteError::ContentNotFound(identifier) => write!(f, "content not found: {identifier}"),
            SiteError::XmlParsing(msg) => write!(f, "XML parsing error: {msg}"),
            SiteError::BadRequest(msg) => write!(f, "invalid input: {msg}"),
            SiteError::MembershipNotFound(identifier) => {
                write!(f, "membership not found: {identifier}")
            }
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

impl From<std::io::Error> for SiteError {
    fn from(err: std::io::Error) -> Self {
        SiteError::Io(err.to_string())
    }
}

impl From<tera::Error> for SiteError {
    fn from(err: tera::Error) -> Self {
        SiteError::TeraTemplate(err)
    }
}

impl From<quick_xml::Error> for SiteError {
    fn from(err: quick_xml::Error) -> Self {
        SiteError::XmlParsing(err.to_string())
    }
}
