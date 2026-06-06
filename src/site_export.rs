use crate::errors::SiteError;
use crate::publish::{RsyncPublishConfig, S3CompatiblePublishConfig};
use crate::{
    entities, list_asset_variants, list_audit_events, list_memberships, list_users_by_ids,
};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use uuid::Uuid;

pub const SITE_EXPORT_FORMAT_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSite {
    pub id: Uuid,
    pub short_name: String,
    pub full_title: String,
    pub template_name: String,
    #[serde(default)]
    pub publish_on_render: bool,
    #[serde(default)]
    pub internal_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass_import_assets: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_config: Option<ExportSitePublishConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSitePublishConfig {
    pub method: entities::site_publish_config::PublishMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3: Option<S3CompatiblePublishConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsync: Option<RsyncPublishConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportUser {
    pub id: Uuid,
    pub subject: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportMembership {
    pub id: Uuid,
    pub site_id: Uuid,
    pub user_id: Uuid,
    pub role: crate::web::SiteRole,
    pub user: ExportUser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportTag {
    pub id: Uuid,
    pub site_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportTagReference {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportContentTag {
    pub id: Uuid,
    pub content_id: Uuid,
    pub tag_id: Uuid,
    pub tag: ExportTagReference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportContentAlias {
    pub id: Uuid,
    pub content_id: Uuid,
    pub site_id: Uuid,
    pub alias_path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRevisionAlias {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub content_id: Uuid,
    pub alias_path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRevisionTag {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub content_id: Uuid,
    pub tag_id: Uuid,
    pub tag: ExportTagReference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportFileMetadata {
    pub relative_path: String,
    pub exists: bool,
    pub byte_length: Option<u64>,
    pub modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportTemplateOverride {
    pub file_name: String,
    pub file: ExportFileMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteImportResult {
    pub site_id: Uuid,
    pub site_short_name: String,
    pub created_users: usize,
    pub reused_users: usize,
    pub warnings: Vec<String>,
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
        .collect::<HashMap<_, _>>();

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
        .collect::<HashMap<_, _>>();
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
        content_tag_links.sort_by_key(|left| left.id);

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
            revision_tag_links.sort_by_key(|left| left.id);

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
    let publish_config = entities::site_publish_config::Entity::find_by_id(site_id)
        .one(db)
        .await?
        .map(|row| match row.method {
            entities::site_publish_config::PublishMethod::S3Compatible => {
                let s3 = serde_json::from_value::<S3CompatiblePublishConfig>(row.config_json)
                    .map_err(|error| {
                        SiteError::BadRequest(format!("invalid publish config in export: {error}"))
                    })?;
                Ok(ExportSitePublishConfig {
                    method: row.method,
                    s3: Some(s3),
                    rsync: None,
                })
            }
            entities::site_publish_config::PublishMethod::RsyncSsh => {
                let rsync = serde_json::from_value::<RsyncPublishConfig>(row.config_json).map_err(
                    |error| {
                        SiteError::BadRequest(format!("invalid publish config in export: {error}"))
                    },
                )?;
                Ok(ExportSitePublishConfig {
                    method: row.method,
                    s3: None,
                    rsync: Some(rsync),
                })
            }
            entities::site_publish_config::PublishMethod::Disabled => Err(SiteError::BadRequest(
                format!("unsupported publish method in export: {}", row.method),
            )),
        })
        .transpose()?;

    Ok(SiteExport {
        format_version: SITE_EXPORT_FORMAT_VERSION,
        exported_at: Utc::now(),
        site: ExportSite {
            id: site.id,
            short_name: site.short_name,
            full_title: site.full_title,
            template_name: site.template_name,
            publish_on_render: site.publish_on_render,
            internal_domains: site.internal_domains.into_vec(),
            mass_import_assets: site.mass_import_assets,
            created_at: site.created_at,
            updated_at: site.updated_at,
            publish_config,
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

pub fn deserialize_site_export(json: &[u8]) -> Result<SiteExport, SiteError> {
    let export = serde_json::from_slice::<SiteExport>(json)
        .map_err(|error| SiteError::BadRequest(format!("invalid site export json: {error}")))?;
    validate_site_export(&export)?;
    Ok(export)
}

pub async fn import_site_json<C: ConnectionTrait>(
    db: &C,
    json: &[u8],
) -> Result<SiteImportResult, SiteError> {
    let export = deserialize_site_export(json)?;
    import_site_export(db, &export).await
}

pub async fn import_site_export<C: ConnectionTrait>(
    db: &C,
    export: &SiteExport,
) -> Result<SiteImportResult, SiteError> {
    validate_site_export(export)?;

    let existing_site = entities::site::Entity::find()
        .filter(entities::site::Column::ShortName.eq(export.site.short_name.clone()))
        .one(db)
        .await?;
    if existing_site.is_some() {
        return Err(SiteError::BadRequest(format!(
            "site short_name already exists: {}",
            export.site.short_name
        )));
    }

    let site_id = if entities::site::Entity::find_by_id(export.site.id)
        .one(db)
        .await?
        .is_some()
    {
        Uuid::now_v7()
    } else {
        export.site.id
    };

    entities::site::ActiveModel {
        id: Set(site_id),
        short_name: Set(export.site.short_name.clone()),
        full_title: Set(export.site.full_title.clone()),
        template_name: Set(export.site.template_name.clone()),
        publish_on_render: Set(export.site.publish_on_render),
        internal_domains: Set(entities::site::InternalDomains::from(
            crate::normalize_internal_domains(export.site.internal_domains.clone()),
        )),
        mass_import_assets: Set(export.site.mass_import_assets.clone()),
        created_at: Set(export.site.created_at),
        updated_at: Set(export.site.updated_at),
    }
    .insert(db)
    .await?;

    if let Some(publish_config) = &export.site.publish_config {
        let config_json = match publish_config.method {
            entities::site_publish_config::PublishMethod::S3Compatible => {
                serde_json::to_value(publish_config.s3.as_ref().ok_or_else(|| {
                    SiteError::BadRequest("export publish config missing S3 settings".to_string())
                })?)
            }
            entities::site_publish_config::PublishMethod::RsyncSsh => {
                serde_json::to_value(publish_config.rsync.as_ref().ok_or_else(|| {
                    SiteError::BadRequest(
                        "export publish config missing rsync settings".to_string(),
                    )
                })?)
            }
            entities::site_publish_config::PublishMethod::Disabled => {
                return Err(SiteError::BadRequest(
                    "disabled publish configs cannot be imported".to_string(),
                ));
            }
        }
        .map_err(|error| {
            SiteError::internal(format!("failed to serialize publish config: {error}"))
        })?;
        entities::site_publish_config::ActiveModel {
            site_id: Set(site_id),
            method: Set(publish_config.method),
            config_json: Set(config_json),
            created_at: Set(export.exported_at),
            updated_at: Set(export.exported_at),
        }
        .insert(db)
        .await?;
    }

    let mut created_users = 0usize;
    let mut reused_users = 0usize;
    let mut user_id_map = HashMap::new();

    for membership in &export.memberships {
        if membership.role.is_admin() {
            return Err(SiteError::BadRequest(
                "site membership exports cannot contain admin roles".to_string(),
            ));
        }
        if membership.user_id != membership.user.id {
            return Err(SiteError::BadRequest(format!(
                "membership {} has mismatched embedded user id",
                membership.id
            )));
        }
        if user_id_map.contains_key(&membership.user.id) {
            continue;
        }

        let existing_user = entities::user::Entity::find()
            .filter(entities::user::Column::Subject.eq(membership.user.subject.clone()))
            .one(db)
            .await?;
        let resolved_user_id = if let Some(existing_user) = existing_user {
            reused_users = reused_users.saturating_add(1);
            existing_user.id
        } else {
            let user_id = if entities::user::Entity::find_by_id(membership.user.id)
                .one(db)
                .await?
                .is_some()
            {
                Uuid::now_v7()
            } else {
                membership.user.id
            };
            entities::user::ActiveModel {
                id: Set(user_id),
                subject: Set(membership.user.subject.clone()),
                created_at: Set(membership.user.created_at),
                last_login_at: Set(membership.user.last_login_at),
                email: Set(membership.user.email.clone()),
                display_name: Set(membership.user.display_name.clone()),
                admin: Set(membership.user.admin),
            }
            .insert(db)
            .await?;
            created_users = created_users.saturating_add(1);
            user_id
        };
        user_id_map.insert(membership.user.id, resolved_user_id);
    }

    for membership in &export.memberships {
        entities::site_membership::ActiveModel {
            id: Set(membership.id),
            site_id: Set(site_id),
            user_id: Set(*user_id_map.get(&membership.user.id).ok_or_else(|| {
                SiteError::internal(format!(
                    "missing imported user mapping for {}",
                    membership.id
                ))
            })?),
            role: Set(membership.role),
        }
        .insert(db)
        .await?;
    }

    let mut tag_id_map = HashMap::new();
    for tag in &export.tags {
        entities::tag::ActiveModel {
            id: Set(tag.id),
            site_id: Set(site_id),
            name: Set(tag.name.clone()),
        }
        .insert(db)
        .await?;
        tag_id_map.insert(tag.id, tag.id);
    }

    for content in &export.content_items {
        entities::content_item::ActiveModel {
            id: Set(content.id),
            site_id: Set(site_id),
            page_type: Set(content.page_type),
            title: Set(content.title.clone()),
            slug: Set(content.slug.clone()),
            page_content: Set(content.page_content.clone()),
            draft: Set(content.draft),
            creator_sub: Set(content.creator_sub.clone()),
            created_at: Set(content.created_at),
            last_updated: Set(content.last_updated),
            published_at: Set(content.published_at),
        }
        .insert(db)
        .await?;

        for alias in &content.aliases {
            entities::content_alias::ActiveModel {
                id: Set(alias.id),
                content_id: Set(content.id),
                site_id: Set(site_id),
                alias_path: Set(alias.alias_path.clone()),
                kind: Set(alias.kind.clone()),
            }
            .insert(db)
            .await?;
        }

        for tag in &content.tags {
            entities::content_tag::ActiveModel {
                id: Set(tag.id),
                content_id: Set(content.id),
                tag_id: Set(*tag_id_map.get(&tag.tag_id).ok_or_else(|| {
                    SiteError::BadRequest(format!(
                        "missing tag {} referenced by content {}",
                        tag.tag_id, content.id
                    ))
                })?),
            }
            .insert(db)
            .await?;
        }

        for revision in &content.revisions {
            entities::content_revision::ActiveModel {
                id: Set(revision.id),
                content_id: Set(content.id),
                site_id: Set(site_id),
                revision_number: Set(revision.revision_number),
                title: Set(revision.title.clone()),
                slug: Set(revision.slug.clone()),
                page_content: Set(revision.page_content.clone()),
                draft: Set(revision.draft),
                page_type: Set(revision.page_type),
                editor_sub: Set(revision.editor_sub.clone()),
                created_at: Set(revision.created_at),
            }
            .insert(db)
            .await?;

            for alias in &revision.aliases {
                entities::content_revision_alias::ActiveModel {
                    id: Set(alias.id),
                    revision_id: Set(revision.id),
                    content_id: Set(content.id),
                    alias_path: Set(alias.alias_path.clone()),
                    kind: Set(alias.kind.clone()),
                }
                .insert(db)
                .await?;
            }

            for tag in &revision.tags {
                entities::content_revision_tag::ActiveModel {
                    id: Set(tag.id),
                    revision_id: Set(revision.id),
                    content_id: Set(content.id),
                    tag_id: Set(*tag_id_map.get(&tag.tag_id).ok_or_else(|| {
                        SiteError::BadRequest(format!(
                            "missing tag {} referenced by revision {}",
                            tag.tag_id, revision.id
                        ))
                    })?),
                }
                .insert(db)
                .await?;
            }
        }
    }

    for asset in &export.assets {
        entities::asset::ActiveModel {
            id: Set(asset.id),
            site_id: Set(site_id),
            uploader_sub: Set(asset.uploader_sub.clone()),
            original_filename: Set(asset.original_filename.clone()),
            storage_basename: Set(asset.storage_basename.clone()),
            mime_type: Set(asset.mime_type.clone()),
            byte_length: Set(asset.byte_length),
            width: Set(asset.width),
            height: Set(asset.height),
            created_at: Set(asset.created_at),
        }
        .insert(db)
        .await?;

        for variant in &asset.variants {
            entities::asset_variant::ActiveModel {
                id: Set(variant.id),
                asset_id: Set(asset.id),
                variant_kind: Set(variant.variant_kind.clone()),
                filename: Set(variant.filename.clone()),
                mime_type: Set(variant.mime_type.clone()),
                byte_length: Set(variant.byte_length),
                width: Set(variant.width),
                height: Set(variant.height),
            }
            .insert(db)
            .await?;
        }
    }

    let old_site_id = export.site.id.to_string();
    for event in &export.audit_events {
        let entity_id = if event.entity_type == "site" && event.entity_id == old_site_id {
            site_id.to_string()
        } else {
            event.entity_id.clone()
        };

        // Audit event primary keys are internal identifiers, so imports assign fresh IDs.
        entities::audit_event::ActiveModel {
            id: Set(Uuid::now_v7()),
            site_id: Set(event.site_id.map(|_| site_id)),
            actor_sub: Set(event.actor_sub.clone()),
            event_type: Set(event.event_type.clone()),
            entity_type: Set(event.entity_type.clone()),
            entity_id: Set(entity_id),
            created_at: Set(event.created_at),
            payload_json: Set(event.payload_json.clone()),
        }
        .insert(db)
        .await?;
    }

    Ok(SiteImportResult {
        site_id,
        site_short_name: export.site.short_name.clone(),
        created_users,
        reused_users,
        warnings: build_import_warnings(export),
    })
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

fn validate_site_export(export: &SiteExport) -> Result<(), SiteError> {
    if export.format_version != SITE_EXPORT_FORMAT_VERSION {
        return Err(SiteError::BadRequest(format!(
            "unsupported site export format version: {}",
            export.format_version
        )));
    }

    if export.site.short_name.trim().is_empty() {
        return Err(SiteError::BadRequest(
            "site export is missing short_name".to_string(),
        ));
    }

    Ok(())
}

fn build_import_warnings(export: &SiteExport) -> Vec<String> {
    let mut warnings = Vec::new();
    let asset_file_count = export
        .assets
        .iter()
        .map(|asset| 1usize.saturating_add(asset.variants.len()))
        .sum::<usize>();
    if asset_file_count > 0 {
        warnings.push(format!(
            "{asset_file_count} asset file reference(s) were not restored because site exports contain file metadata only."
        ));
    }
    if !export.template_overrides.is_empty() {
        warnings.push(format!(
            "{} template override file(s) were not restored because site exports contain file metadata only.",
            export.template_overrides.len()
        ));
    }
    warnings
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
        create_membership, create_site, create_tag, list_assets, list_memberships, list_revisions,
        update_content,
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
                created_at: None,
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
    async fn export_and_import_preserve_rsync_publish_config() {
        let source_db = test_db_start().await;
        let site = create_site(
            &source_db,
            "publish-rsync".to_string(),
            "Publish Rsync".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create source site");
        crate::update_site_settings(
            &source_db,
            site.id,
            site.full_title.clone(),
            site.template_name.clone(),
            true,
            site.internal_domains.clone().into_vec(),
            site.mass_import_assets.clone(),
        )
        .await
        .expect("failed to enable publish on render");

        let config = RsyncPublishConfig {
            ssh_host: "example.com".to_string(),
            ssh_user: Some("deploy".to_string()),
            ssh_port: Some(2222),
            remote_path: "/var/www/example".to_string(),
            identity_file: Some("/tmp/id_ed25519".to_string()),
        };
        crate::publish::save_rsync_publish_config(&source_db, site.id, config.clone())
            .await
            .expect("failed to save rsync config");

        let export = export_site(&source_db, site.id)
            .await
            .expect("failed to export site");
        let publish_config = export
            .site
            .publish_config
            .as_ref()
            .expect("missing export publish config");
        assert_eq!(
            publish_config.method,
            entities::site_publish_config::PublishMethod::RsyncSsh
        );
        assert!(export.site.publish_on_render);
        assert_eq!(publish_config.rsync, Some(config.clone()));
        assert!(publish_config.s3.is_none());

        let target_db = test_db_start().await;
        let import = import_site_export(&target_db, &export)
            .await
            .expect("failed to import site export");
        let imported_config = entities::site_publish_config::Entity::find_by_id(import.site_id)
            .one(&target_db)
            .await
            .expect("failed to fetch imported config")
            .expect("missing imported config");
        assert_eq!(
            imported_config.method,
            entities::site_publish_config::PublishMethod::RsyncSsh
        );
        let imported_site = entities::site::Entity::find_by_id(import.site_id)
            .one(&target_db)
            .await
            .expect("failed to load imported site")
            .expect("missing imported site");
        assert!(imported_site.publish_on_render);
        let restored: RsyncPublishConfig =
            serde_json::from_value(imported_config.config_json).expect("deserialize rsync config");
        assert_eq!(restored, config);
    }

    #[tokio::test]
    async fn import_site_export_restores_database_records_and_reports_file_warnings() {
        let source_db = test_db_start().await;
        let upload_root = TempDir::new().expect("failed to create upload root");
        let override_root = TempDir::new().expect("failed to create override root");

        let site = create_site(
            &source_db,
            "roundtrip".to_string(),
            "Roundtrip Site".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create source site");
        let reused_user = crate::entities::user::create_user(
            &source_db,
            "existing-owner",
            Some("existing-owner@example.com"),
            Some("Existing Owner"),
            false,
        )
        .await
        .expect("failed to create existing source user");
        let new_user = crate::entities::user::create_user(
            &source_db,
            "new-editor",
            Some("new-editor@example.com"),
            Some("New Editor"),
            false,
        )
        .await
        .expect("failed to create new source user");
        create_membership(
            &source_db,
            NewMembership {
                site_id: site.id,
                user_id: reused_user.id,
                role: SiteRole::Owner,
            },
        )
        .await
        .expect("failed to create owner membership");
        create_membership(
            &source_db,
            NewMembership {
                site_id: site.id,
                user_id: new_user.id,
                role: SiteRole::Editor,
            },
        )
        .await
        .expect("failed to create editor membership");

        let tag = create_tag(
            &source_db,
            NewTag {
                site_id: site.id,
                name: "Roundtrip".to_string(),
            },
        )
        .await
        .expect("failed to create source tag");
        let content = create_content(
            &source_db,
            NewContent {
                site_id: site.id,
                page_type: entities::PageType::Page,
                title: "Landing".to_string(),
                slug: "landing".to_string(),
                page_content: "Source body".to_string(),
                draft: true,
                creator_sub: "existing-owner".to_string(),
                created_at: None,
                published_at: None,
            },
        )
        .await
        .expect("failed to create source content");
        create_alias(
            &source_db,
            NewAlias {
                content_id: content.id,
                site_id: site.id,
                alias_path: "/home".to_string(),
                kind: "alias".to_string(),
            },
        )
        .await
        .expect("failed to create source alias");
        add_content_tag(
            &source_db,
            NewContentTag {
                content_id: content.id,
                site_id: site.id,
                tag_name: tag.name.clone(),
            },
        )
        .await
        .expect("failed to tag source content");
        update_content(
            &source_db,
            crate::UpdateContent {
                content_id: content.id,
                page_type: None,
                title: Some("Landing Updated".to_string()),
                slug: Some("landing".to_string()),
                page_content: Some("Updated source body".to_string()),
                draft: Some(false),
                published_at: None,
                editor_sub: "new-editor".to_string(),
            },
        )
        .await
        .expect("failed to update source content");

        let asset = create_asset(
            &source_db,
            NewAsset {
                site_id: site.id,
                uploader_sub: "existing-owner".to_string(),
                original_filename: "banner.png".to_string(),
                storage_basename: "banner.png".to_string(),
                mime_type: "image/png".to_string(),
                byte_length: 100,
                width: Some(400),
                height: Some(200),
            },
        )
        .await
        .expect("failed to create source asset");
        create_asset_variant(
            &source_db,
            NewAssetVariant {
                asset_id: asset.id,
                variant_kind: "thumbnail".to_string(),
                filename: "banner-thumb.png".to_string(),
                mime_type: "image/png".to_string(),
                byte_length: 40,
                width: Some(100),
                height: Some(50),
            },
        )
        .await
        .expect("failed to create source asset variant");

        fs::write(upload_root.path().join("banner.png"), b"banner-bytes")
            .await
            .expect("failed to write source asset");
        fs::write(
            override_root.path().join("page.html"),
            b"<html>override</html>",
        )
        .await
        .expect("failed to write source template override");

        log_audit_event(
            &source_db,
            "existing-owner",
            "update_site",
            "site",
            site.id,
            Some(site.id),
            Some(serde_json::json!({ "full_title": "Roundtrip Site" })),
        )
        .await
        .expect("failed to write source audit event");

        let export = export_site_with_roots(
            &source_db,
            site.id,
            upload_root.path(),
            override_root.path(),
        )
        .await
        .expect("failed to export source site");

        let target_db = test_db_start().await;
        crate::entities::user::create_user(
            &target_db,
            "existing-owner",
            Some("target-existing@example.com"),
            Some("Target Existing Owner"),
            false,
        )
        .await
        .expect("failed to seed target existing user");

        let import = import_site_export(&target_db, &export)
            .await
            .expect("failed to import site export");

        assert_eq!(import.site_short_name, "roundtrip");
        assert_eq!(import.created_users, 1);
        assert_eq!(import.reused_users, 1);
        assert_eq!(import.warnings.len(), 2);
        assert!(import.warnings[0].contains("asset file reference"));
        assert!(import.warnings[1].contains("template override file"));

        let imported_site = entities::site::Entity::find_by_id(import.site_id)
            .one(&target_db)
            .await
            .expect("failed to fetch imported site")
            .expect("missing imported site");
        assert_eq!(imported_site.full_title, "Roundtrip Site");

        let imported_memberships = list_memberships(&target_db, imported_site.id)
            .await
            .expect("failed to list imported memberships");
        assert_eq!(imported_memberships.len(), 2);

        let imported_content = entities::content_item::Entity::find()
            .filter(entities::content_item::Column::SiteId.eq(imported_site.id))
            .all(&target_db)
            .await
            .expect("failed to list imported content");
        assert_eq!(imported_content.len(), 1);
        assert_eq!(imported_content[0].title, "Landing Updated");

        let imported_revisions = list_revisions(&target_db, imported_content[0].id)
            .await
            .expect("failed to list imported revisions");
        assert_eq!(imported_revisions.len(), 2);

        let imported_assets = list_assets(&target_db, imported_site.id)
            .await
            .expect("failed to list imported assets");
        assert_eq!(imported_assets.len(), 1);
        assert_eq!(imported_assets[0].original_filename, "banner.png");

        let imported_audit_events = list_audit_events(&target_db, Some(imported_site.id))
            .await
            .expect("failed to list imported audit events");
        assert_eq!(imported_audit_events.len(), 1);
        assert_ne!(
            imported_audit_events[0].id, export.audit_events[0].id,
            "imported audit events should receive fresh ids"
        );

        let imported_tags = entities::tag::Entity::find()
            .filter(entities::tag::Column::SiteId.eq(imported_site.id))
            .all(&target_db)
            .await
            .expect("failed to list imported tags");
        assert_eq!(imported_tags.len(), 1);
        assert_eq!(imported_tags[0].name, "Roundtrip");

        let imported_new_user = entities::user::Entity::find()
            .filter(entities::user::Column::Subject.eq("new-editor"))
            .one(&target_db)
            .await
            .expect("failed to fetch imported new user")
            .expect("missing imported new user");
        assert_eq!(
            imported_new_user.email.as_deref(),
            Some("new-editor@example.com")
        );
    }

    #[tokio::test]
    async fn import_site_export_rejects_duplicate_short_name() {
        let db = test_db_start().await;
        let export = SiteExport {
            format_version: SITE_EXPORT_FORMAT_VERSION,
            exported_at: Utc::now(),
            site: ExportSite {
                id: Uuid::now_v7(),
                short_name: "duplicate".to_string(),
                full_title: "Duplicate".to_string(),
                template_name: "default".to_string(),
                publish_on_render: false,
                internal_domains: Vec::new(),
                mass_import_assets: None,
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

        create_site(
            &db,
            "duplicate".to_string(),
            "Existing".to_string(),
            "default".to_string(),
        )
        .await
        .expect("failed to create existing site");

        let error = import_site_export(&db, &export)
            .await
            .expect_err("expected import to fail");
        match error {
            SiteError::BadRequest(message) => {
                assert!(message.contains("site short_name already exists"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn deserialize_site_export_rejects_unsupported_versions() {
        let payload = serde_json::json!({
            "format_version": SITE_EXPORT_FORMAT_VERSION + 1,
            "exported_at": Utc::now(),
            "site": {
                "id": Uuid::now_v7(),
                "short_name": "bad-version",
                "full_title": "Bad Version",
                "template_name": "default",
                "created_at": Utc::now(),
                "updated_at": serde_json::Value::Null
            },
            "memberships": [],
            "tags": [],
            "content_items": [],
            "assets": [],
            "audit_events": [],
            "template_overrides": []
        });

        let error = deserialize_site_export(payload.to_string().as_bytes())
            .expect_err("expected unsupported version");
        match error {
            SiteError::BadRequest(message) => {
                assert!(message.contains("unsupported site export format version"));
            }
            other => panic!("unexpected error: {other}"),
        }
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
                publish_on_render: false,
                internal_domains: Vec::new(),
                mass_import_assets: None,
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

        let json = serialize_site_export_pretty(&export).expect("failed to serialize export");
        assert!(json.contains(&format!(
            "\n  \"format_version\": {},",
            SITE_EXPORT_FORMAT_VERSION
        )));
    }
}
