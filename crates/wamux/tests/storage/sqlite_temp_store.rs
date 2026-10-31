//! #229: the SQLite store works where no temporary directory is usable.
//!
//! The published image is `scratch`: no /tmp, no /var/tmp, `/` as the working
//! directory and a user who cannot write there. SQLite spills a statement
//! sub-journal past 64 KiB to a temporary file, so the first savepoint that
//! touched more (the 812 prekeys marked uploaded right after pairing) failed
//! with SQLITE_IOERR_GETTEMPPATH. The store keeps its temporaries in memory.
//!
//! The second test is the one `scripts/check-sqlite-no-tmp.sh` runs inside a
//! container with no usable temp dir; on a host with a /tmp it passes either
//! way. Its store goes in `WAMUX_TEST_STORE_DIR` when set, not in TMPDIR:
//! SQLite reads TMPDIR too, and the harness must leave it unset.

use bytes::Bytes;
use wamux::storage::StorageEngine;
use wamux::storage::sql::{SqlPool, SqlStore, SqliteConn};

/// `PRAGMA temp_store` answers 2 for MEMORY (0 is the compiled-in default).
const TEMP_STORE_MEMORY: i64 = 2;

/// What whatsapp-rust uploads on pairing (`DEFAULT_WANTED_PRE_KEY_COUNT`).
const PAIRING_PREKEYS: u32 = 812;

/// Big enough that the 812 rows span well over the 64 KiB a sub-journal holds
/// in memory before SQLite spills it to a temporary file.
const PREKEY_RECORD_BYTES: usize = 200;

fn store_dir() -> tempfile::TempDir {
    match std::env::var_os("WAMUX_TEST_STORE_DIR") {
        Some(dir) => tempfile::tempdir_in(dir).expect("a store dir in WAMUX_TEST_STORE_DIR"),
        None => tempfile::tempdir().expect("a store dir"),
    }
}

async fn open_store(dir: &tempfile::TempDir) -> SqlStore {
    let url = format!("sqlite://{}", dir.path().join("wamux.db").display());
    SqlStore::open_sqlite(&url)
        .await
        .expect("open the sqlite store")
}

fn sqlite_conn(store: &SqlStore) -> SqliteConn {
    match store.pool() {
        SqlPool::Sqlite(conn) => conn.clone(),
        SqlPool::Pg(_) => panic!("opened as sqlite, holds a postgres pool"),
    }
}

fn prekeys() -> Vec<(u32, Bytes)> {
    (1..=PAIRING_PREKEYS)
        .map(|id| (id, Bytes::from(vec![(id % 251) as u8; PREKEY_RECORD_BYTES])))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_temp_store_is_memory_on_writer_and_readers() {
    let dir = store_dir();
    let store = open_store(&dir).await;
    let conn = sqlite_conn(&store);

    let on_writer: i64 = {
        let mut writer = conn.lock().await;
        sqlx::query_scalar("PRAGMA temp_store")
            .fetch_one(&mut *writer)
            .await
            .unwrap()
    };
    let on_reader: i64 = sqlx::query_scalar("PRAGMA temp_store")
        .fetch_one(conn.read_pool())
        .await
        .unwrap();

    assert_eq!(
        on_writer, TEMP_STORE_MEMORY,
        "the writer keeps temporaries in memory"
    );
    assert_eq!(
        on_reader, TEMP_STORE_MEMORY,
        "the readers keep temporaries in memory"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_prekey_upload_batch_commits_without_a_temp_dir() {
    let dir = store_dir();
    let store = open_store(&dir).await;
    let account = store.create_account(Some("no-tmp")).await.unwrap();
    let backend = store.device_backend(account.device_id);
    let keys = prekeys();
    let ids: Vec<u32> = keys.iter().map(|(id, _)| *id).collect();

    backend.store_prekeys_batch(&keys, false).await.unwrap();
    backend
        .mark_prekeys_uploaded(&ids)
        .await
        .expect("the pairing batch commits");

    let uploaded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prekeys WHERE device_id = $1 AND uploaded")
            .bind(account.device_id)
            .fetch_one(sqlite_conn(&store).read_pool())
            .await
            .unwrap();
    assert_eq!(
        uploaded,
        i64::from(PAIRING_PREKEYS),
        "every prekey is marked uploaded"
    );
    backend
        .put_session("peer@s.whatsapp.net.0", b"after the batch")
        .await
        .expect("the writer still takes writes after the batch");
}
