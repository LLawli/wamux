//! Batch writes and `commit_patch` are all-or-nothing (#104). A trigger
//! rejects one magic key; it sits in the MIDDLE of each batch, so a per-row
//! loop would leave the rows before it behind. Triggers change the schema, so
//! Postgres gets a throwaway database (the `wamux_upgrade_` prefix the
//! leftovers check knows) and SQLite a file of its own.

use std::sync::Arc;

use bytes::Bytes;
use sqlx::PgPool;
use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::traits::{Backend, DeviceInfo, DeviceListRecord, LidPnMappingEntry};
use wamux::storage::StorageEngine;
use wamux::storage::sql::{SqlPool, SqlStore};

use crate::common;

const IN: &str = "reject-in.0";
const DEL: &str = "reject-del.0";
const PREKEY_IN: u32 = 4242;
const PREKEY_DEL: u32 = 4343;
const MAC_IN: [u8; 32] = [0xFA; 32];
const LOW: &str = "regular_low";

/// Address-keyed tables: reject inserting `IN` and deleting `DEL`.
const ADDRESS_TABLES: [&str; 3] = ["identities", "sessions", "sender_keys"];

fn postgres_triggers() -> String {
    let mut sql = String::from(
        "CREATE FUNCTION wamux_test_reject() RETURNS trigger LANGUAGE plpgsql
         AS $$ BEGIN RAISE EXCEPTION 'rejected by test'; END $$;\n",
    );
    let mut add = |name: &str, table: &str, event: &str, when: &str| {
        sql.push_str(&format!(
            "CREATE TRIGGER {name} BEFORE {event} ON {table} FOR EACH ROW WHEN ({when})
             EXECUTE FUNCTION wamux_test_reject();\n"
        ));
    };
    for table in ADDRESS_TABLES {
        add(
            &format!("{table}_in"),
            table,
            "INSERT",
            &format!("NEW.address = '{IN}'"),
        );
        add(
            &format!("{table}_del"),
            table,
            "DELETE",
            &format!("OLD.address = '{DEL}'"),
        );
    }
    add(
        "prekeys_in",
        "prekeys",
        "INSERT",
        &format!("NEW.id = {PREKEY_IN}"),
    );
    add(
        "prekeys_del",
        "prekeys",
        "DELETE",
        &format!("OLD.id = {PREKEY_DEL}"),
    );
    add(
        "lid_in",
        "lid_pn_mapping",
        "INSERT",
        &format!("NEW.lid = '{IN}'"),
    );
    add(
        "devices_in",
        "device_registry",
        "INSERT",
        &format!("NEW.user_id = '{IN}'"),
    );
    add(
        "macs_in",
        "app_state_mutation_macs",
        "INSERT",
        "NEW.index_mac = '\\xfafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafa'::bytea",
    );
    sql
}

fn sqlite_triggers() -> String {
    let mut sql = String::new();
    let mut add = |name: &str, table: &str, event: &str, when: &str| {
        sql.push_str(&format!(
            "CREATE TRIGGER {name} BEFORE {event} ON {table} WHEN {when}
             BEGIN SELECT RAISE(ABORT, 'rejected by test'); END;\n"
        ));
    };
    for table in ADDRESS_TABLES {
        add(
            &format!("{table}_in"),
            table,
            "INSERT",
            &format!("NEW.address = '{IN}'"),
        );
        add(
            &format!("{table}_del"),
            table,
            "DELETE",
            &format!("OLD.address = '{DEL}'"),
        );
    }
    add(
        "prekeys_in",
        "prekeys",
        "INSERT",
        &format!("NEW.id = {PREKEY_IN}"),
    );
    add(
        "prekeys_del",
        "prekeys",
        "DELETE",
        &format!("OLD.id = {PREKEY_DEL}"),
    );
    add(
        "lid_in",
        "lid_pn_mapping",
        "INSERT",
        &format!("NEW.lid = '{IN}'"),
    );
    add(
        "devices_in",
        "device_registry",
        "INSERT",
        &format!("NEW.user_id = '{IN}'"),
    );
    add(
        "macs_in",
        "app_state_mutation_macs",
        "INSERT",
        "NEW.index_mac = x'fafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafa'",
    );
    sql
}

/// One engine with the triggers installed and one account.
struct Rigged {
    /// The SQL store, kept to close its Postgres pool; `None` on Turso.
    store: Option<SqlStore>,
    backend: Arc<dyn Backend>,
    pg_database: Option<String>,
    _dir: Option<tempfile::TempDir>,
}

async fn rigged_postgres() -> Rigged {
    let name = format!("wamux_upgrade_{}", uuid::Uuid::new_v4().simple());
    let admin = PgPool::connect(&common::database_url()).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let base = common::database_url();
    let url = format!("{}/{name}", base.rsplit_once('/').unwrap().0);
    let store = SqlStore::open_postgres(&url, 2)
        .await
        .expect("open throwaway postgres");
    sqlx::raw_sql(&postgres_triggers())
        .execute(common::pg_pool(&store))
        .await
        .expect("triggers");
    rig(store, Some(name), None).await
}

async fn rigged_sqlite() -> Rigged {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("rigged.db").display()
    );
    let store = SqlStore::open_sqlite(&url).await.expect("open sqlite");
    sqlx::raw_sql(&sqlite_triggers())
        .execute(common::lite_pool(&store))
        .await
        .expect("triggers");
    rig(store, None, Some(dir)).await
}

/// Turso runs the SQLite trigger text as is (#106: RAISE(ABORT) verified on
/// turso 0.8.1 before this test was written).
#[cfg(feature = "turso")]
async fn rigged_turso() -> Rigged {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("turso://{}", dir.path().join("rigged.db").display());
    let store = wamux::storage::turso::TursoStore::open(&url)
        .await
        .expect("open turso");
    store
        .connection()
        .lock()
        .await
        .execute_batch(sqlite_triggers())
        .await
        .expect("triggers");
    let row = store.create_account(Some("atomicity")).await.unwrap();
    Rigged {
        backend: store.device_backend(row.device_id),
        store: None,
        pg_database: None,
        _dir: Some(dir),
    }
}

async fn rig(
    store: SqlStore,
    pg_database: Option<String>,
    dir: Option<tempfile::TempDir>,
) -> Rigged {
    let row = store.create_account(Some("atomicity")).await.unwrap();
    let backend = store.device_backend(row.device_id);
    Rigged {
        store: Some(store),
        backend,
        pg_database,
        _dir: dir,
    }
}

async fn unrig(rigged: Rigged) {
    if let Some(SqlPool::Pg(pool)) = rigged.store.as_ref().map(SqlStore::pool) {
        pool.close().await;
    }
    let Some(name) = rigged.pg_database else {
        return;
    };
    let admin = PgPool::connect(&common::database_url()).await.unwrap();
    sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

fn rejected<T: std::fmt::Debug>(result: wacore::store::error::Result<T>, what: &str) {
    assert!(
        result.is_err(),
        "{what}: the trigger must reject the batch, got {result:?}"
    );
}

fn arc(s: &str) -> Arc<str> {
    Arc::from(s)
}

async fn signal_batches_are_all_or_nothing(b: &dyn Backend) {
    let key = [1u8; 32];
    rejected(
        b.put_identities_batch(&[(arc("n1.0"), key), (arc(IN), key), (arc("n2.0"), key)])
            .await,
        "put_identities_batch",
    );
    assert_eq!(
        b.load_identity("n1.0").await.unwrap(),
        None,
        "put_identities_batch left a row"
    );

    for address in ["d1.0", DEL, "d2.0"] {
        b.put_identity(address, key).await.unwrap();
        b.put_session(address, b"s").await.unwrap();
        b.put_sender_key(address, b"k").await.unwrap();
    }
    rejected(
        b.delete_identities_batch(&[arc("d1.0"), arc(DEL), arc("d2.0")])
            .await,
        "delete_identities_batch",
    );
    assert!(
        b.load_identity("d1.0").await.unwrap().is_some(),
        "delete_identities_batch removed a row"
    );

    let s = Bytes::from_static(b"s");
    rejected(
        b.put_sessions_batch(&[
            (arc("n1.0"), s.clone()),
            (arc(IN), s.clone()),
            (arc("n2.0"), s.clone()),
        ])
        .await,
        "put_sessions_batch",
    );
    assert!(
        b.get_session("n1.0").await.unwrap().is_none(),
        "put_sessions_batch left a row"
    );
    rejected(
        b.delete_sessions_batch(&[arc("d1.0"), arc(DEL), arc("d2.0")])
            .await,
        "delete_sessions_batch",
    );
    assert!(
        b.get_session("d1.0").await.unwrap().is_some(),
        "delete_sessions_batch removed a row"
    );

    rejected(
        b.put_sender_keys_batch(&[
            (arc("n1.0"), s.clone()),
            (arc(IN), s.clone()),
            (arc("n2.0"), s),
        ])
        .await,
        "put_sender_keys_batch",
    );
    assert!(
        b.get_sender_key("n1.0").await.unwrap().is_none(),
        "put_sender_keys_batch left a row"
    );
    rejected(
        b.delete_sender_keys_batch(&[arc("d1.0"), arc(DEL), arc("d2.0")])
            .await,
        "delete_sender_keys_batch",
    );
    assert!(
        b.get_sender_key("d1.0").await.unwrap().is_some(),
        "delete_sender_keys_batch removed a row"
    );
}

async fn prekey_batches_are_all_or_nothing(b: &dyn Backend) {
    let r = Bytes::from_static(b"p");
    rejected(
        b.store_prekeys_batch(
            &[(5001, r.clone()), (PREKEY_IN, r.clone()), (5002, r)],
            false,
        )
        .await,
        "store_prekeys_batch",
    );
    assert!(
        b.load_prekey(5001).await.unwrap().is_none(),
        "store_prekeys_batch left a row"
    );
    for id in [6001, PREKEY_DEL, 6002] {
        b.store_prekey(id, b"p", false).await.unwrap();
    }
    rejected(
        b.remove_prekeys_batch(&[6001, PREKEY_DEL, 6002]).await,
        "remove_prekeys_batch",
    );
    assert!(
        b.load_prekey(6001).await.unwrap().is_some(),
        "remove_prekeys_batch removed a row"
    );
}

fn lid(lid: &str) -> LidPnMappingEntry {
    LidPnMappingEntry {
        lid: lid.to_string(),
        phone_number: format!("pn-{lid}"),
        created_at: 1,
        updated_at: 1,
        learning_source: "usync".to_string(),
    }
}

fn devices(user: &str) -> DeviceListRecord {
    DeviceListRecord {
        user: user.into(),
        devices: vec![DeviceInfo::new(0, None)].into(),
        timestamp: 1,
        phash: None,
        raw_id: None,
    }
}

async fn protocol_batches_are_all_or_nothing(b: &dyn Backend) {
    rejected(
        b.put_lid_mappings(&[lid("n1"), lid(IN), lid("n2")]).await,
        "put_lid_mappings",
    );
    assert!(
        b.get_lid_mapping("n1").await.unwrap().is_none(),
        "put_lid_mappings left a row"
    );
    rejected(
        b.update_device_lists(vec![devices("u1"), devices(IN), devices("u2")])
            .await,
        "update_device_lists",
    );
    assert!(
        b.get_devices("u1").await.unwrap().is_none(),
        "update_device_lists left a row"
    );
}

async fn batch_writes_are_all_or_nothing(rigged: Rigged) {
    signal_batches_are_all_or_nothing(&*rigged.backend).await;
    prekey_batches_are_all_or_nothing(&*rigged.backend).await;
    protocol_batches_are_all_or_nothing(&*rigged.backend).await;
    unrig(rigged).await;
}

#[tokio::test]
async fn postgres_batch_writes_are_all_or_nothing() {
    batch_writes_are_all_or_nothing(rigged_postgres().await).await;
}

#[tokio::test]
async fn sqlite_batch_writes_are_all_or_nothing() {
    batch_writes_are_all_or_nothing(rigged_sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_batch_writes_are_all_or_nothing() {
    batch_writes_are_all_or_nothing(rigged_turso().await).await;
}

fn mac(index: [u8; 32], value: u8) -> AppStateMutationMAC {
    AppStateMutationMAC {
        index_mac: index.to_vec(),
        value_mac: vec![value; 32],
    }
}

/// Version 2 would remove MAC A and add B plus the rejected one. Nothing of it
/// may land: the version stays 1 and A stays, or the next patch's ltHash is
/// computed over a state no server ever had.
async fn commit_patch_is_atomic(rigged: Rigged) {
    let b = &*rigged.backend;
    let state = |version: u64| HashState {
        version,
        ..HashState::default()
    };
    b.commit_patch(LOW, state(1), &[], &[mac([0xA1; 32], 1)])
        .await
        .unwrap();
    let result = b
        .commit_patch(
            LOW,
            state(2),
            &[vec![0xA1; 32]],
            &[mac([0xB2; 32], 2), mac(MAC_IN, 3)],
        )
        .await;
    rejected(result, "commit_patch");
    assert_eq!(
        b.get_version(LOW).await.unwrap().map(|s| s.version),
        Some(1),
        "version moved"
    );
    assert!(
        b.get_mutation_mac(LOW, &[0xA1; 32])
            .await
            .unwrap()
            .is_some(),
        "removed MAC is gone"
    );
    assert!(
        b.get_mutation_mac(LOW, &[0xB2; 32])
            .await
            .unwrap()
            .is_none(),
        "added MAC landed"
    );
    unrig(rigged).await;
}

#[tokio::test]
async fn postgres_commit_patch_is_atomic() {
    commit_patch_is_atomic(rigged_postgres().await).await;
}

#[tokio::test]
async fn sqlite_commit_patch_is_atomic() {
    commit_patch_is_atomic(rigged_sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_commit_patch_is_atomic() {
    commit_patch_is_atomic(rigged_turso().await).await;
}
