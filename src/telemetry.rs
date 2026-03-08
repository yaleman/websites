use std::str::FromStr;

use anyhow::Context;
use tracing_subscriber::filter::Directive;
use tracing_subscriber::fmt::time::SystemTime;
use tracing_subscriber::{EnvFilter, fmt};

pub fn init(service: &str) -> anyhow::Result<()> {
    init_writer(service, std::io::stdout)
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
