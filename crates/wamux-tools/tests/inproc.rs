//! #64: the in-process bootstrap shared by pair_cli, pair_backfill and
//! stress_live. Postgres-backed, like those binaries.

use std::collections::HashMap;

use wamux_tools::inproc::{
    DEFAULT_DATABASE_URL, database_url_from, open_registry, resolve_or_create,
};

fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string())
}

#[test]
fn database_url_defaults_to_the_dev_database() {
    let empty: HashMap<String, String> = HashMap::new();
    let lookup = |v: &str| empty.get(v).cloned();
    assert_eq!(database_url_from(&lookup), DEFAULT_DATABASE_URL);
    assert_eq!(
        DEFAULT_DATABASE_URL,
        "postgres://wamux:wamux@localhost:5433/wamux"
    );
}

#[test]
fn database_url_reads_the_override() {
    let lookup = |v: &str| (v == "DATABASE_URL").then(|| "postgres://x@h/db".to_string());
    assert_eq!(database_url_from(&lookup), "postgres://x@h/db");
}

#[tokio::test]
async fn resolve_or_create_creates_once_then_reuses() {
    let registry = open_registry(&database_url()).await.unwrap();
    let name = format!("tools-64-inproc-{}", uuid::Uuid::new_v4());

    let (first, created) = resolve_or_create(&registry, &name).await.unwrap();
    assert!(created, "the account did not exist yet");
    let (again, created_again) = resolve_or_create(&registry, &name).await.unwrap();
    assert!(!created_again);
    assert_eq!(first.uuid, again.uuid);

    // A second registry over the same store sees the persisted account.
    let fresh = open_registry(&database_url()).await.unwrap();
    let (loaded, created_fresh) = resolve_or_create(&fresh, &name).await.unwrap();
    assert!(!created_fresh, "open_registry must load persisted accounts");
    assert_eq!(loaded.uuid, first.uuid);

    registry.delete(&first).await.unwrap();
}
