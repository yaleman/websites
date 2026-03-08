use std::sync::Arc;

use clap::Parser;
use sqlx::types::time::OffsetDateTime;
use tower_sessions::{Session, cookie::time::Duration};
use tower_sessions_sqlx_store::SqliteStore;
use websites::{constants::SESSION_USER_SUB, db::db_start};

#[derive(Parser)]
#[command(
    name = "session_seed",
    about = "Seed a session for admin UI tests, session is only 5 minutes long."
)]
struct Args {
    #[arg(long = "database-url")]
    database_url: String,
    #[arg(long = "user-sub", default_value = "test-user")]
    user_sub: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let db = db_start(&args.database_url)
        .await
        .expect("failed to start db");

    let pool = db.get_sqlite_connection_pool();

    let store = SqliteStore::new(pool.clone());
    store
        .migrate()
        .await
        .expect("failed to migrate session store");

    let store = Arc::new(store);
    let session = Session::new(
        None,
        store,
        Some(tower_sessions::Expiry::AtDateTime(
            OffsetDateTime::now_utc().saturating_add(Duration::minutes(5)),
        )),
    );
    session
        .insert(SESSION_USER_SUB, args.user_sub)
        .await
        .expect("failed to insert user_sub");
    session.save().await.expect("failed to save session");

    let session_id = session.id().expect("missing session id");
    println!("{session_id}");
}
