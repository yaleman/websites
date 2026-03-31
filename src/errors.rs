use crate::web::state::AdminTemplateData;
use askama::Template;
use axum::http::StatusCode;
use axum::{
    Json,
    response::{Html, IntoResponse},
};
use sea_orm::DbErr;
use serde_json::json;
use uuid::Uuid;

#[derive(Template)]
#[template(path = "error.html")]
struct SiteErrorPage {
    status_code: axum::http::StatusCode,
    template_shared: AdminTemplateData,
    pub(crate) message: String,
}

impl SiteErrorPage {
    pub fn new(status_code: StatusCode, title: &str, message: &str) -> Self {
        SiteErrorPage {
            status_code,
            template_shared: AdminTemplateData::new(title.to_string()).with_hide_nav(true),
            message: message.to_string(),
        }
    }
}

impl IntoResponse for SiteErrorPage {
    fn into_response(self) -> axum::response::Response {
        (
            self.status_code,
            Html(
                self.render()
                    .unwrap_or_else(|_| "Error rendering error page".to_string()),
            ),
        )
            .into_response()
    }
}

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
        // TODO: use the error page template for all errors, not just unauthorized. For API routes, we can return JSON, but for regular web routes, we should return the error page.
        match self {
            SiteError::NotFound => (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response(),
            SiteError::Internal(msg) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(msg)).into_response()
            }
            SiteError::UnAuthorized(ref msg) => SiteErrorPage::new(StatusCode::UNAUTHORIZED, &self.to_string(), &format!("Please log in to access this page. {}", msg))
                .into_response(),
            SiteError::Database(error) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
            SiteError::Io(error) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
            SiteError::TeraTemplate(err) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("Template rendering error: {:?}", err)),
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
            SiteError::NotFound => write!(f, "Not Found"),
            SiteError::Internal(msg) => write!(f, "Internal Error: {msg}"),
            SiteError::UnAuthorized(msg) => write!(f, "Unauthorized: {msg}"),

            SiteError::Database(msg) => write!(f, "Database Error: {msg}"),
            SiteError::Io(msg) => write!(f, "I/O Error: {msg}"),
            SiteError::TeraTemplate(err) => write!(f, "Template Rendering Error: {err}"),
            SiteError::SiteNotFound(identifier) => write!(f, "Site Not Found: {identifier}"),
            SiteError::ContentNotFound(identifier) => write!(f, "Content Not Found: {identifier}"),
            SiteError::XmlParsing(msg) => write!(f, "XML Parsing Error: {msg}"),
            SiteError::BadRequest(msg) => write!(f, "Invalid Input: {msg}"),
            SiteError::MembershipNotFound(identifier) => {
                write!(f, "Membership Not Found: {identifier}")
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
