//! Opening the committed fixture stores without touching them: SQLite is
//! copied to a temp dir, Postgres is loaded into a database of its own (the
//! shared test database already holds other suites' accounts).

use std::path::PathBuf;

use sqlx::{Connection, PgConnection, PgPool};
use uuid::Uuid;
use wamux::storage::sql::{SqlPool, SqlStore};

use crate::common;

/// The account each engine's fixture holds (README of the fixture).
pub const SQLITE_ACCOUNT: Uuid = uuid::uuid!("08d9f022-cba8-4ed7-8baa-3d077b219d8b");
pub const POSTGRES_ACCOUNT: Uuid = uuid::uuid!("f7d0ffd6-1b78-4963-9a49-814141641187");

const POSTGRES_DUMP: &str = include_str!("../fixtures/store-0f40e34/postgres.sql");

fn fixture_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/store-0f40e34")
        .join(name)
}

/// A copy of the SQLite fixture, opened the way the daemon opens a store.
pub async fn sqlite() -> (SqlStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let copy = dir.path().join("wamux.db");
    std::fs::copy(fixture_file("wamux.db"), &copy).expect("copy the sqlite fixture");
    let url = format!("sqlite://{}?mode=rwc", copy.display());
    let store = SqlStore::open_sqlite(&url)
        .await
        .expect("open the 0f40e34 sqlite store");
    (store, dir)
}

/// A copy of the SQLite fixture opened WITH a store key (#165): the plaintext
/// store a 0.1.0 daemon wrote, converted on open.
pub async fn sqlite_converted() -> (SqlStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let copy = dir.path().join("wamux.db");
    std::fs::copy(fixture_file("wamux.db"), &copy).expect("copy the sqlite fixture");
    let url = format!("sqlite://{}?mode=rwc", copy.display());
    // expect: a literal of 64 hex characters.
    let key = wamux::storage::StoreKey::parse_hex(&"ab".repeat(32)).expect("hex");
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key))
        .await
        .expect("convert the 0f40e34 sqlite store");
    (store, dir)
}

/// A copy of the SQLite fixture, opened by the Turso engine (#106): a store
/// sqlx wrote, migrated and read by the other family.
#[cfg(feature = "turso")]
pub async fn turso() -> (wamux::storage::turso::TursoStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let copy = dir.path().join("wamux.db");
    std::fs::copy(fixture_file("wamux.db"), &copy).expect("copy the sqlite fixture");
    let store = wamux::storage::turso::TursoStore::open(&format!("turso://{}", copy.display()))
        .await
        .expect("open the 0f40e34 sqlite store with turso");
    (store, dir)
}

/// A throwaway database holding the Postgres fixture.
pub struct ThrowawayPg {
    name: String,
}

/// The Postgres fixture loaded into a fresh database, then opened.
pub async fn postgres() -> (SqlStore, ThrowawayPg) {
    let name = format!("wamux_upgrade_{}", Uuid::new_v4().simple());
    let admin = PgPool::connect(&common::database_url()).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .expect("create the throwaway database");
    admin.close().await;
    let url = database_url_for(&name);
    load_dump(&url).await;
    let store = SqlStore::open_postgres(&url, 2)
        .await
        .expect("open the 0f40e34 postgres store");
    (store, ThrowawayPg { name })
}

fn database_url_for(name: &str) -> String {
    let base = common::database_url();
    format!("{}/{name}", base.rsplit_once('/').unwrap().0)
}

/// On one dedicated connection: the dump empties `search_path` for its
/// session, which a pooled connection would carry into later queries.
async fn load_dump(url: &str) {
    let mut conn = PgConnection::connect(url).await.unwrap();
    sqlx::raw_sql(POSTGRES_DUMP)
        .execute(&mut conn)
        .await
        .expect("load the 0f40e34 postgres dump");
    conn.close().await.unwrap();
}

/// Close the store's pool, then drop its database. The `wamux_upgrade_` prefix
/// is the one the leftovers check already knows to look for.
pub async fn drop_postgres(store: SqlStore, db: ThrowawayPg) {
    if let SqlPool::Pg(pool) = store.pool() {
        pool.close().await;
    }
    let admin = PgPool::connect(&common::database_url()).await.unwrap();
    sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", db.name))
        .execute(&admin)
        .await
        .expect("drop the throwaway database");
}
