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
    use tempfile::NamedTempFile;

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

    #[tokio::test]
    async fn dump_openapi_spec_writes_json_to_disk() {
        let temp_file = NamedTempFile::new().expect("failed to create temp file");
        let path = temp_file.path().to_path_buf();

        dump_openapi_spec(&path)
            .await
            .expect("failed to dump OpenAPI spec");

        let spec_contents = tokio::fs::read_to_string(&path)
            .await
            .expect("failed to read dumped OpenAPI spec");
        let spec: serde_json::Value =
            serde_json::from_str(&spec_contents).expect("failed to parse dumped OpenAPI spec");

        assert_eq!(spec["info"]["title"], env!("CARGO_PKG_NAME"));
        assert!(spec["paths"]["/api/site/{site_id}/content"].is_object());
    }
}
