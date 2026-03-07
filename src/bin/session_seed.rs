use std::sync::Arc;

use clap::Parser;
use sqlx::sqlite::SqlitePool;
use tower_sessions::Session;
use tower_sessions_sqlx_store::SqliteStore;

#[derive(Parser)]
#[command(name = "session_seed", about = "Seed a session for admin UI tests.")]
struct Args {
    #[arg(long = "database-url")]
    database_url: String,
    #[arg(long = "user-sub", default_value = "test-user")]
    user_sub: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let pool = SqlitePool::connect(&args.database_url)
        .await
        .expect("failed to connect sqlite");
    let store = SqliteStore::new(pool);
    store
        .migrate()
        .await
        .expect("failed to migrate session store");

    let store = Arc::new(store);
    let session = Session::new(None, store, None);
    session
        .insert("user_sub", args.user_sub)
        .await
        .expect("failed to insert user_sub");
    session.save().await.expect("failed to save session");

    let session_id = session.id().expect("missing session id");
    println!("{session_id}");
}
