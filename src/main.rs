#![forbid(unsafe_code)]
#![deny(warnings)]
#![deny(deprecated)]
#![recursion_limit = "512"]
#![warn(unused_extern_crates)]
// Enable some groups of clippy lints.
#![deny(clippy::suspicious)]
#![deny(clippy::perf)]
// Specific lints to enforce.
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::await_holding_lock)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::trivially_copy_pass_by_ref)]
#![deny(clippy::disallowed_types)]
#![deny(clippy::manual_let_else)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::unreachable)]

use clap::Parser;

use websites::{
    cli::execute,
    cli::{Commands, ServeCommands},
    resolve_upload_root, telemetry,
};

#[tokio::main]
async fn main() {
    let cli = websites::cli::Cli::parse();
    let upload_root = cli.upload_dir.clone().unwrap_or_else(resolve_upload_root);

    if let Err(err) = telemetry::init_with_log_path("websites", cli.log_path.clone()).await {
        eprintln!("failed to initialize telemetry: {}", err);
        std::process::exit(1);
    }
    let command = cli.command.unwrap_or_else(|| {
        let listen = std::env::var("WEBSITES_LISTEN_ADDR").unwrap_or("127.0.0.1:9000".to_string());
        Commands::Serve {
            command: ServeCommands::Admin { listen },
        }
    });

    if let Err(error) = execute(
        command,
        &cli.db_path,
        &cli.site_templates_dir,
        &upload_root,
        &cli.rendered_dir,
        &cli.log_path,
        &cli.oidc,
    )
    .await
    {
        eprintln!("error: {}", error);
        std::process::exit(1);
    }
}
