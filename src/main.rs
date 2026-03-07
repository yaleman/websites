use clap::Parser;

use websites::{
    cli::{Commands, ServeCommands},
    execute, telemetry,
};

#[tokio::main]
async fn main() {
    let cli = websites::cli::Cli::parse();

    if let Err(err) = telemetry::init("websites") {
        eprintln!("failed to initialize telemetry: {}", err);
        std::process::exit(1);
    }
    let command = cli.command.unwrap_or(Commands::Serve {
        command: ServeCommands::Admin {
            listen: "127.0.0.1:9000".to_string(),
        },
    });

    if let Err(error) = execute(command, &cli.database_url, &cli.oidc).await {
        eprintln!("error: {}", error);
        std::process::exit(1);
    }
}
