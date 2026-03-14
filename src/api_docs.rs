use crate::web::*;
use utoipa::OpenApi;
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

#[derive(OpenApi)]
#[openapi(
    paths(
        api_site_assets_library
    ),
    components(
        schemas(
            AssetLibraryItem
        )
    ),
    modifiers(&SecurityAddon),
    tags((name = env!("CARGO_PKG_NAME"), description = env!("CARGO_PKG_DESCRIPTION")))
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            );
        }
    }
}
