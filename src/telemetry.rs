use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use anyhow::Context;
use tokio::fs;
use tracing_subscriber::filter::Directive;
use tracing_subscriber::fmt::time::SystemTime;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::{EnvFilter, fmt};

static LOG_GUARD: OnceLock<Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>> =
    OnceLock::new();

pub async fn init(service: &str) -> anyhow::Result<()> {
    init_with_log_path(service, crate::resolve_log_path()).await
}

pub async fn init_with_log_path(service: &str, log_path: std::path::PathBuf) -> anyhow::Result<()> {
    let log_dir = log_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let log_file_name = match log_path.file_name() {
        Some(value) => value.to_string_lossy().into_owned(),
        None => {
            init_writer(service, std::io::stdout)?;
            tracing::warn!(
                log_path = %log_path.display(),
                "failed to initialize file logging; using stdout only"
            );
            return Ok(());
        }
    };

    if let Err(error) = fs::create_dir_all(&log_dir).await {
        init_writer(service, std::io::stdout)?;
        tracing::warn!(
            log_path = %log_path.display(),
            error = %error,
            "failed to initialize file logging; using stdout only"
        );
        return Ok(());
    }
    let file_appender = tracing_appender::rolling::never(&log_dir, log_file_name);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let guard_slot = LOG_GUARD.get_or_init(|| Mutex::new(None));
    *guard_slot
        .lock()
        .map_err(|_| anyhow::anyhow!("failed to lock telemetry log guard"))? = Some(guard);

    init_writer(service, std::io::stdout.and(file_writer))
}

pub fn init_writer<T: for<'writer> fmt::MakeWriter<'writer> + 'static + Send + Sync>(
    service: &str,
    writer: T,
) -> anyhow::Result<()> {
    let default_directive = format!("info,{service}=info");
    let mut env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_directive));

    env_filter = env_filter.add_directive(
        Directive::from_str("sqlx=warn").with_context(|| "failed to parse directive for sqlx")?,
    );

    fmt()
        .with_env_filter(env_filter)
        .with_timer(SystemTime)
        .with_ansi(false)
        .with_writer(writer)
        .with_target(true)
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing subscriber: {error}"))?;

    Ok(())
}

pub fn test() {
    let env_filter = EnvFilter::new("debug,sqlx::query=warn");

    let _ = fmt()
        .with_env_filter(env_filter)
        .with_timer(SystemTime)
        .with_writer(std::io::stderr)
        .with_target(true)
        .try_init();
}
