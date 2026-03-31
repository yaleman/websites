use crate::entities;
use crate::entities::site_publish_config::{PublishMethod, PublishMethod::S3Compatible};
use crate::entities::site_publish_run::{PublishRunStatus, PublishRunStatus::*};
use crate::errors::SiteError;
use crate::render_site_into_dir;
use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use aws_types::region::Region;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::error::Error as StdError;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::fs;
use tokio::task;
use tracing::{debug, error, info};
use uuid::Uuid;

pub const DEFAULT_S3_REGION: &str = "us-east-1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3CompatiblePublishConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    #[serde(default)]
    pub force_path_style: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RsyncPublishConfig {
    pub ssh_host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    pub remote_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishOutcome {
    pub rendered_file_count: usize,
    pub published_file_count: usize,
    pub deleted_object_count: usize,
}

#[async_trait::async_trait]
pub trait PublishBackend: Send + Sync {
    async fn list_object_keys(&self, prefix: &str) -> Result<Vec<String>, SiteError>;
    async fn put_object(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), SiteError>;
    async fn delete_objects(&self, keys: &[String]) -> Result<(), SiteError>;
}

#[derive(Clone)]
pub struct S3PublishBackend {
    client: S3Client,
    bucket: String,
    endpoint_url: Option<String>,
    run_id: Uuid,
}

impl S3CompatiblePublishConfig {
    pub fn validate(&self) -> Result<(), SiteError> {
        if self
            .endpoint_url
            .as_deref()
            .is_some_and(|endpoint_url| endpoint_url.trim().is_empty())
        {
            return Err(SiteError::BadRequest(
                "publish endpoint_url cannot be blank".to_string(),
            ));
        }
        if self.bucket.trim().is_empty() {
            return Err(SiteError::BadRequest(
                "publish bucket is required".to_string(),
            ));
        }
        if self.region.trim().is_empty() {
            return Err(SiteError::BadRequest(
                "publish region is required".to_string(),
            ));
        }
        if self.access_key_id.trim().is_empty() {
            return Err(SiteError::BadRequest(
                "publish access_key_id is required".to_string(),
            ));
        }
        if self.secret_access_key.trim().is_empty() {
            return Err(SiteError::BadRequest(
                "publish secret_access_key is required".to_string(),
            ));
        }

        Ok(())
    }

    pub fn normalized_prefix(&self) -> String {
        self.prefix.trim().trim_matches('/').to_string()
    }
}

impl RsyncPublishConfig {
    pub fn validate(&self) -> Result<(), SiteError> {
        if self.ssh_host.trim().is_empty() {
            return Err(SiteError::BadRequest(
                "publish ssh_host is required".to_string(),
            ));
        }
        if self.remote_path.trim().is_empty() {
            return Err(SiteError::BadRequest(
                "publish remote_path is required".to_string(),
            ));
        }
        if self
            .identity_file
            .as_deref()
            .is_some_and(|identity_file| identity_file.trim().is_empty())
        {
            return Err(SiteError::BadRequest(
                "publish identity_file cannot be blank".to_string(),
            ));
        }

        Ok(())
    }

    pub fn normalized_remote_path(&self) -> String {
        self.remote_path.trim().trim_end_matches('/').to_string()
    }

    pub fn ssh_target(&self) -> String {
        let host = self.ssh_host.trim();
        if let Some(user) = self
            .ssh_user
            .as_deref()
            .map(str::trim)
            .filter(|user| !user.is_empty())
        {
            format!("{user}@{host}")
        } else {
            host.to_string()
        }
    }
}

impl S3PublishBackend {
    pub async fn from_config(
        config: &S3CompatiblePublishConfig,
        run_id: Uuid,
    ) -> Result<Self, SiteError> {
        config.validate()?;
        let shared_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .credentials_provider(Credentials::new(
                config.access_key_id.clone(),
                config.secret_access_key.clone(),
                None,
                None,
                "site-publish",
            ))
            .load()
            .await;
        let mut builder = aws_sdk_s3::config::Builder::from(&shared_config);
        if let Some(endpoint_url) = &config.endpoint_url {
            builder = builder.endpoint_url(endpoint_url.clone());
        }
        builder = builder.force_path_style(config.force_path_style);
        let client = S3Client::from_conf(builder.build());

        Ok(Self {
            client,
            bucket: config.bucket.clone(),
            endpoint_url: config.endpoint_url.clone(),
            run_id,
        })
    }
}

fn error_chain(error: &dyn StdError) -> String {
    let mut messages = vec![error.to_string()];
    let mut current = error.source();
    while let Some(source) = current {
        messages.push(source.to_string());
        current = source.source();
    }
    messages.join(": ")
}

#[async_trait::async_trait]
impl PublishBackend for S3PublishBackend {
    async fn list_object_keys(&self, prefix: &str) -> Result<Vec<String>, SiteError> {
        let prefix = normalize_prefix(prefix);
        let mut continuation_token = None;
        let mut keys = Vec::new();

        loop {
            debug!(
                run_id = %self.run_id,
                bucket = %self.bucket,
                prefix = %prefix,
                endpoint_url = self.endpoint_url.as_deref().unwrap_or("aws-default"),
                "listing published objects"
            );
            let response = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix.clone())
                .set_continuation_token(continuation_token.take())
                .send()
                .await
                .map_err(|error| {
                    let detail = error_chain(&error);
                    error!(
                        run_id = %self.run_id,
                        bucket = %self.bucket,
                        prefix = %prefix,
                        endpoint_url = self.endpoint_url.as_deref().unwrap_or("aws-default"),
                        error = %detail,
                        "list s3 objects failed"
                    );
                    SiteError::internal(format!("list s3 objects failed: {detail}"))
                })?;

            if let Some(objects) = response.contents {
                for object in objects {
                    if let Some(key) = object.key {
                        keys.push(key);
                    }
                }
            }

            if response.is_truncated.unwrap_or(false) {
                continuation_token = response.next_continuation_token;
            } else {
                break;
            }
        }

        Ok(keys)
    }

    async fn put_object(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), SiteError> {
        debug!(
            run_id = %self.run_id,
            bucket = %self.bucket,
            key = %key,
            content_type = %content_type,
            byte_length = bytes.len(),
            endpoint_url = self.endpoint_url.as_deref().unwrap_or("aws-default"),
            "uploading published object"
        );
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(bytes))
            .content_type(content_type)
            .send()
            .await
            .map_err(|error| {
                let detail = error_chain(&error);
                error!(
                    run_id = %self.run_id,
                    bucket = %self.bucket,
                    key = %key,
                    content_type = %content_type,
                    endpoint_url = self.endpoint_url.as_deref().unwrap_or("aws-default"),
                    error = %detail,
                    "s3 upload failed"
                );
                SiteError::internal(format!("s3 upload failed: {detail}"))
            })?;

        Ok(())
    }

    async fn delete_objects(&self, keys: &[String]) -> Result<(), SiteError> {
        if keys.is_empty() {
            return Ok(());
        }

        for chunk in keys.chunks(1000) {
            debug!(
                run_id = %self.run_id,
                bucket = %self.bucket,
                endpoint_url = self.endpoint_url.as_deref().unwrap_or("aws-default"),
                delete_count = chunk.len(),
                "deleting stale published objects"
            );
            let objects = chunk
                .iter()
                .map(|key| {
                    ObjectIdentifier::builder().key(key).build().map_err(|err| {
                        SiteError::internal(format!(
                            "Failed to build s3 object identifier: {err:?}"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, SiteError>>()?;
            let delete = Delete::builder()
                .set_objects(Some(objects))
                .quiet(true)
                .build()
                .map_err(|err| {
                    SiteError::internal(format!("Failed to build s3 delete request: {err:?}"))
                })?;
            self.client
                .delete_objects()
                .bucket(&self.bucket)
                .delete(delete)
                .send()
                .await
                .map_err(|error| {
                    let detail = error_chain(&error);
                    error!(
                        run_id = %self.run_id,
                        bucket = %self.bucket,
                        endpoint_url = self.endpoint_url.as_deref().unwrap_or("aws-default"),
                        delete_count = chunk.len(),
                        error = %detail,
                        "s3 delete failed"
                    );
                    SiteError::internal(format!("s3 delete failed: {detail}"))
                })?;
        }

        Ok(())
    }
}

pub async fn get_site_publish_config<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
) -> Result<Option<entities::site_publish_config::Model>, SiteError> {
    entities::site_publish_config::Entity::find_by_id(site_id)
        .one(db)
        .await
        .map_err(SiteError::from)
}

fn deserialize_publish_config<T: for<'de> Deserialize<'de>>(
    config: &entities::site_publish_config::Model,
) -> Result<T, SiteError> {
    serde_json::from_value(config.config_json.clone())
        .map_err(|error| SiteError::BadRequest(format!("invalid publish config: {error}")))
}

pub async fn get_s3_publish_config(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> Result<Option<S3CompatiblePublishConfig>, SiteError> {
    let Some(config) = get_site_publish_config(db, site_id).await? else {
        return Ok(None);
    };

    if config.method != S3Compatible {
        return Err(SiteError::BadRequest(format!(
            "unsupported publish method: {}",
            config.method
        )));
    }

    serde_json::from_value(config.config_json)
        .map(Some)
        .map_err(|error| SiteError::BadRequest(format!("invalid publish config: {error}")))
}

pub async fn get_rsync_publish_config(
    db: &DatabaseConnection,
    site_id: Uuid,
) -> Result<Option<RsyncPublishConfig>, SiteError> {
    let Some(config) = get_site_publish_config(db, site_id).await? else {
        return Ok(None);
    };

    if config.method != PublishMethod::RsyncSsh {
        return Err(SiteError::BadRequest(format!(
            "unsupported publish method: {}",
            config.method
        )));
    }

    deserialize_publish_config(&config).map(Some)
}

async fn save_publish_config<C: ConnectionTrait, T: Serialize>(
    db: &C,
    site_id: Uuid,
    method: PublishMethod,
    config: T,
) -> Result<entities::site_publish_config::Model, SiteError> {
    let now = Utc::now();
    let config_json = serde_json::to_value(config).map_err(|error| {
        SiteError::internal(format!("failed to serialize publish config: {error}"))
    })?;

    let existing = entities::site_publish_config::Entity::find_by_id(site_id)
        .one(db)
        .await
        .map_err(SiteError::from)?;

    if let Some(existing) = existing {
        let mut active = existing.into_active_model();
        active.method = Set(method);
        active.config_json = Set(config_json);
        active.updated_at = Set(now);
        active.update(db).await.map_err(SiteError::from)
    } else {
        entities::site_publish_config::ActiveModel {
            site_id: Set(site_id),
            method: Set(method),
            config_json: Set(config_json),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .map_err(SiteError::from)
    }
}

pub async fn save_s3_publish_config<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    config: S3CompatiblePublishConfig,
) -> Result<entities::site_publish_config::Model, SiteError> {
    config.validate()?;
    save_publish_config(db, site_id, PublishMethod::S3Compatible, config).await
}

pub async fn save_rsync_publish_config<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    config: RsyncPublishConfig,
) -> Result<entities::site_publish_config::Model, SiteError> {
    config.validate()?;
    save_publish_config(db, site_id, PublishMethod::RsyncSsh, config).await
}

pub async fn delete_site_publish_config<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
) -> Result<(), SiteError> {
    entities::site_publish_config::Entity::delete_by_id(site_id)
        .exec(db)
        .await
        .map_err(SiteError::from)?;
    Ok(())
}

fn publish_config_summary_for_log(
    config: &entities::site_publish_config::Model,
) -> Result<Vec<(&'static str, String)>, SiteError> {
    match config.method {
        PublishMethod::S3Compatible => {
            let parsed = deserialize_publish_config::<S3CompatiblePublishConfig>(config)?;
            let prefix = parsed.normalized_prefix();
            let endpoint_url = parsed
                .endpoint_url
                .unwrap_or_else(|| "aws-default".to_string());
            let bucket = parsed.bucket;
            Ok(vec![
                ("method", config.method.to_string()),
                ("bucket", bucket),
                ("prefix", prefix),
                ("endpoint_url", endpoint_url),
            ])
        }
        PublishMethod::RsyncSsh => {
            let parsed = deserialize_publish_config::<RsyncPublishConfig>(config)?;
            Ok(vec![
                ("method", config.method.to_string()),
                ("ssh_host", parsed.ssh_target()),
                ("remote_path", parsed.normalized_remote_path()),
                ("identity_file", parsed.identity_file.unwrap_or_default()),
            ])
        }
        PublishMethod::Disabled => Ok(vec![("method", config.method.to_string())]),
    }
}

pub async fn list_site_publish_runs(
    db: &DatabaseConnection,
    site_id: Uuid,
    limit: u64,
) -> Result<Vec<entities::site_publish_run::Model>, SiteError> {
    entities::site_publish_run::Entity::find()
        .filter(entities::site_publish_run::Column::SiteId.eq(site_id))
        .order_by_desc(entities::site_publish_run::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await
        .map_err(SiteError::from)
}

pub async fn get_site_publish_run(
    db: &DatabaseConnection,
    site_id: Uuid,
    run_id: Uuid,
) -> Result<Option<entities::site_publish_run::Model>, SiteError> {
    entities::site_publish_run::Entity::find_by_id(run_id)
        .filter(entities::site_publish_run::Column::SiteId.eq(site_id))
        .one(db)
        .await
        .map_err(SiteError::from)
}

async fn create_publish_run<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    method: PublishMethod,
    actor_sub: &str,
) -> Result<entities::site_publish_run::Model, SiteError> {
    let now = Utc::now();
    entities::site_publish_run::ActiveModel {
        id: Set(Uuid::now_v7()),
        site_id: Set(site_id),
        method: Set(method),
        status: Set(PublishRunStatus::Queued),
        actor_sub: Set(actor_sub.to_string()),
        rendered_file_count: Set(0),
        published_file_count: Set(0),
        deleted_object_count: Set(0),
        error_message: Set(None),
        created_at: Set(now),
        started_at: Set(None),
        finished_at: Set(None),
    }
    .insert(db)
    .await
    .map_err(SiteError::from)
}

async fn update_publish_run<C: ConnectionTrait>(
    db: &C,
    run_id: Uuid,
    status: PublishRunStatus,
    rendered_file_count: i32,
    published_file_count: i32,
    deleted_object_count: i32,
    error_message: Option<String>,
) -> Result<entities::site_publish_run::Model, SiteError> {
    let Some(existing) = entities::site_publish_run::Entity::find_by_id(run_id)
        .one(db)
        .await
        .map_err(SiteError::from)?
    else {
        return Err(SiteError::NotFound);
    };

    let now = Utc::now();
    let mut active = existing.into_active_model();
    active.status = Set(status);
    active.rendered_file_count = Set(rendered_file_count);
    active.published_file_count = Set(published_file_count);
    active.deleted_object_count = Set(deleted_object_count);
    active.error_message = Set(error_message);
    if matches!(status, Running) {
        active.started_at = Set(Some(now));
    }
    if matches!(status, Succeeded | Failed) {
        active.finished_at = Set(Some(now));
    }
    active.update(db).await.map_err(SiteError::from)
}

pub async fn queue_site_publish(
    db: Arc<DatabaseConnection>,
    site_id: Uuid,
    actor_sub: String,
    site_templates_root: PathBuf,
    upload_root: PathBuf,
) -> Result<entities::site_publish_run::Model, SiteError> {
    let site = entities::site::Entity::find_by_id(site_id)
        .one(db.as_ref())
        .await
        .map_err(SiteError::from)?
        .ok_or_else(|| SiteError::SiteNotFound(site_id.to_string()))?;
    let Some(config) = get_site_publish_config(db.as_ref(), site_id).await? else {
        return Err(SiteError::BadRequest(
            "site publish configuration has not been saved yet".to_string(),
        ));
    };

    if config.method == PublishMethod::Disabled {
        return Err(SiteError::BadRequest(
            "site publish configuration is disabled".to_string(),
        ));
    }

    let run = create_publish_run(db.as_ref(), site_id, config.method, &actor_sub).await?;
    let run_id = run.id;
    let site_id_for_log = site.id;
    let site_short_name_for_log = site.short_name.clone();
    let db_for_job = Arc::clone(&db);
    let db_for_failure = Arc::clone(&db);
    let site_templates_root = site_templates_root.clone();
    let upload_root = upload_root.clone();

    let method_for_log = config.method.to_string();
    let config_summary = publish_config_summary_for_log(&config)?;
    info!(
        %run_id,
        site_id = %site_id_for_log,
        site_short_name = %site_short_name_for_log,
        method = %config.method,
        summary = ?config_summary,
        "publish job queued"
    );

    tokio::spawn(async move {
        if let Err(error) = publish_site_job(
            db_for_job,
            run_id,
            site,
            config,
            site_templates_root,
            upload_root,
        )
        .await
        {
            let _ = update_publish_run(
                db_for_failure.as_ref(),
                run_id,
                PublishRunStatus::Failed,
                0,
                0,
                0,
                Some(error.to_string()),
            )
            .await;
            error!(
                %run_id,
                site_id = %site_id_for_log,
                site_short_name = %site_short_name_for_log,
                method = %method_for_log,
                summary = ?config_summary,
                error = %error,
                "publish job failed"
            );
        }
    });

    Ok(run)
}

pub async fn publish_rendered_site<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
    actor_sub: String,
    rendered_root: &Path,
) -> Result<PublishOutcome, SiteError> {
    let site = entities::site::Entity::find_by_id(site_id)
        .one(db)
        .await
        .map_err(SiteError::from)?
        .ok_or_else(|| SiteError::SiteNotFound(site_id.to_string()))?;
    let Some(config) = get_site_publish_config(db, site_id).await? else {
        return Err(SiteError::BadRequest(
            "site publish configuration has not been saved yet".to_string(),
        ));
    };

    if config.method == PublishMethod::Disabled {
        return Err(SiteError::BadRequest(
            "site publish configuration is disabled".to_string(),
        ));
    }

    let rendered_root = rendered_root.join(&site.short_name);

    let run = create_publish_run(db, site_id, config.method, &actor_sub).await?;
    let run_id = run.id;
    update_publish_run(db, run_id, PublishRunStatus::Running, 0, 0, 0, None).await?;

    match publish_rendered_tree_with_config(&rendered_root, &site, &config, run_id).await {
        Ok(outcome) => {
            let rendered_count = i32::try_from(outcome.rendered_file_count).unwrap_or(i32::MAX);
            let published_count = i32::try_from(outcome.published_file_count).unwrap_or(i32::MAX);
            let deleted_count = i32::try_from(outcome.deleted_object_count).unwrap_or(i32::MAX);
            update_publish_run(
                db,
                run_id,
                PublishRunStatus::Succeeded,
                rendered_count,
                published_count,
                deleted_count,
                None,
            )
            .await?;
            info!(
                %run_id,
                site_id = %site.id,
                site_short_name = %site.short_name,
                rendered_file_count = rendered_count,
                published_file_count = published_count,
                deleted_object_count = deleted_count,
                "site publish completed"
            );
            Ok(outcome)
        }
        Err(error) => {
            update_publish_run(
                db,
                run_id,
                PublishRunStatus::Failed,
                0,
                0,
                0,
                Some(error.to_string()),
            )
            .await?;
            Err(error)
        }
    }
}

async fn publish_site_job(
    db: Arc<DatabaseConnection>,
    run_id: Uuid,
    site: entities::site::Model,
    config: entities::site_publish_config::Model,
    site_templates_root: PathBuf,
    upload_root: PathBuf,
) -> Result<(), SiteError> {
    update_publish_run(
        db.as_ref(),
        run_id,
        PublishRunStatus::Running,
        0,
        0,
        0,
        None,
    )
    .await?;

    let tmp_dir = TempDir::new().map_err(SiteError::from)?;
    let override_root =
        crate::resolve_site_template_override_root_with_upload_root(&upload_root, site.id);
    let files_written = render_site_into_dir(
        db.as_ref(),
        site.id,
        site_templates_root.as_path(),
        tmp_dir.path(),
        &upload_root,
        &override_root,
    )
    .await?;

    let outcome = publish_rendered_tree_with_config(tmp_dir.path(), &site, &config, run_id).await?;

    let rendered_count = i32::try_from(files_written).unwrap_or(i32::MAX);
    let published_count = i32::try_from(outcome.published_file_count).unwrap_or(i32::MAX);
    let deleted_count = i32::try_from(outcome.deleted_object_count).unwrap_or(i32::MAX);

    update_publish_run(
        db.as_ref(),
        run_id,
        PublishRunStatus::Succeeded,
        rendered_count,
        published_count,
        deleted_count,
        None,
    )
    .await?;

    info!(
        %run_id,
        site_id = %site.id,
        site_short_name = %site.short_name,
        rendered_file_count = rendered_count,
        published_file_count = published_count,
        deleted_object_count = deleted_count,
        "site publish completed"
    );

    Ok(())
}

async fn publish_rendered_tree_with_config(
    root: &Path,
    site: &entities::site::Model,
    config: &entities::site_publish_config::Model,
    run_id: Uuid,
) -> Result<PublishOutcome, SiteError> {
    match config.method {
        PublishMethod::S3Compatible => {
            let config = deserialize_publish_config::<S3CompatiblePublishConfig>(config)?;
            let backend = S3PublishBackend::from_config(&config, run_id).await?;
            info!(
                %run_id,
                site_id = %site.id,
                site_short_name = %site.short_name,
                bucket = %config.bucket,
                prefix = %config.normalized_prefix(),
                endpoint_url = config.endpoint_url.as_deref().unwrap_or("aws-default"),
                "starting site publish"
            );
            mirror_rendered_tree_to_backend(root, &config.normalized_prefix(), &backend).await
        }
        PublishMethod::RsyncSsh => {
            let config = deserialize_publish_config::<RsyncPublishConfig>(config)?;
            info!(
                %run_id,
                site_id = %site.id,
                site_short_name = %site.short_name,
                ssh_target = %config.ssh_target(),
                remote_path = %config.normalized_remote_path(),
                "starting site publish"
            );
            publish_rendered_tree_via_rsync(root, &config, run_id).await
        }
        PublishMethod::Disabled => Err(SiteError::BadRequest(
            "site publish configuration is disabled".to_string(),
        )),
    }
}

pub async fn mirror_rendered_tree_to_backend<B: PublishBackend>(
    root: &Path,
    prefix: &str,
    backend: &B,
) -> Result<PublishOutcome, SiteError> {
    let files = collect_rendered_files(root).await?;
    let mut expected_keys = HashSet::with_capacity(files.len());

    for file in &files {
        let key = object_key_for_path(prefix, &file.relative_path);
        let bytes = fs::read(&file.absolute_path).await?;
        let content_type = mime_guess::from_path(&file.absolute_path)
            .first_or_octet_stream()
            .essence_str()
            .to_string();
        debug!(
            relative_path = %file.relative_path.display(),
            key = %key,
            content_type = %content_type,
            byte_length = bytes.len(),
            "publishing rendered file"
        );
        backend.put_object(&key, bytes, &content_type).await?;
        expected_keys.insert(key);
    }

    let remote_keys = backend.list_object_keys(prefix).await?;
    let stale_keys = remote_keys
        .into_iter()
        .filter(|key| !expected_keys.contains(key))
        .collect::<Vec<_>>();
    backend.delete_objects(&stale_keys).await?;

    Ok(PublishOutcome {
        rendered_file_count: files.len(),
        published_file_count: files.len(),
        deleted_object_count: stale_keys.len(),
    })
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_./:@".contains(character))
    {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', r"'\''"))
}

fn rsync_binary_path() -> String {
    env::var("WEBSITES_RSYNC_BIN").unwrap_or_else(|_| "rsync".to_string())
}

fn build_rsync_destination(config: &RsyncPublishConfig) -> String {
    let remote_path = config.normalized_remote_path();
    format!("{}:{remote_path}/", config.ssh_target())
}

fn build_ssh_command(config: &RsyncPublishConfig) -> String {
    let mut args = vec![
        "ssh".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
    ];
    if let Some(port) = config.ssh_port {
        args.push("-p".to_string());
        args.push(port.to_string());
    }
    if let Some(identity_file) = config
        .identity_file
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push("-i".to_string());
        args.push(shell_quote(identity_file));
    }
    args.join(" ")
}

fn build_rsync_args(source_dir: &Path, config: &RsyncPublishConfig) -> Vec<String> {
    vec![
        "-a".to_string(),
        "--delete".to_string(),
        "--stats".to_string(),
        "-e".to_string(),
        build_ssh_command(config),
        format!("{}/", source_dir.display()),
        build_rsync_destination(config),
    ]
}

fn parse_rsync_stats(output: &str) -> (usize, usize) {
    let mut transferred = 0usize;
    let mut deleted = 0usize;

    for line in output.lines() {
        let line = line.trim();
        let transferred_prefixes = [
            "Number of regular files transferred: ",
            "Number of files transferred: ",
        ];
        for prefix in transferred_prefixes {
            if let Some(value) = line.strip_prefix(prefix)
                && let Ok(parsed) = value.trim().parse::<usize>()
            {
                transferred = parsed;
            }
        }
        let deleted_prefixes = ["Number of deleted files: ", "Total deleted files: "];
        for prefix in deleted_prefixes {
            if let Some(value) = line.strip_prefix(prefix)
                && let Ok(parsed) = value.trim().parse::<usize>()
            {
                deleted = parsed;
            }
        }
    }

    (transferred, deleted)
}

fn run_rsync_command(
    source_dir: &Path,
    config: &RsyncPublishConfig,
) -> Result<std::process::Output, SiteError> {
    let mut command = Command::new(rsync_binary_path());
    command.args(build_rsync_args(source_dir, config));
    command
        .output()
        .map_err(|error| SiteError::internal(format!("failed to execute rsync: {error}")))
}

async fn publish_rendered_tree_via_rsync(
    root: &Path,
    config: &RsyncPublishConfig,
    run_id: Uuid,
) -> Result<PublishOutcome, SiteError> {
    let root = root.to_path_buf();
    let rendered_file_count = collect_rendered_files(root.as_path()).await?.len();
    let root_for_command = root.clone();
    let config_for_command = config.clone();
    let output =
        task::spawn_blocking(move || run_rsync_command(&root_for_command, &config_for_command))
            .await
            .map_err(|error| SiteError::internal(format!("rsync task failed: {error}")))??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let (published_file_count, deleted_object_count) =
        parse_rsync_stats(&format!("{stdout}\n{stderr}"));
    if !output.status.success() {
        return Err(SiteError::internal(format!(
            "rsync publish failed: status={} stdout={} stderr={}",
            output.status,
            stdout.trim(),
            stderr.trim()
        )));
    }

    info!(
        %run_id,
        ssh_target = %config.ssh_target(),
        remote_path = %config.normalized_remote_path(),
        published_file_count,
        deleted_object_count,
        "rsync publish completed"
    );

    Ok(PublishOutcome {
        rendered_file_count,
        published_file_count,
        deleted_object_count,
    })
}

#[derive(Debug, Clone)]
struct RenderedFile {
    relative_path: PathBuf,
    absolute_path: PathBuf,
}

async fn collect_rendered_files(root: &Path) -> Result<Vec<RenderedFile>, SiteError> {
    let mut files = Vec::new();
    let mut directories = vec![root.to_path_buf()];

    while let Some(directory) = directories.pop() {
        let mut entries = match fs::read_dir(&directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(SiteError::from(error)),
        };

        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            let path = entry.path();
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file() {
                let relative_path = path.strip_prefix(root).map_err(|error| {
                    SiteError::internal(format!("invalid render tree path: {error}"))
                })?;
                files.push(RenderedFile {
                    relative_path: relative_path.to_path_buf(),
                    absolute_path: path,
                });
            }
        }
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn object_key_for_path(prefix: &str, relative_path: &Path) -> String {
    let relative = relative_path.to_string_lossy().replace('\\', "/");
    let prefix = normalize_prefix(prefix);
    if prefix.is_empty() {
        relative
    } else {
        format!("{prefix}/{relative}")
    }
}

fn normalize_prefix(prefix: &str) -> String {
    prefix.trim().trim_matches('/').to_string()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::db::test_db_start;
    use crate::entities::site::Entity as SiteEntity;
    use crate::{create_site, render_site};
    use sea_orm::EntityTrait;
    use std::collections::HashMap;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[derive(Default)]
    struct FakePublishBackend {
        objects: Mutex<HashMap<String, (Vec<u8>, String)>>,
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
            // SAFETY: this test runs in isolation and restores the variable on drop.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    #[cfg(unix)]
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: this restores the process environment to its prior state.
            unsafe {
                if let Some(previous) = &self.previous {
                    std::env::set_var(self.key, previous);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn create_fake_identity_file() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("create identity dir");
        let path = dir.path().join("id_ed25519");
        std::fs::write(&path, b"fake-identity").expect("write fake identity file");
        (dir, path)
    }

    #[cfg(unix)]
    fn write_fake_rsync_binary(path: &std::path::Path) {
        let script = r#"#!/usr/bin/env bash
set -euo pipefail
source_dir="${@: -2:1}"
destination="${@: -1}"
source_dir="${source_dir%/}"
remote_path="${destination#*:}"
remote_path="${remote_path%/}"
mkdir -p "$remote_path"
if [ -d "$remote_path" ]; then
  find "$remote_path" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
fi
cp -R "$source_dir"/. "$remote_path"/
transferred=$(find "$source_dir" -type f | wc -l | tr -d ' ')
deleted=1
printf 'Number of files transferred: %s\n' "$transferred"
printf 'Number of deleted files: %s\n' "$deleted"
"#;
        fs::write(path, script).expect("write fake rsync binary");
        let mut permissions = fs::metadata(path)
            .expect("fake rsync metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("chmod fake rsync binary");
    }

    #[async_trait::async_trait]
    impl PublishBackend for FakePublishBackend {
        async fn list_object_keys(&self, prefix: &str) -> Result<Vec<String>, SiteError> {
            let prefix = normalize_prefix(prefix);
            let objects = self.objects.lock().expect("lock fake backend");
            Ok(objects
                .keys()
                .filter(|key| prefix.is_empty() || key.starts_with(&prefix))
                .cloned()
                .collect())
        }

        async fn put_object(
            &self,
            key: &str,
            bytes: Vec<u8>,
            content_type: &str,
        ) -> Result<(), SiteError> {
            self.objects
                .lock()
                .expect("lock fake backend")
                .insert(key.to_string(), (bytes, content_type.to_string()));
            Ok(())
        }

        async fn delete_objects(&self, keys: &[String]) -> Result<(), SiteError> {
            let mut objects = self.objects.lock().expect("lock fake backend");
            for key in keys {
                objects.remove(key);
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn mirror_rendered_tree_uploads_and_deletes_stale_objects() {
        let root = TempDir::new().expect("create temp root");
        fs::create_dir_all(root.path().join("nested")).expect("create nested dir");
        fs::write(root.path().join("index.html"), b"hello").expect("write index");
        fs::write(root.path().join("nested").join("app.css"), b"body{}").expect("write css");

        let backend = FakePublishBackend::default();
        backend
            .put_object("old.txt", b"old".to_vec(), "text/plain")
            .await
            .expect("seed fake backend");

        let outcome = mirror_rendered_tree_to_backend(root.path(), "", &backend)
            .await
            .expect("mirror tree");

        assert_eq!(outcome.rendered_file_count, 2);
        assert_eq!(outcome.published_file_count, 2);
        assert_eq!(outcome.deleted_object_count, 1);

        let objects = backend.objects.lock().expect("lock fake backend");
        assert!(objects.contains_key("index.html"));
        assert!(objects.contains_key("nested/app.css"));
        assert!(!objects.contains_key("old.txt"));
        assert_eq!(
            objects.get("nested/app.css").expect("css object").1,
            "text/css"
        );
    }

    #[tokio::test]
    async fn normalize_prefix_trims_slashes() {
        assert_eq!(normalize_prefix("/foo/bar/"), "foo/bar");
        assert_eq!(normalize_prefix(""), "");
    }

    #[tokio::test]
    async fn s3_config_round_trips_through_json() {
        let config = S3CompatiblePublishConfig {
            endpoint_url: None,
            bucket: "bucket".to_string(),
            prefix: "site".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: "key".to_string(),
            secret_access_key: "secret".to_string(),
            force_path_style: true,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        let decoded: S3CompatiblePublishConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(decoded, config);
    }

    #[tokio::test]
    async fn rsync_config_round_trips_through_json() {
        let (_identity_dir, identity_file) = create_fake_identity_file();
        let config = RsyncPublishConfig {
            ssh_host: "example.com".to_string(),
            ssh_user: Some("deploy".to_string()),
            ssh_port: Some(2222),
            remote_path: "/var/www/example".to_string(),
            identity_file: Some(identity_file.display().to_string()),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        let decoded: RsyncPublishConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(decoded, config);
    }

    #[tokio::test]
    async fn build_rsync_args_sets_ssh_transport() {
        let config = RsyncPublishConfig {
            ssh_host: "example.com".to_string(),
            ssh_user: Some("deploy".to_string()),
            ssh_port: Some(2222),
            remote_path: "/var/www/example".to_string(),
            identity_file: Some("/tmp/id file".to_string()),
        };
        let args = build_rsync_args(Path::new("/tmp/rendered"), &config);
        assert_eq!(
            args,
            vec![
                "-a",
                "--delete",
                "--stats",
                "-e",
                "ssh -o BatchMode=yes -p 2222 -i '/tmp/id file'",
                "/tmp/rendered/",
                "deploy@example.com:/var/www/example/",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn parse_rsync_stats_reads_transferred_and_deleted_counts() {
        let (transferred, deleted) =
            parse_rsync_stats("Number of files transferred: 4\nNumber of deleted files: 2\n");
        assert_eq!(transferred, 4);
        assert_eq!(deleted, 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn publish_rendered_tree_via_rsync_mirrors_tree() {
        let _env_lock = crate::test_support::env_lock().lock_owned().await;
        let source = TempDir::new().expect("create source dir");
        let remote_root = TempDir::new().expect("create remote dir");
        let script_root = TempDir::new().expect("create script dir");
        fs::create_dir_all(source.path().join("nested")).expect("create source nested dir");
        fs::write(source.path().join("index.html"), b"hello").expect("write source file");
        fs::write(source.path().join("nested").join("app.css"), b"body{}")
            .expect("write nested source file");
        fs::write(remote_root.path().join("stale.txt"), b"stale").expect("seed stale remote file");

        let rsync_bin = script_root.path().join("fake-rsync.sh");
        write_fake_rsync_binary(&rsync_bin);
        let _guard = EnvVarGuard::set(
            "WEBSITES_RSYNC_BIN",
            rsync_bin.to_str().expect("fake rsync path"),
        );

        let config = RsyncPublishConfig {
            ssh_host: "localhost".to_string(),
            ssh_user: None,
            ssh_port: None,
            remote_path: remote_root
                .path()
                .to_str()
                .expect("remote path")
                .to_string(),
            identity_file: None,
        };
        let outcome = publish_rendered_tree_via_rsync(source.path(), &config, Uuid::now_v7())
            .await
            .expect("publish via rsync");

        assert_eq!(outcome.rendered_file_count, 2);
        assert_eq!(outcome.published_file_count, 2);
        assert_eq!(outcome.deleted_object_count, 1);
        assert!(remote_root.path().join("index.html").exists());
        assert!(remote_root.path().join("nested").join("app.css").exists());
        assert!(!remote_root.path().join("stale.txt").exists());
    }

    #[tokio::test]
    async fn publish_config_round_trips_through_db() {
        let db = test_db_start().await;
        let site = create_site(
            &db,
            "publish-config".to_string(),
            "Publish Config".to_string(),
            "default".to_string(),
        )
        .await
        .expect("create site");

        let config = S3CompatiblePublishConfig {
            endpoint_url: None,
            bucket: "bucket".to_string(),
            prefix: "site".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: "key".to_string(),
            secret_access_key: "secret".to_string(),
            force_path_style: true,
        };

        save_s3_publish_config(&db, site.id, config.clone())
            .await
            .expect("save config");

        let loaded = get_s3_publish_config(&db, site.id)
            .await
            .expect("load config")
            .expect("missing config");
        assert_eq!(loaded, config);

        delete_site_publish_config(&db, site.id)
            .await
            .expect("delete config");
        assert!(
            get_s3_publish_config(&db, site.id)
                .await
                .expect("load deleted config")
                .is_none()
        );
    }

    #[tokio::test]
    async fn rsync_publish_config_round_trips_through_db() {
        let db = test_db_start().await;
        let site = create_site(
            &db,
            "publish-rsync-config".to_string(),
            "Publish Rsync Config".to_string(),
            "default".to_string(),
        )
        .await
        .expect("create site");
        let (_identity_dir, identity_file) = create_fake_identity_file();

        let config = RsyncPublishConfig {
            ssh_host: "example.com".to_string(),
            ssh_user: Some("deploy".to_string()),
            ssh_port: Some(2222),
            remote_path: "/var/www/example".to_string(),
            identity_file: Some(identity_file.display().to_string()),
        };

        save_rsync_publish_config(&db, site.id, config.clone())
            .await
            .expect("save config");

        let loaded = get_rsync_publish_config(&db, site.id)
            .await
            .expect("load config")
            .expect("missing config");
        assert_eq!(loaded, config);
    }

    #[tokio::test]
    async fn render_site_outputs_files() {
        let db = test_db_start().await;
        let site = create_site(
            &db,
            "publish-test".to_string(),
            "Publish Test".to_string(),
            "default".to_string(),
        )
        .await
        .expect("create site");
        let rendered_dir = TempDir::new().expect("create rendered dir");
        let site_templates_root = crate::resolve_site_templates_root();
        let upload_root = crate::resolve_upload_root();
        let files_written = render_site(
            &db,
            site.id,
            site_templates_root.as_path(),
            rendered_dir.path(),
            upload_root.as_path(),
        )
        .await
        .expect("render site");
        assert!(files_written > 0);
        assert!(
            SiteEntity::find_by_id(site.id)
                .one(&db)
                .await
                .expect("reload site")
                .is_some()
        );
    }
}
