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
    unsafe {
        std::env::set_var("WEBSITES_UPLOAD_ROOT", &upload_root);
    }

    if let Err(err) = telemetry::init("websites") {
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
        &cli.rendered_dir,
        &cli.oidc,
    )
    .await
    {
        eprintln!("error: {}", error);
        std::process::exit(1);
    }
}
