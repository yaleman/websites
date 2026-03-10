use crate::web::*;
use utoipa::OpenApi;

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
    tags((name = env!("CARGO_PKG_NAME"), description = env!("CARGO_PKG_DESCRIPTION")))
)]
pub struct ApiDoc;
