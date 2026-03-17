use std::path::PathBuf;

use crate::api::*;
use anyhow::Context;
use utoipa::OpenApi;
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

#[derive(OpenApi)]
#[openapi(
    paths(
        api_site_content_list,
        api_site_content_search,
        api_site_content_create,
        api_site_content_get,
        api_site_content_update,
        api_site_content_delete,
        api_site_assets_list,
        api_site_asset_create,
        api_site_asset_get,
        api_site_asset_delete,
        api_site_assets_library
    ),
    components(
        schemas(
            ApiErrorResponse,
            ApiCreateContentRequest,
            ApiUpdateContentRequest,
            ApiContentListResponse,
            ContentItemWithTags,
            AssetUploadRequest,
            ApiAssetListResponse,
            ApiAssetResponse,
            ApiAssetSummary,
            ApiAssetDetail,
            ApiAssetVariant,
            AssetLibraryResponse,
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

/// utility function to dump the OpenAPI spec to a file, useful for debugging and testing that the generated spec is valid and complete
/// ```rust
/// use websites::api_docs::dump_openapi_spec;
/// use tempfile::NamedTempFile;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let temp_file = NamedTempFile::new()?;
///     dump_openapi_spec(&temp_file.path().to_path_buf()).await?;
///     let spec_contents = std::fs::read_to_string(temp_file.path())?;
///     println!("Dumped OpenAPI spec:\n{}", spec_contents);
///     assert!(spec_contents.contains("websites"));
///     Ok(())
/// }
///
/// ```
pub async fn dump_openapi_spec(path: &PathBuf) -> anyhow::Result<()> {
    let spec = ApiDoc::openapi();
    let spec_json =
        serde_json::to_string_pretty(&spec).context("failed to serialize OpenAPI spec to JSON")?;
    tokio::fs::write(path, spec_json)
        .await
        .with_context(|| format!("failed to write OpenAPI spec to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_spec_includes_issue_4_paths() {
        let spec = ApiDoc::openapi();
        let paths = spec.paths.paths;

        assert!(paths.contains_key("/api/site/{site_id}/content"));
        assert!(paths.contains_key("/api/site/{site_id}/content/search"));
        assert!(paths.contains_key("/api/site/{site_id}/content/{content_id}"));
        assert!(paths.contains_key("/api/site/{site_id}/assets"));
        assert!(paths.contains_key("/api/site/{site_id}/assets/{asset_id}"));
        assert!(paths.contains_key("/api/site/{site_id}/assets/library"));
    }
}
