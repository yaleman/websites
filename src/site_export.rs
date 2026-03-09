use crate::errors::SiteError;
use crate::{
    entities, list_asset_variants, list_audit_events, list_memberships, list_users_by_ids,
};
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::Serialize;
use std::path::Path;
use tokio::fs;
use uuid::Uuid;

pub const SITE_EXPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct SiteExport {
    pub format_version: u32,
    pub exported_at: DateTime<Utc>,
    pub site: ExportSite,
    pub memberships: Vec<ExportMembership>,
    pub tags: Vec<ExportTag>,
    pub content_items: Vec<ExportContentItem>,
    pub assets: Vec<ExportAsset>,
    pub audit_events: Vec<ExportAuditEvent>,
    pub template_overrides: Vec<ExportTemplateOverride>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportSite {
    pub id: Uuid,
    pub short_name: String,
    pub full_title: String,
    pub template_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportUser {
    pub id: Uuid,
    pub subject: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub admin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportMembership {
    pub id: Uuid,
    pub site_id: Uuid,
    pub user_id: Uuid,
    pub role: crate::web::SiteRole,
    pub user: ExportUser,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportTag {
    pub id: Uuid,
    pub site_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportTagReference {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportContentTag {
    pub id: Uuid,
    pub content_id: Uuid,
    pub tag_id: Uuid,
    pub tag: ExportTagReference,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportContentAlias {
    pub id: Uuid,
    pub content_id: Uuid,
    pub site_id: Uuid,
    pub alias_path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportRevisionAlias {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub content_id: Uuid,
    pub alias_path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportRevisionTag {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub content_id: Uuid,
    pub tag_id: Uuid,
    pub tag: ExportTagReference,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportContentRevision {
    pub id: Uuid,
    pub content_id: Uuid,
    pub site_id: Uuid,
    pub revision_number: i32,
    pub title: String,
    pub slug: String,
    pub page_content: String,
    pub draft: bool,
    pub page_type: entities::PageType,
    pub editor_sub: String,
    pub created_at: DateTime<Utc>,
    pub aliases: Vec<ExportRevisionAlias>,
    pub tags: Vec<ExportRevisionTag>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportContentItem {
    pub id: Uuid,
    pub site_id: Uuid,
    pub page_type: entities::PageType,
    pub title: String,
    pub slug: String,
    pub page_content: String,
    pub draft: bool,
    pub creator_sub: String,
    pub created_at: DateTime<Utc>,
    pub last_updated: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
    pub aliases: Vec<ExportContentAlias>,
    pub tags: Vec<ExportContentTag>,
    pub revisions: Vec<ExportContentRevision>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportFileMetadata {
    pub relative_path: String,
    pub exists: bool,
    pub byte_length: Option<u64>,
    pub modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportAssetVariant {
    pub id: Uuid,
    pub asset_id: Uuid,
    pub variant_kind: String,
    pub filename: String,
    pub mime_type: String,
    pub byte_length: i32,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub file: ExportFileMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportAsset {
    pub id: Uuid,
    pub site_id: Uuid,
    pub uploader_sub: String,
    pub original_filename: String,
    pub storage_basename: String,
    pub mime_type: String,
    pub byte_length: i32,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub original_file: ExportFileMetadata,
    pub variants: Vec<ExportAssetVariant>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportAuditEvent {
    pub id: Uuid,
    pub site_id: Option<Uuid>,
    pub actor_sub: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub created_at: DateTime<Utc>,
    pub payload_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportTemplateOverride {
    pub file_name: String,
    pub file: ExportFileMetadata,
}

pub async fn export_site(db: &DatabaseConnection, site_id: Uuid) -> Result<SiteExport, SiteError> {
    let upload_root = crate::resolve_upload_root();
    let override_root = crate::resolve_site_template_override_root(site_id);
    export_site_with_roots(db, site_id, &upload_root, &override_root).await
}

pub async fn export_site_with_roots(
    db: &DatabaseConnection,
    site_id: Uuid,
    upload_root: &Path,
    override_root: &Path,
) -> Result<SiteExport, SiteError> {
    let site = entities::site::Entity::find_by_id(site_id)
        .one(db)
        .await?
        .ok_or_else(|| SiteError::SiteNotFound(site_id.to_string()))?;

    let memberships = list_memberships(db, site_id)
        .await
        .map_err(SiteError::internal)?;
    let mut user_ids = memberships
        .iter()
        .map(|membership| membership.user_id)
        .collect::<Vec<_>>();
    user_ids.sort_unstable();
    user_ids.dedup();

    let users = list_users_by_ids(db, user_ids).await?;
    let mut user_map = users
        .into_iter()
        .map(|user| (user.id, user))
        .collect::<std::collections::HashMap<_, _>>();

    let mut exported_memberships = memberships
        .into_iter()
        .filter_map(|membership| {
            let user = user_map.remove(&membership.user_id)?;
            Some(ExportMembership {
                id: membership.id,
                site_id: membership.site_id,
                user_id: membership.user_id,
                role: membership.role,
                user: ExportUser {
                    id: user.id,
                    subject: user.subject,
                    created_at: user.created_at,
                    last_login_at: user.last_login_at,
                    email: user.email,
                    display_name: user.display_name,
                    admin: user.admin,
                },
            })
        })
        .collect::<Vec<_>>();
    exported_memberships.sort_by(|left, right| {
        left.user
            .subject
            .cmp(&right.user.subject)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut tags = entities::tag::Entity::find()
        .filter(entities::tag::Column::SiteId.eq(site_id))
        .all(db)
        .await?;
    tags.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    let tag_map = tags
        .iter()
        .map(|tag| {
            (
                tag.id,
                ExportTagReference {
                    id: tag.id,
                    name: tag.name.clone(),
                },
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let exported_tags = tags
        .into_iter()
        .map(|tag| ExportTag {
            id: tag.id,
            site_id: tag.site_id,
            name: tag.name,
        })
        .collect::<Vec<_>>();

    let mut content_items = entities::content_item::Entity::find()
        .filter(entities::content_item::Column::SiteId.eq(site_id))
        .all(db)
        .await?;
    content_items.sort_by(|left, right| {
        left.page_type
            .as_ref()
            .cmp(right.page_type.as_ref())
            .then_with(|| left.slug.cmp(&right.slug))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut exported_content_items = Vec::with_capacity(content_items.len());
    for content in content_items {
        let mut aliases = entities::content_alias::Entity::find()
            .filter(entities::content_alias::Column::ContentId.eq(content.id))
            .all(db)
            .await?;
        aliases.sort_by(|left, right| {
            left.alias_path
                .cmp(&right.alias_path)
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut content_tag_links = entities::content_tag::Entity::find()
            .filter(entities::content_tag::Column::ContentId.eq(content.id))
            .all(db)
            .await?;
        content_tag_links.sort_by(|left, right| left.id.cmp(&right.id));

        let mut revisions = entities::content_revision::Entity::find()
            .filter(entities::content_revision::Column::ContentId.eq(content.id))
            .all(db)
            .await?;
        revisions.sort_by(|left, right| {
            left.revision_number
                .cmp(&right.revision_number)
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut exported_revisions = Vec::with_capacity(revisions.len());
        for revision in revisions {
            let mut revision_aliases = entities::content_revision_alias::Entity::find()
                .filter(entities::content_revision_alias::Column::RevisionId.eq(revision.id))
                .all(db)
                .await?;
            revision_aliases.sort_by(|left, right| {
                left.alias_path
                    .cmp(&right.alias_path)
                    .then_with(|| left.id.cmp(&right.id))
            });

            let mut revision_tag_links = entities::content_revision_tag::Entity::find()
                .filter(entities::content_revision_tag::Column::RevisionId.eq(revision.id))
                .all(db)
                .await?;
            revision_tag_links.sort_by(|left, right| left.id.cmp(&right.id));

            exported_revisions.push(ExportContentRevision {
                id: revision.id,
                content_id: revision.content_id,
                site_id: revision.site_id,
                revision_number: revision.revision_number,
                title: revision.title,
                slug: revision.slug,
                page_content: revision.page_content,
                draft: revision.draft,
                page_type: revision.page_type,
                editor_sub: revision.editor_sub,
                created_at: revision.created_at,
                aliases: revision_aliases
                    .into_iter()
                    .map(|alias| ExportRevisionAlias {
                        id: alias.id,
                        revision_id: alias.revision_id,
                        content_id: alias.content_id,
                        alias_path: alias.alias_path,
                        kind: alias.kind,
                    })
                    .collect(),
                tags: revision_tag_links
                    .into_iter()
                    .filter_map(|link| {
                        tag_map
                            .get(&link.tag_id)
                            .cloned()
                            .map(|tag| ExportRevisionTag {
                                id: link.id,
                                revision_id: link.revision_id,
                                content_id: link.content_id,
                                tag_id: link.tag_id,
                                tag,
                            })
                    })
                    .collect(),
            });
        }

        exported_content_items.push(ExportContentItem {
            id: content.id,
            site_id: content.site_id,
            page_type: content.page_type,
            title: content.title,
            slug: content.slug,
            page_content: content.page_content,
            draft: content.draft,
            creator_sub: content.creator_sub,
            created_at: content.created_at,
            last_updated: content.last_updated,
            published_at: content.published_at,
            aliases: aliases
                .into_iter()
                .map(|alias| ExportContentAlias {
                    id: alias.id,
                    content_id: alias.content_id,
                    site_id: alias.site_id,
                    alias_path: alias.alias_path,
                    kind: alias.kind,
                })
                .collect(),
            tags: content_tag_links
                .into_iter()
                .filter_map(|link| {
                    tag_map
                        .get(&link.tag_id)
                        .cloned()
                        .map(|tag| ExportContentTag {
                            id: link.id,
                            content_id: link.content_id,
                            tag_id: link.tag_id,
                            tag,
                        })
                })
                .collect(),
            revisions: exported_revisions,
        });
    }

    let mut assets = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .all(db)
        .await?;
    assets.sort_by(|left, right| {
        left.original_filename
            .cmp(&right.original_filename)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut exported_assets = Vec::with_capacity(assets.len());
    for asset in assets {
        let mut variants = list_asset_variants(db, asset.id)
            .await
            .map_err(SiteError::internal)?;
        variants.sort_by(|left, right| {
            left.variant_kind
                .cmp(&right.variant_kind)
                .then_with(|| left.filename.cmp(&right.filename))
                .then_with(|| left.id.cmp(&right.id))
        });

        let original_file = file_metadata(upload_root, Path::new(&asset.storage_basename)).await?;
        let exported_variants = variants
            .into_iter()
            .map(|variant| async move {
                Ok::<ExportAssetVariant, SiteError>(ExportAssetVariant {
                    id: variant.id,
                    asset_id: variant.asset_id,
                    variant_kind: variant.variant_kind.clone(),
                    filename: variant.filename.clone(),
                    mime_type: variant.mime_type,
                    byte_length: variant.byte_length,
                    width: variant.width,
                    height: variant.height,
                    file: file_metadata(upload_root, Path::new(&variant.filename)).await?,
                })
            })
            .collect::<Vec<_>>();

        let mut resolved_variants = Vec::with_capacity(exported_variants.len());
        for variant in exported_variants {
            resolved_variants.push(variant.await?);
        }

        exported_assets.push(ExportAsset {
            id: asset.id,
            site_id: asset.site_id,
            uploader_sub: asset.uploader_sub,
            original_filename: asset.original_filename,
            storage_basename: asset.storage_basename,
            mime_type: asset.mime_type,
            byte_length: asset.byte_length,
            width: asset.width,
            height: asset.height,
            created_at: asset.created_at,
            original_file,
            variants: resolved_variants,
        });
    }

    let mut audit_events = list_audit_events(db, Some(site_id))
        .await
        .map_err(SiteError::internal)?;
    audit_events.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let exported_audit_events = audit_events
        .into_iter()
        .map(|event| ExportAuditEvent {
            id: event.id,
            site_id: event.site_id,
            actor_sub: event.actor_sub,
            event_type: event.event_type,
            entity_type: event.entity_type,
            entity_id: event.entity_id,
            created_at: event.created_at,
            payload_json: event.payload_json,
        })
        .collect();

    let template_overrides = export_template_overrides(override_root).await?;

    Ok(SiteExport {
        format_version: SITE_EXPORT_FORMAT_VERSION,
        exported_at: Utc::now(),
        site: ExportSite {
            id: site.id,
            short_name: site.short_name,
            full_title: site.full_title,
            template_name: site.template_name,
            created_at: site.created_at,
            updated_at: site.updated_at,
        },
        memberships: exported_memberships,
        tags: exported_tags,
        content_items: exported_content_items,
        assets: exported_assets,
        audit_events: exported_audit_events,
        template_overrides,
    })
}

pub fn serialize_site_export_pretty(export: &SiteExport) -> Result<String, SiteError> {
    serde_json::to_string_pretty(export)
        .map_err(|error| SiteError::internal(format!("failed to serialize site export: {error}")))
}

async fn export_template_overrides(
    override_root: &Path,
) -> Result<Vec<ExportTemplateOverride>, SiteError> {
    let mut entries = match fs::read_dir(override_root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(SiteError::from(error)),
    };

    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry.file_type().await?;
        if !file_type.is_file() {
            continue;
        }
        files.push(file_name);
    }
    files.sort();

    let mut overrides = Vec::with_capacity(files.len());
    for file_name in files {
        overrides.push(ExportTemplateOverride {
            file_name: file_name.clone(),
            file: file_metadata(override_root, Path::new(&file_name)).await?,
        });
    }

    Ok(overrides)
}

async fn file_metadata(root: &Path, relative_path: &Path) -> Result<ExportFileMetadata, SiteError> {
    let path = root.join(relative_path);
    match fs::metadata(&path).await {
        Ok(metadata) => Ok(ExportFileMetadata {
            relative_path: normalize_relative_path(relative_path),
            exists: true,
            byte_length: Some(metadata.len()),
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ExportFileMetadata {
            relative_path: normalize_relative_path(relative_path),
            exists: false,
            byte_length: None,
            modified_at: None,
        }),
        Err(error) => Err(SiteError::from(error)),
    }
}

fn normalize_relative_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_db_start;
    use crate::entities::audit_event::log_audit_event;
    use crate::web::SiteRole;
    use crate::{
        NewAlias, NewAsset, NewAssetVariant, NewContent, NewContentTag, NewMembership, NewTag,
        add_content_tag, create_alias, create_asset, create_asset_variant, create_content,
        create_membership, create_site, create_tag, update_content,
    };
    use tempfile::TempDir;

    #[tokio::test]
    async fn export_site_includes_site_scoped_records_and_file_metadata() {
        let db = test_db_start().await;
        let upload_root = TempDir::new().expect("failed to create upload root");
        let override_root = TempDir::new().expect("failed to create override root");

        let site = create_site(
            &db,
            "exported".to_string(),
            "Exported Site".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create site");
        let other_site = create_site(
            &db,
            "other".to_string(),
            "Other Site".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create other site");

        let user = crate::entities::user::create_user(
            &db,
            "owner-subject",
            Some("owner@example.com"),
            Some("Owner User"),
            false,
        )
        .await
        .expect("failed to create user");
        create_membership(
            &db,
            NewMembership {
                site_id: site.id,
                user_id: user.id,
                role: SiteRole::Owner,
            },
        )
        .await
        .expect("failed to create membership");

        let _other_tag = create_tag(
            &db,
            NewTag {
                site_id: other_site.id,
                name: "Other".to_string(),
            },
        )
        .await
        .expect("failed to create other site tag");

        let export_tag = create_tag(
            &db,
            NewTag {
                site_id: site.id,
                name: "Export".to_string(),
            },
        )
        .await
        .expect("failed to create export tag");

        let content = create_content(
            &db,
            NewContent {
                site_id: site.id,
                page_type: entities::PageType::Post,
                title: "Export Me".to_string(),
                slug: "export-me".to_string(),
                page_content: "Body".to_string(),
                draft: true,
                creator_sub: "creator".to_string(),
                published_at: None,
            },
        )
        .await
        .expect("failed to create content");
        create_alias(
            &db,
            NewAlias {
                content_id: content.id,
                site_id: site.id,
                alias_path: "/legacy/export-me".to_string(),
                kind: "alias".to_string(),
            },
        )
        .await
        .expect("failed to create alias");
        add_content_tag(
            &db,
            NewContentTag {
                content_id: content.id,
                site_id: site.id,
                tag_name: export_tag.name.clone(),
            },
        )
        .await
        .expect("failed to add content tag");
        update_content(
            &db,
            crate::UpdateContent {
                content_id: content.id,
                page_type: None,
                title: Some("Export Me Updated".to_string()),
                slug: Some("export-me-updated".to_string()),
                page_content: Some("Updated body".to_string()),
                draft: Some(false),
                published_at: None,
                editor_sub: "editor".to_string(),
            },
        )
        .await
        .expect("failed to update content");

        let asset = create_asset(
            &db,
            NewAsset {
                site_id: site.id,
                uploader_sub: "uploader".to_string(),
                original_filename: "photo.jpg".to_string(),
                storage_basename: "asset.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                byte_length: 42,
                width: Some(320),
                height: Some(240),
            },
        )
        .await
        .expect("failed to create asset");
        create_asset_variant(
            &db,
            NewAssetVariant {
                asset_id: asset.id,
                variant_kind: "thumbnail".to_string(),
                filename: "asset-thumb.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                byte_length: 24,
                width: Some(120),
                height: Some(90),
            },
        )
        .await
        .expect("failed to create asset variant");

        fs::write(upload_root.path().join("asset.jpg"), b"asset-data")
            .await
            .expect("failed to write asset file");
        fs::write(upload_root.path().join("override-only.html"), b"shared")
            .await
            .expect("failed to write unrelated upload file");
        fs::write(
            override_root.path().join("page.html"),
            b"<html>override</html>",
        )
        .await
        .expect("failed to write override file");

        log_audit_event(
            &db,
            "actor",
            "update_site",
            "site",
            site.id,
            Some(site.id),
            Some(serde_json::json!({ "field": "value" })),
        )
        .await
        .expect("failed to log audit");
        log_audit_event(&db, "actor", "global", "user", "user-1", None, None)
            .await
            .expect("failed to log unrelated audit");

        let export = export_site_with_roots(&db, site.id, upload_root.path(), override_root.path())
            .await
            .expect("failed to export site");

        assert_eq!(export.format_version, SITE_EXPORT_FORMAT_VERSION);
        assert_eq!(export.site.id, site.id);
        assert_eq!(export.memberships.len(), 1);
        assert_eq!(export.memberships[0].user.subject, "owner-subject");
        assert_eq!(export.tags.len(), 1);
        assert_eq!(export.tags[0].name, "Export");
        assert_eq!(export.content_items.len(), 1);
        assert_eq!(export.content_items[0].aliases.len(), 1);
        assert_eq!(export.content_items[0].tags.len(), 1);
        assert_eq!(export.content_items[0].revisions.len(), 2);
        assert_eq!(export.content_items[0].revisions[1].tags.len(), 1);
        assert_eq!(
            export.content_items[0].revisions[1].aliases[0].alias_path,
            "/legacy/export-me"
        );
        assert_eq!(export.assets.len(), 1);
        assert!(export.assets[0].original_file.exists);
        assert_eq!(export.assets[0].original_file.relative_path, "asset.jpg");
        assert_eq!(export.assets[0].variants.len(), 1);
        assert!(!export.assets[0].variants[0].file.exists);
        assert_eq!(
            export.assets[0].variants[0].file.relative_path,
            "asset-thumb.jpg"
        );
        assert_eq!(export.audit_events.len(), 1);
        assert_eq!(export.template_overrides.len(), 1);
        assert_eq!(export.template_overrides[0].file_name, "page.html");
        assert!(export.template_overrides[0].file.exists);
    }

    #[tokio::test]
    async fn serialize_site_export_pretty_produces_pretty_json() {
        let export = SiteExport {
            format_version: SITE_EXPORT_FORMAT_VERSION,
            exported_at: Utc::now(),
            site: ExportSite {
                id: Uuid::nil(),
                short_name: "test".to_string(),
                full_title: "Test".to_string(),
                template_name: "default".to_string(),
                created_at: Utc::now(),
                updated_at: None,
            },
            memberships: Vec::new(),
            tags: Vec::new(),
            content_items: Vec::new(),
            assets: Vec::new(),
            audit_events: Vec::new(),
            template_overrides: Vec::new(),
        };

        let json = serialize_site_export_pretty(&export).expect("failed to serialize export");
        assert!(json.contains("\n  \"format_version\": 1,"));
    }
}
