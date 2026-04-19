#![allow(clippy::disallowed_methods)]

use super::state::*;
use super::*;
use super::{content::*, sites::*};

use crate::constants::SESSION_USER;
use crate::db::test_db_start;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{DefaultBodyLimit, Multipart};
use axum::http::{Request, StatusCode, header};
use axum::routing::{get, post};
use sea_orm::{ActiveModelTrait, Set};
use serde::de::value::{Error as ValueError, StrDeserializer};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tower::ServiceExt;
use tower_sessions::{Expiry, MemoryStore, SessionManagerLayer};

const TINY_PNG_BYTES: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0xff,
    0x7f, 0x00, 0x09, 0xfb, 0x03, 0xfd, 0x28, 0xa6, 0xe3, 0x8a, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[tokio::test]
async fn ensure_site_owner_membership_is_idempotent() {
    let db = test_db_start().await;
    let site = crate::create_site(
        &db,
        "test".to_string(),
        "Test Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");

    let first = ensure_site_owner_membership(&db, "tester", None, site.id)
        .await
        .expect("failed to create membership");
    assert!(first.is_some(), "expected membership on first call");
    if let Some(membership) = first {
        assert_eq!(
            membership.role,
            SiteRole::Owner,
            "expected role to be owner"
        );
    }

    let second = ensure_site_owner_membership(&db, "tester", None, site.id)
        .await
        .expect("failed to check membership");
    assert!(second.is_none(), "expected no membership on second call");
}

#[test]
fn rewrite_preview_asset_urls_rewrites_root_asset_paths() {
    let site_id = Uuid::nil();
    let rendered = rewrite_preview_asset_urls(
        r#"<link href="/assets/style.css"><style>body{background:url('/assets/bg.png')}</style>"#,
        site_id,
    );

    assert!(
        rendered
            .contains("/admin/site/00000000-0000-0000-0000-000000000000/preview-assets/style.css")
    );
    assert!(
        rendered.contains("/admin/site/00000000-0000-0000-0000-000000000000/preview-assets/bg.png")
    );
}

#[test]
fn sanitize_preview_asset_path_rejects_parent_components() {
    assert!(sanitize_preview_asset_path("../secret.txt").is_err());
    assert!(sanitize_preview_asset_path("/etc/passwd").is_err());
}

#[test]
fn sanitize_preview_asset_path_allows_nested_relative_paths() {
    let path = sanitize_preview_asset_path("css/site/style.css")
        .expect("expected nested preview asset path to be accepted");

    assert_eq!(path, PathBuf::from("css/site/style.css"));
}

#[test]
fn role_satisfies_enforces_site_role_hierarchy() {
    assert!(role_satisfies(SiteRole::Viewer, SiteRole::Viewer));
    assert!(role_satisfies(SiteRole::Author, SiteRole::Viewer));
    assert!(role_satisfies(SiteRole::Editor, SiteRole::Author));
    assert!(role_satisfies(SiteRole::Owner, SiteRole::Editor));
    assert!(role_satisfies(SiteRole::Admin, SiteRole::Owner));

    assert!(!role_satisfies(SiteRole::Viewer, SiteRole::Author));
    assert!(!role_satisfies(SiteRole::Author, SiteRole::Editor));
    assert!(!role_satisfies(SiteRole::Editor, SiteRole::Owner));
}

#[test]
fn sort_content_items_orders_titles_descending_case_insensitively() {
    fn content(title: &str, page_type: PageType) -> entities::content_item::Model {
        entities::content_item::Model {
            id: Uuid::now_v7(),
            site_id: Uuid::now_v7(),
            page_type,
            title: title.to_string(),
            slug: title.to_lowercase().replace(' ', "-"),
            page_content: String::new(),
            draft: true,
            creator_sub: "tester".to_string(),
            created_at: DateTime::parse_from_rfc3339("2026-03-09T00:00:00Z")
                .expect("invalid created_at")
                .with_timezone(&Utc),
            last_updated: None,
            published_at: None,
        }
    }

    let mut items = vec![
        content("alpha page", PageType::Page),
        content("Zulu post", PageType::Post),
        content("Beta page", PageType::Page),
    ];

    sort_content_items(&mut items, ContentListSortBy::TitleDesc);

    let titles = items.into_iter().map(|item| item.title).collect::<Vec<_>>();
    assert_eq!(titles, vec!["Zulu post", "Beta page", "alpha page"]);
}

fn test_asset(
    original_filename: &str,
    byte_length: i32,
    created_at: &str,
) -> entities::asset::Model {
    entities::asset::Model {
        id: Uuid::now_v7(),
        site_id: Uuid::now_v7(),
        uploader_sub: "tester".to_string(),
        original_filename: original_filename.to_string(),
        storage_basename: format!("{}-stored", original_filename.to_lowercase()),
        mime_type: "image/png".to_string(),
        byte_length,
        width: Some(100),
        height: Some(80),
        created_at: DateTime::parse_from_rfc3339(created_at)
            .expect("invalid asset created_at")
            .with_timezone(&Utc),
    }
}

#[test]
fn asset_sort_parsing_defaults_and_fallbacks() {
    assert_eq!(AssetSortBy::from_query(None), AssetSortBy::Uploaded);
    assert_eq!(
        AssetSortDirection::from_query(None),
        AssetSortDirection::Desc
    );
    assert_eq!(
        AssetSortBy::from_query(Some("invalid")),
        AssetSortBy::Uploaded
    );
    assert_eq!(
        AssetSortDirection::from_query(Some("invalid")),
        AssetSortDirection::Desc
    );
}

#[test]
fn asset_sort_parsing_maps_supported_values() {
    assert_eq!(
        AssetSortBy::from_query(Some("uploaded")),
        AssetSortBy::Uploaded
    );
    assert_eq!(AssetSortBy::from_query(Some("size")), AssetSortBy::Size);
    assert_eq!(AssetSortBy::from_query(Some("name")), AssetSortBy::Name);
    assert_eq!(
        AssetSortDirection::from_query(Some("asc")),
        AssetSortDirection::Asc
    );
    assert_eq!(
        AssetSortDirection::from_query(Some("desc")),
        AssetSortDirection::Desc
    );
}

#[test]
fn sort_assets_orders_by_uploaded_in_both_directions() {
    let mut descending = vec![
        test_asset("charlie.png", 30, "2026-03-10T00:00:00Z"),
        test_asset("alpha.png", 10, "2026-03-11T00:00:00Z"),
        test_asset("bravo.png", 20, "2026-03-11T00:00:00Z"),
    ];
    sort_assets(
        &mut descending,
        AssetSortBy::Uploaded,
        AssetSortDirection::Desc,
    );
    assert_eq!(
        descending
            .iter()
            .map(|asset| asset.original_filename.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha.png", "bravo.png", "charlie.png"]
    );

    let mut ascending = vec![
        test_asset("charlie.png", 30, "2026-03-10T00:00:00Z"),
        test_asset("alpha.png", 10, "2026-03-11T00:00:00Z"),
        test_asset("bravo.png", 20, "2026-03-11T00:00:00Z"),
    ];
    sort_assets(
        &mut ascending,
        AssetSortBy::Uploaded,
        AssetSortDirection::Asc,
    );
    assert_eq!(
        ascending
            .iter()
            .map(|asset| asset.original_filename.as_str())
            .collect::<Vec<_>>(),
        vec!["charlie.png", "alpha.png", "bravo.png"]
    );
}

#[test]
fn sort_assets_orders_by_size_in_both_directions() {
    let mut ascending = vec![
        test_asset("bravo.png", 40, "2026-03-11T00:00:00Z"),
        test_asset("alpha.png", 40, "2026-03-10T00:00:00Z"),
        test_asset("charlie.png", 10, "2026-03-12T00:00:00Z"),
    ];
    sort_assets(&mut ascending, AssetSortBy::Size, AssetSortDirection::Asc);
    assert_eq!(
        ascending
            .iter()
            .map(|asset| asset.original_filename.as_str())
            .collect::<Vec<_>>(),
        vec!["charlie.png", "alpha.png", "bravo.png"]
    );

    let mut descending = vec![
        test_asset("bravo.png", 40, "2026-03-11T00:00:00Z"),
        test_asset("alpha.png", 40, "2026-03-10T00:00:00Z"),
        test_asset("charlie.png", 10, "2026-03-12T00:00:00Z"),
    ];
    sort_assets(&mut descending, AssetSortBy::Size, AssetSortDirection::Desc);
    assert_eq!(
        descending
            .iter()
            .map(|asset| asset.original_filename.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha.png", "bravo.png", "charlie.png"]
    );
}

#[test]
fn sort_assets_orders_by_name_case_insensitively() {
    let mut ascending = vec![
        test_asset("bravo.png", 20, "2026-03-10T00:00:00Z"),
        test_asset("Alpha.png", 10, "2026-03-11T00:00:00Z"),
        test_asset("alpha.png", 30, "2026-03-12T00:00:00Z"),
    ];
    sort_assets(&mut ascending, AssetSortBy::Name, AssetSortDirection::Asc);
    assert_eq!(
        ascending
            .iter()
            .map(|asset| asset.original_filename.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha.png", "Alpha.png", "bravo.png"]
    );

    let mut descending = vec![
        test_asset("bravo.png", 20, "2026-03-10T00:00:00Z"),
        test_asset("Alpha.png", 10, "2026-03-11T00:00:00Z"),
        test_asset("alpha.png", 30, "2026-03-12T00:00:00Z"),
    ];
    sort_assets(&mut descending, AssetSortBy::Name, AssetSortDirection::Desc);
    assert_eq!(
        descending
            .iter()
            .map(|asset| asset.original_filename.as_str())
            .collect::<Vec<_>>(),
        vec!["bravo.png", "alpha.png", "Alpha.png"]
    );
}

#[test]
fn build_search_rows_links_to_editor() {
    let site_id = Uuid::now_v7();
    let content_id = Uuid::now_v7();
    let rows = build_search_rows(
        vec![entities::content_item::Model {
            id: content_id,
            site_id,
            page_type: PageType::Page,
            title: "Search Result".to_string(),
            slug: "search-result".to_string(),
            page_content: "body".to_string(),
            draft: false,
            creator_sub: "tester".to_string(),
            created_at: Utc::now(),
            last_updated: None,
            published_at: None,
        }],
        &HashMap::from([(site_id, "Test Site".to_string())]),
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].edit_href,
        format!("/admin/site/{site_id}/content/{content_id}/edit")
    );
}

#[test]
fn content_list_status_filter_deserializes_expected_values() {
    let draft = ContentListStatusFilter::deserialize(StrDeserializer::<ValueError>::new("draft"))
        .expect("expected draft status filter to deserialize");
    let published =
        ContentListStatusFilter::deserialize(StrDeserializer::<ValueError>::new("published"))
            .expect("expected published status filter to deserialize");
    let all = ContentListStatusFilter::deserialize(StrDeserializer::<ValueError>::new("all"))
        .expect("expected all status filter to deserialize");

    assert_eq!(draft, ContentListStatusFilter::Draft);
    assert_eq!(published, ContentListStatusFilter::Published);
    assert_eq!(all, ContentListStatusFilter::All);
    assert!(
        ContentListStatusFilter::deserialize(StrDeserializer::<ValueError>::new("invalid"))
            .is_err(),
        "expected invalid status filter to fail deserialization"
    );
}

#[test]
fn content_list_href_preserves_non_default_status_filter() {
    let href = content_list_href(
        Uuid::nil(),
        ContentListPageTypeFilter::Page,
        ContentListStatusFilter::Draft,
        ContentListSortBy::TitleDesc,
    );

    assert_eq!(
        href,
        "/admin/site/00000000-0000-0000-0000-000000000000/content?sort_by=title_desc&page_type=page&status=draft"
    );
}

#[test]
fn content_list_href_omits_default_status_filter() {
    let href = content_list_href(
        Uuid::nil(),
        ContentListPageTypeFilter::All,
        ContentListStatusFilter::All,
        ContentListSortBy::CreatedDesc,
    );

    assert_eq!(
        href,
        "/admin/site/00000000-0000-0000-0000-000000000000/content?sort_by=created_desc"
    );
}

#[tokio::test]
async fn admin_assets_page_sorts_by_name_and_renders_selected_controls() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "asset-sort-page".to_string(),
        "Asset Sort Page".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let viewer = crate::entities::user::create_user(
        db.as_ref(),
        "asset-sort-viewer",
        Some("viewer@example.com"),
        Some("Viewer"),
        false,
    )
    .await
    .expect("failed to create viewer");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: viewer.id,
            role: SiteRole::Viewer,
        },
    )
    .await
    .expect("failed to create viewer membership");
    insert_test_asset(
        db.as_ref(),
        site.id,
        "charlie.png",
        300,
        "2026-03-10T00:00:00Z",
    )
    .await;
    insert_test_asset(
        db.as_ref(),
        site.id,
        "alpha.png",
        100,
        "2026-03-11T00:00:00Z",
    )
    .await;
    insert_test_asset(
        db.as_ref(),
        site.id,
        "bravo.png",
        200,
        "2026-03-12T00:00:00Z",
    )
    .await;

    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone()).router;
    let cookie = seed_session_cookie(test_admin_state(db.clone()), session_store, viewer.id).await;

    let response = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/admin/site/{}/assets?sort_by=name&sort_dir=asc",
                    site.id
                ))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build assets page request"),
        )
        .await
        .expect("failed to call assets page");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read assets page body");
    let body = String::from_utf8(body.to_vec()).expect("invalid assets page body");
    assert!(body.contains(r#"<option value="name" selected>"#));
    assert!(body.contains(r#"<option value="asc" selected>"#));

    let alpha_index = body.find("alpha.png").expect("missing alpha asset");
    let bravo_index = body.find("bravo.png").expect("missing bravo asset");
    let charlie_index = body.find("charlie.png").expect("missing charlie asset");
    assert!(alpha_index < bravo_index);
    assert!(bravo_index < charlie_index);
}

#[tokio::test]
async fn admin_dashboard_only_lists_sites_for_current_user() {
    let db = Arc::new(test_db_start().await);
    let visible_site = crate::create_site(
        db.as_ref(),
        "visible-site".to_string(),
        "Visible Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create visible site");
    let hidden_site = crate::create_site(
        db.as_ref(),
        "hidden-site".to_string(),
        "Hidden Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create hidden site");
    let viewer = crate::entities::user::create_user(
        db.as_ref(),
        "dashboard-viewer",
        Some("viewer@example.com"),
        Some("Viewer"),
        false,
    )
    .await
    .expect("failed to create dashboard viewer");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: visible_site.id,
            user_id: viewer.id,
            role: SiteRole::Viewer,
        },
    )
    .await
    .expect("failed to create dashboard membership");

    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone()).router;
    let cookie = seed_session_cookie(test_admin_state(db.clone()), session_store, viewer.id).await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/admin")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build dashboard request"),
        )
        .await
        .expect("failed to call dashboard");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read dashboard body");
    let body = String::from_utf8(body.to_vec()).expect("invalid dashboard body");
    assert!(body.contains(&visible_site.full_title));
    assert!(!body.contains(&hidden_site.full_title));
}

#[tokio::test]
async fn admin_dashboard_lists_all_sites_for_global_admin() {
    let db = Arc::new(test_db_start().await);
    let first_site = crate::create_site(
        db.as_ref(),
        "first-site".to_string(),
        "First Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create first site");
    let second_site = crate::create_site(
        db.as_ref(),
        "second-site".to_string(),
        "Second Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create second site");
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "dashboard-admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create dashboard admin");

    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone()).router;
    let cookie = seed_session_cookie(test_admin_state(db.clone()), session_store, admin.id).await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/admin")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build admin dashboard request"),
        )
        .await
        .expect("failed to call admin dashboard");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read admin dashboard body");
    let body = String::from_utf8(body.to_vec()).expect("invalid admin dashboard body");
    assert!(body.contains(&first_site.full_title));
    assert!(body.contains(&second_site.full_title));
}

#[tokio::test]
async fn admin_asset_upload_page_sorts_by_size_and_renders_selected_controls() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "asset-sort-upload".to_string(),
        "Asset Sort Upload".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let author = crate::entities::user::create_user(
        db.as_ref(),
        "asset-sort-author",
        Some("author@example.com"),
        Some("Author"),
        false,
    )
    .await
    .expect("failed to create author");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: author.id,
            role: SiteRole::Author,
        },
    )
    .await
    .expect("failed to create author membership");
    insert_test_asset(
        db.as_ref(),
        site.id,
        "large.png",
        300,
        "2026-03-10T00:00:00Z",
    )
    .await;
    insert_test_asset(
        db.as_ref(),
        site.id,
        "small.png",
        100,
        "2026-03-11T00:00:00Z",
    )
    .await;
    insert_test_asset(
        db.as_ref(),
        site.id,
        "medium.png",
        200,
        "2026-03-12T00:00:00Z",
    )
    .await;

    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone()).router;
    let cookie = seed_session_cookie(test_admin_state(db.clone()), session_store, author.id).await;

    let response = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/admin/site/{}/assets/new?sort_by=size&sort_dir=asc",
                    site.id
                ))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build asset upload page request"),
        )
        .await
        .expect("failed to call asset upload page");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read asset upload page body");
    let body = String::from_utf8(body.to_vec()).expect("invalid asset upload page body");
    assert!(body.contains("Browse Assets"));
    assert!(body.contains(r#"<option value="size" selected>"#));
    assert!(body.contains(r#"<option value="asc" selected>"#));

    let small_index = body.find("small.png").expect("missing small asset");
    let medium_index = body.find("medium.png").expect("missing medium asset");
    let large_index = body.find("large.png").expect("missing large asset");
    assert!(small_index < medium_index);
    assert!(medium_index < large_index);
}

#[tokio::test]
async fn admin_site_assets_create_accepts_multiple_files_and_redirects() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "asset-batch-create".to_string(),
        "Asset Batch Create".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let author = crate::entities::user::create_user(
        db.as_ref(),
        "asset-batch-author",
        Some("author@example.com"),
        Some("Author"),
        false,
    )
    .await
    .expect("failed to create author");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: author.id,
            role: SiteRole::Author,
        },
    )
    .await
    .expect("failed to create author membership");

    let session_store = MemoryStore::default();
    let test_router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        test_router.session_store.clone(),
        author.id,
    )
    .await;
    let (boundary, body) = multipart_asset_upload_request_body(
        &[
            ("first.png", "image/png", TINY_PNG_BYTES),
            ("second.png", "image/png", TINY_PNG_BYTES),
        ],
        None,
    );

    let response = test_router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/site/{}/assets/new", site.id))
                .header(header::COOKIE, cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build asset create request"),
        )
        .await
        .expect("failed to call asset create route");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .expect("missing location header")
            .to_str()
            .expect("invalid location header"),
        format!("/admin/site/{}/assets", site.id)
    );

    let assets = crate::list_assets(db.as_ref(), site.id)
        .await
        .expect("failed to list created assets");
    assert_eq!(assets.len(), 2);
    assert!(
        assets
            .iter()
            .any(|asset| asset.original_filename == "first.png")
    );
    assert!(
        assets
            .iter()
            .any(|asset| asset.original_filename == "second.png")
    );

    for asset in assets {
        assert!(
            test_router
                .upload_root
                .path()
                .join(&asset.storage_basename)
                .exists(),
            "expected uploaded asset file to exist"
        );
    }
}

#[tokio::test]
async fn admin_site_assets_create_rejects_mixed_file_and_source_url() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "asset-mixed-upload".to_string(),
        "Asset Mixed Upload".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let author = crate::entities::user::create_user(
        db.as_ref(),
        "asset-mixed-author",
        Some("author@example.com"),
        Some("Author"),
        false,
    )
    .await
    .expect("failed to create author");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: author.id,
            role: SiteRole::Author,
        },
    )
    .await
    .expect("failed to create author membership");

    let session_store = MemoryStore::default();
    let test_router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        test_router.session_store.clone(),
        author.id,
    )
    .await;
    let (boundary, body) = multipart_asset_upload_request_body(
        &[("first.png", "image/png", TINY_PNG_BYTES)],
        Some("https://example.com/image.png"),
    );

    let response = test_router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/site/{}/assets/new", site.id))
                .header(header::COOKIE, cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build mixed asset create request"),
        )
        .await
        .expect("failed to call mixed asset create route");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read mixed asset create body");
    let body: serde_json::Value =
        serde_json::from_slice(&body).expect("mixed asset create body should be json");
    assert_eq!(body["message"], "Invalid Input");
    assert_eq!(
        body["details"],
        "provide uploaded files or an image url, not both"
    );

    let assets = crate::list_assets(db.as_ref(), site.id)
        .await
        .expect("failed to list assets after rejected upload");
    assert!(assets.is_empty());
}

#[tokio::test]
async fn admin_site_assets_create_accepts_batches_larger_than_axum_default_limit() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "asset-large-batch-upload".to_string(),
        "Asset Large Batch Upload".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let author = crate::entities::user::create_user(
        db.as_ref(),
        "asset-large-batch-author",
        Some("author@example.com"),
        Some("Author"),
        false,
    )
    .await
    .expect("failed to create author");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: author.id,
            role: SiteRole::Author,
        },
    )
    .await
    .expect("failed to create author membership");

    let session_store = MemoryStore::default();
    let test_router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        test_router.session_store.clone(),
        author.id,
    )
    .await;
    let first_file = vec![0_u8; 1024 * 1024 + 256 * 1024];
    let second_file = vec![1_u8; 1024 * 1024 + 256 * 1024];
    let files = vec![
        ("first.png", "image/png", first_file.as_slice()),
        ("second.png", "image/png", second_file.as_slice()),
    ];
    let (boundary, body) = multipart_asset_upload_request_body(&files, None);

    let response = test_router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/site/{}/assets/new", site.id))
                .header(header::COOKIE, cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build large asset create request"),
        )
        .await
        .expect("failed to call large asset create route");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let assets = crate::list_assets(db.as_ref(), site.id)
        .await
        .expect("failed to list created large assets");
    assert_eq!(assets.len(), 2);
}

#[tokio::test]
async fn asset_upload_parser_returns_payload_too_large_for_limited_routes() {
    async fn limited_upload(multipart: Multipart) -> Result<StatusCode, SiteError> {
        crate::web::assets::parse_asset_create_upload(multipart).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    let router = Router::new().route(
        "/upload",
        post(limited_upload).layer(DefaultBodyLimit::max(64)),
    );
    let large_file = vec![0_u8; 512];
    let files = vec![("first.png", "image/png", large_file.as_slice())];
    let (boundary, body) = multipart_asset_upload_request_body(&files, None);

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build limited upload request"),
        )
        .await
        .expect("failed to call limited upload route");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read limited upload body");
    let body: serde_json::Value =
        serde_json::from_slice(&body).expect("limited upload body should be json");
    assert_eq!(body["message"], "Payload Too Large");
    assert_eq!(
        body["details"],
        "asset upload exceeded the 50 MB upload limit"
    );
}

#[tokio::test]
async fn asset_upload_batch_cleanup_removes_written_files_after_transaction_failure() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "asset-batch-failure".to_string(),
        "Asset Batch Failure".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let upload_root = tempfile::tempdir().expect("failed to create upload root");
    let txn = db.begin().await.expect("failed to start transaction");
    let mut cleanup_filenames = HashSet::new();

    let asset = crate::web::assets::store_uploaded_asset_with_audit(
        &txn,
        crate::web::assets::AssetUploadAuditContext {
            upload_root: upload_root.path(),
            site_id: site.id,
            actor_sub: "asset-failure-author",
            event_type: "create_asset",
        },
        crate::web::assets::UploadedAssetFile {
            bytes: TINY_PNG_BYTES.to_vec(),
            original_filename: "first.png".to_string(),
            mime_type: Some("image/png".to_string()),
        },
        &mut cleanup_filenames,
    )
    .await
    .expect("failed to store first asset inside transaction");

    assert!(
        upload_root.path().join(&asset.storage_basename).exists(),
        "expected uploaded asset file to exist before cleanup"
    );

    crate::web::assets::cleanup_uploaded_files(upload_root.path(), &cleanup_filenames)
        .await
        .expect("failed to clean up uploaded files");

    drop(txn);

    let assets = crate::list_assets(db.as_ref(), site.id)
        .await
        .expect("failed to list assets after transaction rollback");
    assert!(assets.is_empty());

    let upload_entries = std::fs::read_dir(upload_root.path())
        .expect("failed to read upload root")
        .count();
    assert_eq!(upload_entries, 0);
}

#[tokio::test]
async fn can_view_user_profile_allows_self_and_admin_only() {
    let db = test_db_start().await;
    let viewer = crate::entities::user::create_user(&db, "viewer", None, None, false)
        .await
        .expect("failed to create viewer");
    let target = crate::entities::user::create_user(&db, "target", None, None, false)
        .await
        .expect("failed to create target");
    let admin = crate::entities::user::create_user(&db, "admin", None, None, true)
        .await
        .expect("failed to create admin");

    assert!(can_view_user_profile(&viewer, &viewer));
    assert!(!can_view_user_profile(&viewer, &target));
    assert!(can_view_user_profile(&admin, &target));
}

async fn test_login(
    State(state): State<AdminState>,
    session: Session,
    Path(user_id): Path<Uuid>,
) -> Result<StatusCode, SiteError> {
    let user = get_user_by_id(state.db.as_ref(), user_id)
        .await?
        .ok_or(SiteError::NotFound)?;
    session
        .insert(SESSION_USER, user)
        .await
        .map_err(|_| SiteError::internal("failed to seed session".to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn seed_session_cookie(
    state: AdminState,
    session_store: MemoryStore,
    user_id: Uuid,
) -> String {
    let router = Router::new()
        .route("/test-login/{user_id}", get(test_login))
        .layer(
            SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_expiry(Expiry::OnSessionEnd),
        )
        .with_state(state);
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/test-login/{user_id}"))
                .body(Body::empty())
                .expect("failed to build login request"),
        )
        .await
        .expect("failed to perform login request");
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .next()
        .expect("missing set-cookie header")
        .to_str()
        .expect("invalid set-cookie header");
    cookie
        .split(';')
        .next()
        .expect("missing cookie pair")
        .to_string()
}

fn copy_dir_recursive(source: &StdPath, target: &StdPath) {
    std::fs::create_dir_all(target).expect("failed to create template fixture target");
    for entry in std::fs::read_dir(source).expect("failed to read template fixture source") {
        let entry = entry.expect("failed to read template fixture entry");
        let entry_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_recursive(&entry_path, &target_path);
        } else {
            std::fs::copy(&entry_path, &target_path).expect("failed to copy template fixture file");
        }
    }
}

fn test_site_templates_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("failed to create temp template root");
    let default_source = crate::site_template_dir_candidates(
        &crate::resolve_site_templates_root(),
        DEFAULT_TEMPLATE_NAME,
    )
    .into_iter()
    .find(|path| path.exists())
    .expect("expected a bundled default site template directory");
    let default_target = root.path().join("default");
    copy_dir_recursive(&default_source, &default_target);
    root
}

fn test_admin_state(db: std::sync::Arc<DatabaseConnection>) -> AdminState {
    let jwt_signer = signer_from_secret(&token_auth::JwtHs256SecretSetting {
        secret_bytes: vec![7; 32],
    })
    .expect("failed to build test jwt signer");
    AdminState {
        db,
        oidc_client_id: ClientId::new("client".to_string()),
        oidc_client_secret: None,
        oidc_frontend_url: Url::parse("https://example.com").expect("invalid frontend url"),
        oidc_discovery_url: IssuerUrl::new("https://example.com".to_string())
            .expect("invalid discovery url"),
        oidc_client: std::sync::Arc::new(
            build_http_client().expect("failed to build test oidc client"),
        ),
        jwt_signer: Arc::new(jwt_signer),
        jwt_issuer: "https://example.com".to_string(),
        upload_root: std::env::temp_dir().join(format!("websites-test-{}", Uuid::now_v7())),
        log_path: std::env::temp_dir()
            .join(format!("websites-logs-test-{}", Uuid::now_v7()))
            .join(crate::constants::LOG_FILE_NAME),
        site_templates_root: std::env::temp_dir()
            .join(format!("websites-templates-test-{}", Uuid::now_v7())),
        rendered_root: std::env::temp_dir()
            .join(format!("websites-rendered-test-{}", Uuid::now_v7())),
    }
}

#[cfg(unix)]
struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

#[cfg(unix)]
impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: this test restores the variable in Drop and runs in isolation.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

#[cfg(unix)]
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: this restores the prior process environment value for the test.
        unsafe {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

#[cfg(unix)]
fn write_fake_rsync_binary(path: &StdPath) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail
source_dir="${@: -2:1}"
destination="${@: -1}"
source_dir="${source_dir%/}"
remote_path="${destination#*:}"
remote_path="${remote_path%/}"
mkdir -p "$remote_path"
find "$remote_path" -mindepth 1 -exec rm -rf {} +
cp -R "$source_dir"/. "$remote_path"/
transferred=$(find "$source_dir" -type f | wc -l | tr -d ' ')
printf 'Number of files transferred: %s\n' "$transferred"
printf 'Number of deleted files: 1\n'
"#;
    std::fs::write(path, script).expect("write fake rsync binary");
    let mut permissions = std::fs::metadata(path)
        .expect("fake rsync metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod fake rsync binary");
}

#[cfg(unix)]
async fn write_fake_rsync_identity_file(root: &StdPath) -> PathBuf {
    let path = root.join("id_ed25519");
    tokio::fs::write(&path, b"fake-identity")
        .await
        .expect("write fake identity file");
    path
}

fn write_test_log_file(log_path: &StdPath, contents: &str) {
    std::fs::create_dir_all(log_path.parent().expect("test log path missing parent"))
        .expect("failed to create test log root");
    std::fs::write(log_path, contents).expect("failed to write test log file");
}

async fn insert_test_asset(
    db: &DatabaseConnection,
    site_id: Uuid,
    original_filename: &str,
    byte_length: i32,
    created_at: &str,
) -> entities::asset::Model {
    entities::asset::ActiveModel {
        id: Set(Uuid::now_v7()),
        site_id: Set(site_id),
        uploader_sub: Set("tester".to_string()),
        original_filename: Set(original_filename.to_string()),
        storage_basename: Set(format!("{}-{}", Uuid::now_v7(), original_filename)),
        mime_type: Set("image/png".to_string()),
        byte_length: Set(byte_length),
        width: Set(Some(100)),
        height: Set(Some(80)),
        created_at: Set(DateTime::parse_from_rfc3339(created_at)
            .expect("invalid asset created_at")
            .with_timezone(&Utc)),
    }
    .insert(db)
    .await
    .expect("failed to insert test asset")
}

async fn insert_test_publish_run(
    db: &DatabaseConnection,
    site_id: Uuid,
    run_id: Uuid,
) -> entities::site_publish_run::Model {
    let now = Utc::now();
    entities::site_publish_run::ActiveModel {
        id: Set(run_id),
        site_id: Set(site_id),
        method: Set(entities::site_publish_config::PublishMethod::S3Compatible),
        status: Set(entities::site_publish_run::PublishRunStatus::Succeeded),
        actor_sub: Set("admin".to_string()),
        rendered_file_count: Set(3),
        published_file_count: Set(3),
        deleted_object_count: Set(1),
        error_message: Set(Option::<String>::None),
        created_at: Set(now),
        started_at: Set(Some(now)),
        finished_at: Set(Some(now)),
    }
    .insert(db)
    .await
    .expect("failed to insert publish run")
}

#[tokio::test]
async fn admin_logs_view_shows_filtered_entries() {
    let db = Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let state = test_admin_state(db.clone());
    write_test_log_file(
        &state.log_path,
        concat!(
            "2026-03-27T03:45:56.170588Z INFO websites::publish: starting site publish site_id=123 bucket=example prefix=site endpoint_url=aws-default\n",
            "2026-03-27T03:45:56.171588Z ERROR websites::publish: publish job failed run_id=123 site_id=123 site_short_name=demo bucket=example prefix=site endpoint_url=aws-default error=dispatch failure\n",
        ),
    );

    let session_store = MemoryStore::default();
    let router = test_app_router(state, session_store.clone()).router;
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/admin/logs?level=error&q=publish&limit=50")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build logs request"),
        )
        .await
        .expect("failed to call logs route");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read logs body");
    let body = String::from_utf8(body.to_vec()).expect("invalid logs response body");
    assert!(body.contains("publish job failed"));
    assert!(!body.contains("starting site publish"));
    assert!(body.contains("dispatch failure"));
}

#[tokio::test]
async fn publish_run_id_links_to_run_detail_view() {
    let db = Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let site = crate::create_site(
        db.as_ref(),
        "publish-detail".to_string(),
        "Publish Detail".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let run_id = Uuid::now_v7();
    let _run = insert_test_publish_run(db.as_ref(), site.id, run_id).await;

    let state = test_admin_state(db.clone());
    write_test_log_file(
        &state.log_path,
        &format!(
            "2026-03-27T03:45:56.170588Z INFO websites::publish: publish job queued run_id={run_id} site_id={} site_short_name=detail bucket=example prefix=site endpoint_url=aws-default\n2026-03-27T03:45:56.171588Z INFO websites::publish: starting site publish run_id={run_id} site_id={} site_short_name=detail bucket=example prefix=site endpoint_url=aws-default\n2026-03-27T03:45:56.172588Z INFO websites::publish: site publish completed run_id={run_id} site_id={} site_short_name=detail rendered_file_count=3 published_file_count=3 deleted_object_count=1\n",
            site.id, site.id, site.id
        ),
    );

    let session_store = MemoryStore::default();
    let router = test_app_router(state, session_store.clone()).router;
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;

    let publish_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/publish", site.id))
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .expect("failed to build publish request"),
        )
        .await
        .expect("failed to call publish page");
    assert_eq!(publish_response.status(), StatusCode::OK);
    let publish_body = to_bytes(publish_response.into_body(), usize::MAX)
        .await
        .expect("failed to read publish body");
    let publish_body = String::from_utf8(publish_body.to_vec()).expect("invalid publish body");
    let run_link = format!("/admin/site/{}/publish/run/{run_id}", site.id);
    assert!(publish_body.contains(&run_link));

    let detail_response = router
        .oneshot(
            Request::builder()
                .uri(&run_link)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build run detail request"),
        )
        .await
        .expect("failed to call run detail page");
    assert_eq!(detail_response.status(), StatusCode::OK);
    let detail_body = to_bytes(detail_response.into_body(), usize::MAX)
        .await
        .expect("failed to read run detail body");
    let detail_body = String::from_utf8(detail_body.to_vec()).expect("invalid run detail body");
    assert!(detail_body.contains("Publish Run"));
    assert!(detail_body.contains(&run_id.to_string()));
    assert!(detail_body.contains("publish job queued"));
    assert!(detail_body.contains("site publish completed"));
}

#[tokio::test]
async fn publish_settings_can_be_disabled_from_the_method_selector() {
    let db = Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let site = crate::create_site(
        db.as_ref(),
        "publish-disable".to_string(),
        "Publish Disable".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");

    save_s3_publish_config(
        db.as_ref(),
        site.id,
        S3CompatiblePublishConfig {
            endpoint_url: None,
            bucket: "bucket".to_string(),
            prefix: "site".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: "key".to_string(),
            secret_access_key: "secret".to_string(),
            force_path_style: false,
        },
    )
    .await
    .expect("failed to seed publish config");

    let state = test_admin_state(db.clone());
    let session_store = MemoryStore::default();
    let router = test_app_router(state, session_store.clone()).router;
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;

    let page_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/publish", site.id))
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .expect("failed to build publish page request"),
        )
        .await
        .expect("failed to load publish page");
    assert_eq!(page_response.status(), StatusCode::OK);
    let page_body = to_bytes(page_response.into_body(), usize::MAX)
        .await
        .expect("failed to read publish page body");
    let page_body = String::from_utf8(page_body.to_vec()).expect("invalid publish page body");
    assert!(!page_body.contains("Delete configuration"));

    let csrf_token = page_body
        .split("name=\"csrf_token\" value=\"")
        .nth(1)
        .expect("missing publish csrf token")
        .split('"')
        .next()
        .expect("missing publish csrf token value");

    let encoded_csrf_token =
        url::form_urlencoded::byte_serialize(csrf_token.as_bytes()).collect::<String>();

    let disable_response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/site/{}/publish", site.id))
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!(
                        "csrf_token={encoded_csrf_token}&method=disabled&endpoint_url=&bucket=&prefix=&region=&access_key_id=&secret_access_key=&force_path_style=&ssh_host=&ssh_user=&ssh_port=&remote_path=&identity_file="
                    )))
                    .expect("failed to build disable request"),
            )
            .await
            .expect("failed to submit disable form");

    assert_eq!(disable_response.status(), StatusCode::SEE_OTHER);
    let location = disable_response
        .headers()
        .get(header::LOCATION)
        .expect("missing redirect location")
        .to_str()
        .expect("invalid redirect location");
    assert!(location.ends_with(&format!("/admin/site/{}/publish?disabled=1", site.id)));

    let saved_config = get_s3_publish_config(db.as_ref(), site.id)
        .await
        .expect("load publish config");
    assert!(saved_config.is_none(), "publish config should be cleared");
}

#[cfg(unix)]
#[tokio::test]
async fn publish_settings_support_rsync_ssh_and_queue_publish() {
    let _env_lock = crate::test_support::env_lock().lock_owned().await;
    let db = Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin-rsync",
        Some("admin-rsync@example.com"),
        Some("Admin Rsync"),
        true,
    )
    .await
    .expect("failed to create admin");
    let site = crate::create_site(
        db.as_ref(),
        "publish-rsync".to_string(),
        "Publish Rsync".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");

    let script_root = tempfile::tempdir().expect("failed to create script root");
    let remote_root = tempfile::tempdir().expect("failed to create remote root");
    let fake_rsync = script_root.path().join("fake-rsync.sh");
    write_fake_rsync_binary(&fake_rsync);
    let identity_file = write_fake_rsync_identity_file(script_root.path()).await;
    let _guard = EnvVarGuard::set(
        "WEBSITES_RSYNC_BIN",
        fake_rsync.to_str().expect("fake rsync path"),
    );

    let state = test_admin_state(db.clone());
    let session_store = MemoryStore::default();
    let router = test_app_router(state, session_store.clone()).router;
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;
    let encoded_remote_path = url::form_urlencoded::byte_serialize(
        remote_root.path().to_str().expect("remote root").as_bytes(),
    )
    .collect::<String>();

    let page_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/publish", site.id))
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .expect("failed to build publish page request"),
        )
        .await
        .expect("failed to load publish page");
    assert_eq!(page_response.status(), StatusCode::OK);
    let page_body = to_bytes(page_response.into_body(), usize::MAX)
        .await
        .expect("failed to read publish page body");
    let page_body = String::from_utf8(page_body.to_vec()).expect("invalid publish page body");
    let csrf_token = page_body
        .split("name=\"csrf_token\" value=\"")
        .nth(1)
        .expect("missing publish csrf token")
        .split('"')
        .next()
        .expect("missing publish csrf token value");
    let encoded_csrf_token =
        url::form_urlencoded::byte_serialize(csrf_token.as_bytes()).collect::<String>();

    let save_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/site/{}/publish", site.id))
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!(
                        "csrf_token={encoded_csrf_token}&method=rsync_ssh&endpoint_url=&bucket=&prefix=&region=&access_key_id=&secret_access_key=&force_path_style=&ssh_host=publish.example.com&ssh_user=deploy&ssh_port=2222&remote_path={encoded_remote_path}&identity_file={}",
                        url::form_urlencoded::byte_serialize(
                            identity_file.to_string_lossy().as_bytes()
                        )
                        .collect::<String>()
                    )))
                    .expect("failed to build rsync save request"),
            )
            .await
            .expect("failed to submit rsync config");
    assert_eq!(save_response.status(), StatusCode::SEE_OTHER);
    let save_location = save_response
        .headers()
        .get(header::LOCATION)
        .expect("missing save redirect")
        .to_str()
        .expect("invalid save redirect");
    assert!(save_location.ends_with(&format!("/admin/site/{}/publish?saved=1", site.id)));

    let publish_page_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/publish", site.id))
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .expect("failed to build refreshed publish page request"),
        )
        .await
        .expect("failed to reload publish page");
    let publish_page_body = to_bytes(publish_page_response.into_body(), usize::MAX)
        .await
        .expect("failed to read refreshed publish page body");
    let publish_page_body =
        String::from_utf8(publish_page_body.to_vec()).expect("invalid refreshed publish page body");
    let publish_csrf_token = publish_page_body
        .split(&format!("action=\"/admin/site/{}/publish/run\"", site.id))
        .nth(1)
        .expect("missing publish run form")
        .split("name=\"csrf_token\" value=\"")
        .nth(1)
        .expect("missing publish csrf token")
        .split('"')
        .next()
        .expect("missing publish csrf token value");
    let encoded_publish_csrf_token =
        url::form_urlencoded::byte_serialize(publish_csrf_token.as_bytes()).collect::<String>();

    let run_response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/site/{}/publish/run", site.id))
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "csrf_token={encoded_publish_csrf_token}"
                )))
                .expect("failed to build publish run request"),
        )
        .await
        .expect("failed to queue publish run");
    assert_eq!(run_response.status(), StatusCode::SEE_OTHER);

    let run = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(run) = list_site_publish_runs(db.as_ref(), site.id, 1)
                .await
                .expect("load run")
                .into_iter()
                .next()
            {
                match run.status {
                    entities::site_publish_run::PublishRunStatus::Succeeded => break run,
                    entities::site_publish_run::PublishRunStatus::Failed => {
                        panic!(
                            "publish run failed early: {}",
                            run.error_message
                                .unwrap_or_else(|| "missing error message".to_string())
                        );
                    }
                    _ => {}
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("timed out waiting for publish run");

    assert_eq!(run.method, PublishMethod::RsyncSsh);
    assert_eq!(
        run.status,
        entities::site_publish_run::PublishRunStatus::Succeeded
    );
    assert_eq!(run.rendered_file_count, run.published_file_count);
}

#[cfg(unix)]
#[tokio::test]
async fn render_site_auto_publishes_when_enabled() {
    let _env_lock = crate::test_support::env_lock().lock_owned().await;
    let db = Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin-auto-publish",
        Some("admin-auto-publish@example.com"),
        Some("Admin Auto Publish"),
        true,
    )
    .await
    .expect("failed to create admin");
    let site = crate::create_site(
        db.as_ref(),
        "render-auto-publish".to_string(),
        "Render Auto Publish".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let script_root = tempfile::tempdir().expect("failed to create script root");
    let remote_root = tempfile::tempdir().expect("failed to create remote root");
    let fake_rsync = script_root.path().join("fake-rsync.sh");
    write_fake_rsync_binary(&fake_rsync);
    let identity_file = write_fake_rsync_identity_file(script_root.path()).await;
    let _guard = EnvVarGuard::set(
        "WEBSITES_RSYNC_BIN",
        fake_rsync.to_str().expect("fake rsync path"),
    );

    crate::publish::save_rsync_publish_config(
        db.as_ref(),
        site.id,
        RsyncPublishConfig {
            ssh_host: "publish.example.com".to_string(),
            ssh_user: Some("deploy".to_string()),
            ssh_port: Some(2222),
            remote_path: remote_root
                .path()
                .to_str()
                .expect("remote root path")
                .to_string(),
            identity_file: Some(identity_file.to_string_lossy().to_string()),
        },
    )
    .await
    .expect("failed to save rsync publish config");

    crate::update_site_settings(
        db.as_ref(),
        site.id,
        site.full_title.clone(),
        site.template_name.clone(),
        true,
    )
    .await
    .expect("failed to enable publish on render");

    let state = test_admin_state(db.clone());
    let session_store = MemoryStore::default();
    let router = test_app_router(state, session_store.clone()).router;
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;

    let render_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/render", site.id))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build render request"),
        )
        .await
        .expect("failed to render site");
    assert_eq!(render_response.status(), StatusCode::OK);
    let render_body = to_bytes(render_response.into_body(), usize::MAX)
        .await
        .expect("failed to read render body");
    let render_body = String::from_utf8(render_body.to_vec()).expect("invalid render body");
    assert!(render_body.contains("Site rendered with"));
    assert!(render_body.contains("Published "));

    let run = list_site_publish_runs(db.as_ref(), site.id, 1)
        .await
        .expect("load publish run")
        .into_iter()
        .next()
        .expect("missing publish run");
    assert_eq!(run.method, PublishMethod::RsyncSsh);
    assert_eq!(
        run.status,
        entities::site_publish_run::PublishRunStatus::Succeeded
    );
    assert_eq!(run.rendered_file_count, run.published_file_count);
}

#[tokio::test]
async fn admin_logs_view_blocks_non_admins() {
    let db = Arc::new(test_db_start().await);
    let viewer = crate::entities::user::create_user(
        db.as_ref(),
        "viewer",
        Some("viewer@example.com"),
        Some("Viewer"),
        false,
    )
    .await
    .expect("failed to create viewer");
    let state = test_admin_state(db.clone());
    write_test_log_file(
        &state.log_path,
        "2026-03-27T03:45:56.170588Z INFO websites::publish: starting site publish\n",
    );

    let session_store = MemoryStore::default();
    let router = test_app_router(state, session_store.clone()).router;
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        viewer.id,
    )
    .await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/admin/logs")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build logs request"),
        )
        .await
        .expect("failed to call logs route");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

fn site_transfer_test_router(state: AdminState, session_store: MemoryStore) -> Router {
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnSessionEnd);
    let protected = Router::new()
        .route(
            "/admin/sites/import",
            get(admin_sites_import).post(admin_sites_import_create),
        )
        .route("/admin/sites/import/check", get(admin_sites_import_check))
        .route("/admin/site/{site_id}/export.json", get(admin_site_export))
        .layer(from_fn(crate::middleware::require_session));

    Router::new()
        .route("/test-login/{user_id}", get(test_login))
        .merge(protected)
        .layer(session_layer)
        .with_state(state)
}

pub(crate) struct TestRouter {
    pub router: Router,
    #[allow(dead_code)]
    /// These are kept around for lifecycle reasons
    assets_dir: tempfile::TempDir,
    #[allow(dead_code)]
    /// These are kept around for lifecycle reasons
    upload_root: tempfile::TempDir,
    #[allow(dead_code)]
    /// These are kept around for lifecycle reasons
    site_templates_root: tempfile::TempDir,
    #[allow(dead_code)]
    /// These are kept around for lifecycle reasons
    session_store: MemoryStore,
}

fn test_app_router(mut state: AdminState, session_store: MemoryStore) -> TestRouter {
    let session_layer = SessionManagerLayer::new(session_store.clone())
        .with_secure(false)
        .with_expiry(Expiry::OnSessionEnd);
    let assets_dir = tempfile::tempdir().expect("failed to create temp assets dir");
    let upload_root = tempfile::tempdir().expect("failed to create temp upload root");
    let site_templates_root = test_site_templates_root();
    state.upload_root = upload_root.path().to_path_buf();
    state.site_templates_root = site_templates_root.path().to_path_buf();

    let router = build_admin_app(state, assets_dir.path(), upload_root.path()).layer(session_layer);
    TestRouter {
        router,
        assets_dir,
        upload_root,
        site_templates_root,
        session_store,
    }
}

#[tokio::test]
async fn admin_site_preview_asset_uses_bundled_assets_when_configured_root_is_empty() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "preview-site".to_string(),
        "Preview Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let viewer = crate::entities::user::create_user(
        db.as_ref(),
        "viewer",
        Some("viewer@example.com"),
        Some("Viewer"),
        false,
    )
    .await
    .expect("failed to create viewer");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: viewer.id,
            role: SiteRole::Viewer,
        },
    )
    .await
    .expect("failed to create viewer membership");

    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store.clone())
        .with_secure(false)
        .with_expiry(Expiry::OnSessionEnd);
    let assets_dir = tempfile::tempdir().expect("failed to create temp assets dir");
    let upload_root = tempfile::tempdir().expect("failed to create temp upload root");
    let site_templates_root = tempfile::tempdir().expect("failed to create temp template root");
    let mut state = test_admin_state(db.clone());
    state.upload_root = upload_root.path().to_path_buf();
    state.site_templates_root = site_templates_root.path().to_path_buf();

    let router =
        build_admin_app(state.clone(), assets_dir.path(), upload_root.path()).layer(session_layer);
    let cookie = seed_session_cookie(state, session_store, viewer.id).await;
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/preview-assets/style.css", site.id))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build preview asset request"),
        )
        .await
        .expect("failed to call preview asset route");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read preview asset response");
    let css = String::from_utf8(body.to_vec()).expect("preview asset body should be utf-8");
    assert!(css.contains("font-family"));
}

fn multipart_json_request_body(json: &str) -> (String, Vec<u8>) {
    multipart_json_request_body_with_replace(json, false)
}

fn multipart_json_request_body_with_replace(
    json: &str,
    replace_existing: bool,
) -> (String, Vec<u8>) {
    let boundary = "site-import-boundary";
    let replace_field = if replace_existing {
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"replace_existing\"\r\n\r\n1\r\n"
        )
    } else {
        String::new()
    };
    let body = format!(
        "{replace_field}--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"site-export.json\"\r\nContent-Type: application/json\r\n\r\n{json}\r\n--{boundary}--\r\n"
    );
    (boundary.to_string(), body.into_bytes())
}

fn multipart_wordpress_xml_request_body(xml: &str) -> (String, Vec<u8>) {
    let boundary = "wordpress-import-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"wordpress.xml\"\r\nContent-Type: application/xml\r\n\r\n{xml}\r\n--{boundary}--\r\n"
    );
    (boundary.to_string(), body.into_bytes())
}

fn multipart_asset_upload_request_body(
    files: &[(&str, &str, &[u8])],
    source_url: Option<&str>,
) -> (String, Vec<u8>) {
    let boundary = "asset-upload-boundary";
    let mut body = Vec::new();

    for (filename, mime_type, bytes) in files {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }

    if let Some(source_url) = source_url {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"source_url\"\r\n\r\n{source_url}\r\n"
            )
            .as_bytes(),
        );
    }

    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    (boundary.to_string(), body)
}

fn git_command(dir: &StdPath, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Codex")
        .env("GIT_AUTHOR_EMAIL", "codex@example.com")
        .env("GIT_COMMITTER_NAME", "Codex")
        .env("GIT_COMMITTER_EMAIL", "codex@example.com")
        .args(args)
        .status()
        .expect("failed to run git command");
    assert!(status.success(), "git command failed: {:?}", args);
}

fn create_theme_repo(theme_content: &str) -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("failed to create theme repo");
    git_command(repo.path(), &["init", "-b", "main"]);
    std::fs::write(repo.path().join("theme.txt"), theme_content)
        .expect("failed to write theme file");
    git_command(repo.path(), &["add", "theme.txt"]);
    git_command(repo.path(), &["commit", "-m", "initial theme"]);
    repo
}

fn update_theme_repo(repo: &StdPath, theme_content: &str, message: &str) {
    std::fs::write(repo.join("theme.txt"), theme_content).expect("failed to update theme file");
    git_command(repo, &["add", "theme.txt"]);
    git_command(repo, &["commit", "-m", message]);
}

fn urlencoded_theme_form(repo_url: &str, slug: Option<&str>) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("repo_url", repo_url);
    if let Some(slug) = slug {
        serializer.append_pair("slug", slug);
    }
    serializer.finish()
}

#[tokio::test]
async fn health_check_is_public_and_returns_json_ok() {
    let db = test_db_start().await;
    let session_store = MemoryStore::default();
    let test_router = test_app_router(test_admin_state(db.into()), session_store);

    let response = test_router
        .router
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("failed to build health request"),
        )
        .await
        .expect("failed to call health route");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("missing content-type")
            .to_str()
            .expect("invalid content-type"),
        "application/json"
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read health response body");
    assert_eq!(body, "\"ok\"");
}

#[tokio::test]
async fn admin_themes_install_succeeds_for_global_admin() {
    let db = std::sync::Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let repo = create_theme_repo("version-one");
    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;
    let body = urlencoded_theme_form(
        repo.path().to_str().expect("repo path should be utf-8"),
        Some("sample-theme"),
    );

    let response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/themes")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .expect("failed to build install request"),
        )
        .await
        .expect("failed to call install route");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .expect("missing location header")
            .to_str()
            .expect("invalid location header"),
        "/admin/themes?installed=sample-theme"
    );

    let installed_file = router
        .site_templates_root
        .path()
        .join("sample-theme")
        .join("theme.txt");
    let installed_content =
        std::fs::read_to_string(&installed_file).expect("failed to read installed theme file");
    assert_eq!(installed_content, "version-one");

    let admin_page = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/sites/new")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build new site request"),
        )
        .await
        .expect("failed to load create-site page");
    assert_eq!(admin_page.status(), StatusCode::OK);
    let body = to_bytes(admin_page.into_body(), usize::MAX)
        .await
        .expect("failed to read create-site body");
    assert!(
        String::from_utf8_lossy(&body).contains("sample-theme"),
        "expected create-site page to list installed theme"
    );
}

#[tokio::test]
async fn admin_themes_page_uses_shared_admin_styles() {
    let db = std::sync::Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let repo = create_theme_repo("version-one");
    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;
    let install_body = urlencoded_theme_form(
        repo.path().to_str().expect("repo path should be utf-8"),
        Some("sample-theme"),
    );

    let install_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/themes")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(install_body))
                .expect("failed to build install request"),
        )
        .await
        .expect("failed to install theme");
    assert_eq!(install_response.status(), StatusCode::SEE_OTHER);

    let page = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/themes")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build themes request"),
        )
        .await
        .expect("failed to load themes page");
    assert_eq!(page.status(), StatusCode::OK);

    let body = to_bytes(page.into_body(), usize::MAX)
        .await
        .expect("failed to read themes page body");
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains(r#"class="surface grid gap-4 p-4 md:p-6""#),
        "expected theme management sections to use shared surface styling"
    );
    assert!(
        html.contains(r#"class="stack-form""#),
        "expected install form to use shared stacked form styling"
    );
    assert!(
        html.contains(r#"class="btn btn--danger""#),
        "expected destructive theme action to use danger button styling"
    );
    assert!(
        html.contains(r#"class="flex flex-wrap gap-2""#),
        "expected theme action buttons to stay compact inside the table"
    );
}

#[tokio::test]
async fn publish_site_nav_link_only_shows_when_publish_is_configured() {
    let db = std::sync::Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let site = crate::create_site(
        db.as_ref(),
        "publish-nav".to_string(),
        "Publish Nav Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");

    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;

    let disabled_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/content", site.id))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("failed to build content request"),
        )
        .await
        .expect("failed to load content page with publishing disabled");
    assert_eq!(disabled_response.status(), StatusCode::OK);
    let disabled_body = to_bytes(disabled_response.into_body(), usize::MAX)
        .await
        .expect("failed to read disabled content body");
    let disabled_html = String::from_utf8(disabled_body.to_vec()).expect("invalid disabled body");
    assert!(
        !disabled_html.contains(&format!("/admin/site/{}/render?publish=1", site.id)),
        "expected publish link to be hidden when no publish config exists"
    );

    crate::save_s3_publish_config(
        db.as_ref(),
        site.id,
        crate::S3CompatiblePublishConfig {
            endpoint_url: None,
            bucket: "example-bucket".to_string(),
            prefix: "site".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: "access-key".to_string(),
            secret_access_key: "secret-key".to_string(),
            force_path_style: false,
        },
    )
    .await
    .expect("failed to save publish config");

    let enabled_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/content", site.id))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("failed to build enabled content request"),
        )
        .await
        .expect("failed to load content page with publishing enabled");
    assert_eq!(enabled_response.status(), StatusCode::OK);
    let enabled_body = to_bytes(enabled_response.into_body(), usize::MAX)
        .await
        .expect("failed to read enabled content body");
    let enabled_html = String::from_utf8(enabled_body.to_vec()).expect("invalid enabled body");
    assert!(
        enabled_html.contains(&format!("/admin/site/{}/render?publish=1", site.id)),
        "expected publish link to be visible when publish config exists"
    );

    crate::delete_site_publish_config(db.as_ref(), site.id)
        .await
        .expect("failed to delete publish config");

    let disabled_again_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/content", site.id))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("failed to build disabled-again content request"),
        )
        .await
        .expect("failed to load content page after deleting publish config");
    assert_eq!(disabled_again_response.status(), StatusCode::OK);
    let disabled_again_body = to_bytes(disabled_again_response.into_body(), usize::MAX)
        .await
        .expect("failed to read disabled-again content body");
    let disabled_again_html =
        String::from_utf8(disabled_again_body.to_vec()).expect("invalid disabled-again body");
    assert!(
        !disabled_again_html.contains(&format!("/admin/site/{}/render?publish=1", site.id)),
        "expected publish link to hide again once publish config is removed"
    );
}

#[tokio::test]
async fn admin_themes_install_rejects_non_admin_users() {
    let db = std::sync::Arc::new(test_db_start().await);
    let user = crate::entities::user::create_user(
        db.as_ref(),
        "viewer",
        Some("viewer@example.com"),
        Some("Viewer"),
        false,
    )
    .await
    .expect("failed to create viewer");
    let repo = create_theme_repo("version-one");
    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie =
        seed_session_cookie(test_admin_state(db.clone()), session_store.clone(), user.id).await;
    let body = urlencoded_theme_form(
        repo.path().to_str().expect("repo path should be utf-8"),
        Some("blocked-theme"),
    );

    let response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/themes")
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .expect("failed to build install request"),
        )
        .await
        .expect("failed to call install route");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_themes_update_refreshes_from_source_repo() {
    let db = std::sync::Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let repo = create_theme_repo("version-one");
    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;
    let install_body = urlencoded_theme_form(
        repo.path().to_str().expect("repo path should be utf-8"),
        Some("sample-theme"),
    );

    let install_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/themes")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(install_body))
                .expect("failed to build install request"),
        )
        .await
        .expect("failed to call install route");
    assert_eq!(install_response.status(), StatusCode::SEE_OTHER);

    update_theme_repo(repo.path(), "version-two", "update theme");

    let update_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/themes/sample-theme/update")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build update request"),
        )
        .await
        .expect("failed to call update route");

    assert_eq!(update_response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        update_response
            .headers()
            .get(header::LOCATION)
            .expect("missing location header")
            .to_str()
            .expect("invalid location header"),
        "/admin/themes?updated=sample-theme"
    );

    let installed_file = router
        .site_templates_root
        .path()
        .join("sample-theme")
        .join("theme.txt");
    let installed_content =
        std::fs::read_to_string(&installed_file).expect("failed to read refreshed theme file");
    assert_eq!(installed_content, "version-two");
}

#[tokio::test]
async fn admin_themes_delete_blocks_themes_still_in_use() {
    let db = std::sync::Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let repo = create_theme_repo("version-one");
    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;
    let install_body = urlencoded_theme_form(
        repo.path().to_str().expect("repo path should be utf-8"),
        Some("sample-theme"),
    );

    let install_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/themes")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(install_body))
                .expect("failed to build install request"),
        )
        .await
        .expect("failed to call install route");
    assert_eq!(install_response.status(), StatusCode::SEE_OTHER);

    let site = crate::create_site(
        db.as_ref(),
        "theme-site".to_string(),
        "Theme Site".to_string(),
        "sample-theme".to_string(),
    )
    .await
    .expect("failed to create site");

    let settings_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/settings", site.id))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("failed to build settings request"),
        )
        .await
        .expect("failed to load site settings");
    assert_eq!(settings_response.status(), StatusCode::OK);
    let settings_body = to_bytes(settings_response.into_body(), usize::MAX)
        .await
        .expect("failed to read settings body");
    assert!(
        String::from_utf8_lossy(&settings_body).contains("sample-theme"),
        "expected site settings to list installed theme"
    );

    let delete_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/themes/sample-theme/delete")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build delete request"),
        )
        .await
        .expect("failed to call delete route");

    assert_eq!(delete_response.status(), StatusCode::BAD_REQUEST);
    assert!(
        router
            .site_templates_root
            .path()
            .join("sample-theme")
            .exists()
    );
    let theme = crate::entities::theme_registry::Entity::find()
        .filter(crate::entities::theme_registry::Column::Slug.eq("sample-theme"))
        .one(db.as_ref())
        .await
        .expect("failed to load theme registry row");
    assert!(
        theme.is_some(),
        "expected theme row to remain after blocked delete"
    );
}

#[tokio::test]
async fn admin_site_export_allows_owner_and_sets_download_headers() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "export-site".to_string(),
        "Export Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let owner = crate::entities::user::create_user(
        db.as_ref(),
        "owner",
        Some("owner@example.com"),
        Some("Owner"),
        false,
    )
    .await
    .expect("failed to create owner");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: owner.id,
            role: SiteRole::Owner,
        },
    )
    .await
    .expect("failed to create owner membership");

    let session_store = MemoryStore::default();
    let router = site_transfer_test_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        owner.id,
    )
    .await;
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/export.json", site.id))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build export request"),
        )
        .await
        .expect("failed to call export route");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("missing content-type")
            .to_str()
            .expect("invalid content-type"),
        "application/json"
    );
    assert!(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .expect("missing content-disposition")
            .to_str()
            .expect("invalid content-disposition")
            .contains("attachment; filename=\"export-site-site-export.json\"")
    );
}

#[tokio::test]
async fn admin_site_export_rejects_non_owner_members() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "export-site".to_string(),
        "Export Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let viewer = crate::entities::user::create_user(
        db.as_ref(),
        "viewer",
        Some("viewer@example.com"),
        Some("Viewer"),
        false,
    )
    .await
    .expect("failed to create viewer");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: viewer.id,
            role: SiteRole::Viewer,
        },
    )
    .await
    .expect("failed to create viewer membership");

    let session_store = MemoryStore::default();
    let router = site_transfer_test_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        viewer.id,
    )
    .await;
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/export.json", site.id))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build export request"),
        )
        .await
        .expect("failed to call export route");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_site_import_allows_global_admin_and_creates_site() {
    let db = Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let export = crate::SiteExport {
        format_version: crate::SITE_EXPORT_FORMAT_VERSION,
        exported_at: Utc::now(),
        site: crate::site_export::ExportSite {
            id: Uuid::now_v7(),
            short_name: "imported-site".to_string(),
            full_title: "Imported Site".to_string(),
            template_name: DEFAULT_TEMPLATE_NAME.to_string(),
            publish_on_render: false,
            created_at: Utc::now(),
            updated_at: None,
            publish_config: None,
        },
        memberships: Vec::new(),
        tags: Vec::new(),
        content_items: Vec::new(),
        assets: Vec::new(),
        audit_events: Vec::new(),
        template_overrides: Vec::new(),
    };
    let json =
        crate::serialize_site_export_pretty(&export).expect("failed to serialize import json");
    let (boundary, body) = multipart_json_request_body(&json);

    let session_store = MemoryStore::default();
    let router = site_transfer_test_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/sites/import")
                .header(header::COOKIE, cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build import request"),
        )
        .await
        .expect("failed to call import route");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .expect("missing location header")
            .to_str()
            .expect("invalid location header"),
        "/admin?imported=1"
    );

    let imported_site = crate::entities::site::Entity::find()
        .filter(crate::entities::site::Column::ShortName.eq("imported-site"))
        .one(db.as_ref())
        .await
        .expect("failed to query imported site");
    assert!(imported_site.is_some(), "expected imported site to exist");
}

#[tokio::test]
async fn admin_site_import_check_reports_existing_site() {
    let db = Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let existing = crate::create_site(
        db.as_ref(),
        "duplicate-site".to_string(),
        "Existing Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create existing site");

    let session_store = MemoryStore::default();
    let router = site_transfer_test_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/admin/sites/import/check?short_name=duplicate-site")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("failed to build import check request"),
        )
        .await
        .expect("failed to call import check route");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read import check body");
    let lookup: serde_json::Value =
        serde_json::from_slice(&body).expect("failed to parse import check json");
    assert_eq!(
        lookup["short_name"].as_str().expect("missing short_name"),
        "duplicate-site"
    );
    assert!(lookup["exists"].as_bool().expect("missing exists flag"));
    assert_eq!(
        lookup["full_title"].as_str().expect("missing full_title"),
        "Existing Site"
    );
    assert_eq!(existing.short_name, "duplicate-site");
}

#[tokio::test]
async fn admin_site_import_replaces_existing_site_when_requested() {
    let db = Arc::new(test_db_start().await);
    let admin = crate::entities::user::create_user(
        db.as_ref(),
        "admin",
        Some("admin@example.com"),
        Some("Admin"),
        true,
    )
    .await
    .expect("failed to create admin");
    let existing = crate::create_site(
        db.as_ref(),
        "duplicate-site".to_string(),
        "Existing Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create existing site");
    let export = crate::SiteExport {
        format_version: crate::SITE_EXPORT_FORMAT_VERSION,
        exported_at: Utc::now(),
        site: crate::site_export::ExportSite {
            id: Uuid::now_v7(),
            short_name: "duplicate-site".to_string(),
            full_title: "Replaced Site".to_string(),
            template_name: DEFAULT_TEMPLATE_NAME.to_string(),
            publish_on_render: false,
            created_at: Utc::now(),
            updated_at: None,
            publish_config: None,
        },
        memberships: Vec::new(),
        tags: Vec::new(),
        content_items: Vec::new(),
        assets: Vec::new(),
        audit_events: Vec::new(),
        template_overrides: Vec::new(),
    };
    let json =
        crate::serialize_site_export_pretty(&export).expect("failed to serialize import json");
    let (boundary, body) = multipart_json_request_body_with_replace(&json, true);

    let session_store = MemoryStore::default();
    let router = site_transfer_test_router(test_admin_state(db.clone()), session_store.clone());
    let cookie = seed_session_cookie(
        test_admin_state(db.clone()),
        session_store.clone(),
        admin.id,
    )
    .await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/sites/import")
                .header(header::COOKIE, cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build replace import request"),
        )
        .await
        .expect("failed to call replace import route");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .expect("missing location header")
            .to_str()
            .expect("invalid location header"),
        "/admin?imported=1"
    );

    let imported_site = crate::entities::site::Entity::find()
        .filter(crate::entities::site::Column::ShortName.eq("duplicate-site"))
        .one(db.as_ref())
        .await
        .expect("failed to query replaced site")
        .expect("expected replaced site to exist");
    assert_eq!(imported_site.id, export.site.id);
    assert_eq!(imported_site.full_title, "Replaced Site");
    assert_ne!(imported_site.id, existing.id);
}

#[tokio::test]
async fn admin_site_import_rejects_non_admin_users() {
    let db = Arc::new(test_db_start().await);
    let user = crate::entities::user::create_user(
        db.as_ref(),
        "viewer",
        Some("viewer@example.com"),
        Some("Viewer"),
        false,
    )
    .await
    .expect("failed to create user");
    let export = crate::SiteExport {
        format_version: crate::SITE_EXPORT_FORMAT_VERSION,
        exported_at: Utc::now(),
        site: crate::site_export::ExportSite {
            id: Uuid::now_v7(),
            short_name: "blocked-import".to_string(),
            full_title: "Blocked Import".to_string(),
            template_name: DEFAULT_TEMPLATE_NAME.to_string(),
            publish_on_render: false,
            created_at: Utc::now(),
            updated_at: None,
            publish_config: None,
        },
        memberships: Vec::new(),
        tags: Vec::new(),
        content_items: Vec::new(),
        assets: Vec::new(),
        audit_events: Vec::new(),
        template_overrides: Vec::new(),
    };
    let json =
        crate::serialize_site_export_pretty(&export).expect("failed to serialize import json");
    let (boundary, body) = multipart_json_request_body(&json);

    let session_store = MemoryStore::default();
    let router = site_transfer_test_router(test_admin_state(db.clone()), session_store.clone());
    let cookie =
        seed_session_cookie(test_admin_state(db.clone()), session_store.clone(), user.id).await;
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/sites/import")
                .header(header::COOKIE, cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build import request"),
        )
        .await
        .expect("failed to call import route");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_site_wordpress_import_is_idempotent_per_file() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "wordpress-site".to_string(),
        "WordPress Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let user = crate::entities::user::create_user(
        db.as_ref(),
        "author",
        Some("author@example.com"),
        Some("Author"),
        false,
    )
    .await
    .expect("failed to create author");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: user.id,
            role: SiteRole::Author,
        },
    )
    .await
    .expect("failed to create author membership");

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Imported Post</title>
      <link>https://example.com/2020/01/imported-post/?p=123</link>
      <wp:post_id>123</wp:post_id>
      <wp:post_name>imported-post</wp:post_name>
      <wp:post_type>post</wp:post_type>
      <wp:status>publish</wp:status>
      <content:encoded><![CDATA[Hello world]]></content:encoded>
    </item>
  </channel>
</rss>
"#;
    let (boundary, body) = multipart_wordpress_xml_request_body(xml);

    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie =
        seed_session_cookie(test_admin_state(db.clone()), session_store.clone(), user.id).await;

    let first_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/site/{}/settings/wordpress-import", site.id))
                .header(header::COOKIE, &cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body.clone()))
                .expect("failed to build wordpress import request"),
        )
        .await
        .expect("failed to call wordpress import route");

    assert_eq!(first_response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        first_response
            .headers()
            .get(header::LOCATION)
            .expect("missing first location header")
            .to_str()
            .expect("invalid first location header"),
        format!("/admin/site/{}/settings", site.id)
    );

    let first_settings_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/settings", site.id))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("failed to build first settings request"),
        )
        .await
        .expect("failed to load site settings after first import");
    assert_eq!(first_settings_response.status(), StatusCode::OK);
    let first_settings_body = to_bytes(first_settings_response.into_body(), usize::MAX)
        .await
        .expect("failed to read first settings body");
    assert!(
        String::from_utf8_lossy(&first_settings_body).contains("Imported 1 WordPress item."),
        "expected wordpress import toast after first import"
    );

    let first_content = crate::list_content(db.as_ref(), site.id, None, None)
        .await
        .expect("failed to list content after first wordpress import");
    assert_eq!(first_content.len(), 1);

    let second_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/site/{}/settings/wordpress-import", site.id))
                .header(header::COOKIE, &cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build second wordpress import request"),
        )
        .await
        .expect("failed to call wordpress import route a second time");

    assert_eq!(second_response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        second_response
            .headers()
            .get(header::LOCATION)
            .expect("missing second location header")
            .to_str()
            .expect("invalid second location header"),
        format!("/admin/site/{}/settings", site.id)
    );

    let second_settings_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/settings", site.id))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("failed to build second settings request"),
        )
        .await
        .expect("failed to load site settings after second import");
    assert_eq!(second_settings_response.status(), StatusCode::OK);
    let second_settings_body = to_bytes(second_settings_response.into_body(), usize::MAX)
        .await
        .expect("failed to read second settings body");
    assert!(
        String::from_utf8_lossy(&second_settings_body)
            .contains("No new WordPress items were imported."),
        "expected wordpress import toast after duplicate import"
    );

    let second_content = crate::list_content(db.as_ref(), site.id, None, None)
        .await
        .expect("failed to list content after second wordpress import");
    assert_eq!(second_content.len(), 1);
}

#[tokio::test]
async fn admin_site_wordpress_import_reports_updated_password_protected_titles() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "wordpress-site".to_string(),
        "WordPress Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let user = crate::entities::user::create_user(
        db.as_ref(),
        "author",
        Some("author@example.com"),
        Some("Author"),
        false,
    )
    .await
    .expect("failed to create author");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: user.id,
            role: SiteRole::Author,
        },
    )
    .await
    .expect("failed to create author membership");
    let existing = crate::create_content(
        db.as_ref(),
        crate::NewContent {
            site_id: site.id,
            page_type: PageType::Post,
            title: "Imported Post".to_string(),
            slug: "imported-post".to_string(),
            page_content: "Hello world".to_string(),
            draft: false,
            creator_sub: "author".to_string(),
            created_at: None,
            published_at: Some(Utc::now()),
        },
    )
    .await
    .expect("failed to create existing content");
    crate::create_alias(
        db.as_ref(),
        crate::NewAlias {
            content_id: existing.id,
            site_id: site.id,
            alias_path: "/?p=123".to_string(),
            kind: "alias".to_string(),
        },
    )
    .await
    .expect("failed to create existing alias");

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Imported Post</title>
      <link>https://example.com/2020/01/imported-post/?p=123</link>
      <wp:post_id>123</wp:post_id>
      <wp:post_name>imported-post</wp:post_name>
      <wp:post_password>secret</wp:post_password>
      <wp:post_type>post</wp:post_type>
      <wp:status>publish</wp:status>
      <content:encoded><![CDATA[Hello world]]></content:encoded>
    </item>
  </channel>
</rss>
"#;
    let (boundary, body) = multipart_wordpress_xml_request_body(xml);

    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie =
        seed_session_cookie(test_admin_state(db.clone()), session_store.clone(), user.id).await;

    let response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/site/{}/settings/wordpress-import", site.id))
                .header(header::COOKIE, &cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build wordpress import request"),
        )
        .await
        .expect("failed to call wordpress import route");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .expect("missing location header")
            .to_str()
            .expect("invalid location header"),
        format!("/admin/site/{}/settings", site.id)
    );

    let settings_response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/site/{}/settings", site.id))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("failed to build settings request"),
        )
        .await
        .expect("failed to load site settings after wordpress import");
    assert_eq!(settings_response.status(), StatusCode::OK);
    let settings_body = to_bytes(settings_response.into_body(), usize::MAX)
        .await
        .expect("failed to read settings body");
    let settings_html = String::from_utf8(settings_body.to_vec())
        .expect("site settings body should be valid utf-8");
    assert!(
        settings_html
            .contains("Updated 1 existing WordPress item: [PASSWORD-PROTECTED] Imported Post."),
        "expected wordpress import update toast"
    );

    let content = crate::list_content(db.as_ref(), site.id, Some(PageType::Post), None)
        .await
        .expect("failed to list site content");
    assert_eq!(content.len(), 1);
    assert_eq!(
        content[0].title,
        "[PASSWORD-PROTECTED] Imported Post".to_string()
    );
    assert!(content[0].draft);
    assert_eq!(content[0].published_at, None);
}

#[tokio::test]
async fn admin_site_wordpress_import_accepts_large_xml_uploads() {
    let db = Arc::new(test_db_start().await);
    let site = crate::create_site(
        db.as_ref(),
        "wordpress-site".to_string(),
        "WordPress Site".to_string(),
        DEFAULT_TEMPLATE_NAME.to_string(),
    )
    .await
    .expect("failed to create site");
    let user = crate::entities::user::create_user(
        db.as_ref(),
        "author",
        Some("author@example.com"),
        Some("Author"),
        false,
    )
    .await
    .expect("failed to create author");
    crate::create_membership(
        db.as_ref(),
        crate::NewMembership {
            site_id: site.id,
            user_id: user.id,
            role: SiteRole::Author,
        },
    )
    .await
    .expect("failed to create author membership");

    let large_body = "A".repeat(3 * 1024 * 1024);
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Imported Post</title>
      <link>https://example.com/2020/01/imported-post/?p=123</link>
      <wp:post_id>123</wp:post_id>
      <wp:post_name>imported-post</wp:post_name>
      <wp:post_type>post</wp:post_type>
      <wp:status>publish</wp:status>
      <content:encoded><![CDATA[{large_body}]]></content:encoded>
    </item>
  </channel>
</rss>
"#
    );
    let (boundary, body) = multipart_wordpress_xml_request_body(&xml);

    let session_store = MemoryStore::default();
    let router = test_app_router(test_admin_state(db.clone()), session_store.clone());
    let cookie =
        seed_session_cookie(test_admin_state(db.clone()), session_store.clone(), user.id).await;

    let response = router
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/site/{}/settings/wordpress-import", site.id))
                .header(header::COOKIE, &cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("failed to build wordpress import request"),
        )
        .await
        .expect("failed to call wordpress import route");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .expect("missing location header")
            .to_str()
            .expect("invalid location header"),
        format!("/admin/site/{}/settings", site.id)
    );

    let content = crate::list_content(db.as_ref(), site.id, Some(PageType::Post), None)
        .await
        .expect("failed to list site content");
    assert_eq!(content.len(), 1);
    assert_eq!(content[0].title, "Imported Post");
    assert_eq!(content[0].page_content.len(), large_body.len());
}
