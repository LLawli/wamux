//! The engine-neutral bodies: every getter against what `0f40e34` wrote, then
//! new writes, a new account and a cascade on top of those rows.

use async_trait::async_trait;
use uuid::Uuid;
use wacore::store::traits::Backend;
use wamux::storage::StorageEngine;
use wamux::storage::blob_codec::{encode_device, encode_hash_state};
use wamux::storage::sql::{SqlPool, SqlStore};

use crate::values::*;

/// The fixture's account on both engines: the first one each store created.
const FIXTURE_DEVICE_ID: i32 = 1;

const DEVICE_PB: &[u8] = include_bytes!("../fixtures/store-0f40e34/device.pb");

/// An engine plus the one read the trait has no getter for. Implemented for
/// both families (#106), so every body here runs on each engine unchanged.
#[async_trait]
pub trait FixtureStore: StorageEngine {
    /// `prekeys.uploaded` of the fixture account's prekey `id`.
    async fn prekey_uploaded(&self, id: u32) -> bool;
}

#[async_trait]
impl FixtureStore for SqlStore {
    async fn prekey_uploaded(&self, id: u32) -> bool {
        prekey_uploaded(self, id).await
    }
}

/// Turso binds `$N` by appearance (#106), so the raw read writes `?N`.
#[cfg(feature = "turso")]
#[async_trait]
impl FixtureStore for wamux::storage::turso::TursoStore {
    async fn prekey_uploaded(&self, id: u32) -> bool {
        let sql = "SELECT uploaded FROM prekeys WHERE device_id = ?1 AND id = ?2";
        let params = vec![
            turso::Value::Integer(FIXTURE_DEVICE_ID.into()),
            turso::Value::Integer(id.into()),
        ];
        match crate::common::turso_rows(self, sql, params)
            .await
            .as_slice()
        {
            [row] => row[0] == turso::Value::Integer(1),
            other => panic!("prekey {id}: expected one row, got {other:?}"),
        }
    }
}

pub async fn reads_back(store: &dyn FixtureStore, account: Uuid) {
    check_account_row(store, account).await;
    let b = store.device_backend(FIXTURE_DEVICE_ID);
    check_device(&*b).await;
    check_signal(&*b).await;
    check_prekeys(store, &*b).await;
    check_app_sync(&*b).await;
    check_sender_keys_and_mappings(&*b).await;
    check_device_list_and_tokens(&*b).await;
    check_sent_and_secrets(&*b).await;
}

async fn check_account_row(store: &dyn FixtureStore, account: Uuid) {
    let rows = store.list_accounts().await.expect("list accounts");
    assert_eq!(rows.len(), 1, "the fixture holds one account");
    let row = &rows[0];
    assert_eq!(row.uuid, account);
    assert_eq!(row.external_ref.as_deref(), Some(ACCOUNT_REF));
    assert_eq!(row.device_id, FIXTURE_DEVICE_ID);
    assert_eq!(row.push_name, None);
    assert!(
        row.created_at > 1_759_000_000,
        "created_at {}",
        row.created_at
    );
}

/// Byte-equal to the `Device` that was saved, so every persisted field
/// (key material included) came back.
async fn check_device(b: &dyn Backend) {
    assert!(b.exists().await.unwrap());
    let device = b.load().await.unwrap().expect("the device row loads");
    assert_eq!(encode_device(&device), DEVICE_PB);
}

async fn check_signal(b: &dyn Backend) {
    assert_eq!(b.load_identity(ADDRESS).await.unwrap(), Some(IDENTITY));
    let session = b.get_session(ADDRESS).await.unwrap().expect("session");
    assert_eq!(&session[..], SESSION);
    let (id, record) = SIGNED_PREKEY;
    assert_eq!(
        b.load_signed_prekey(id).await.unwrap().as_deref(),
        Some(record)
    );
    assert_eq!(
        b.load_all_signed_prekeys().await.unwrap(),
        vec![(id, record.to_vec())]
    );
    let sender_key = b.get_sender_key(SENDER_KEY_ADDRESS).await.unwrap();
    assert_eq!(sender_key.as_deref(), Some(SENDER_KEY));
}

async fn check_prekeys(store: &dyn FixtureStore, b: &dyn Backend) {
    for (id, record, uploaded) in PREKEYS {
        let loaded = b.load_prekey(id).await.unwrap().expect("prekey");
        assert_eq!(&loaded[..], record, "prekey {id}");
        assert_eq!(store.prekey_uploaded(id).await, uploaded, "prekey {id}");
    }
    assert_eq!(b.get_max_prekey_id().await.unwrap(), 8);
}

/// `uploaded` has no getter in the trait; read the column. sqlx-sqlite binds
/// `$N` by number, so one statement serves both drivers.
async fn prekey_uploaded(store: &SqlStore, id: u32) -> bool {
    let sql = "SELECT uploaded FROM prekeys WHERE device_id = $1 AND id = $2";
    let row = match store.pool() {
        SqlPool::Pg(pool) => {
            sqlx::query_scalar(sql)
                .bind(FIXTURE_DEVICE_ID)
                .bind(id as i32)
                .fetch_one(pool)
                .await
        }
        SqlPool::Sqlite(pool) => {
            sqlx::query_scalar(sql)
                .bind(FIXTURE_DEVICE_ID)
                .bind(id as i32)
                .fetch_one(pool)
                .await
        }
    };
    row.expect("read prekeys.uploaded")
}

async fn check_app_sync(b: &dyn Backend) {
    let key = b
        .get_sync_key(SYNC_KEY_ID)
        .await
        .unwrap()
        .expect("sync key");
    let want = sync_key();
    assert_eq!(key.key_data, want.key_data);
    assert_eq!(key.fingerprint, want.fingerprint);
    assert_eq!(key.timestamp, want.timestamp);
    let state = b.get_version(COLLECTION).await.unwrap().expect("version");
    assert_eq!(encode_hash_state(&state), encode_hash_state(&hash_state()));
    let mac = mutation_mac();
    let value = b
        .get_mutation_mac(COLLECTION, &mac.index_mac)
        .await
        .unwrap();
    assert_eq!(value, Some(mac.value_mac));
    let latest = b.get_latest_sync_key_id().await.unwrap();
    assert_eq!(latest.as_deref(), Some(SYNC_KEY_ID));
}

async fn check_sender_keys_and_mappings(b: &dyn Backend) {
    let mut devices = b.get_sender_key_devices(GROUP).await.unwrap();
    devices.sort();
    let want: Vec<(String, bool)> = SENDER_KEY_DEVICES
        .iter()
        .map(|(jid, has)| (jid.to_string(), *has))
        .collect();
    assert_eq!(devices, want);
    let want = lid_mapping();
    let by_lid = b.get_lid_mapping(&want.lid).await.unwrap().expect("by lid");
    let by_pn = b.get_pn_mapping(&want.phone_number).await.unwrap();
    let all = b.get_all_lid_mappings().await.unwrap();
    assert_eq!(all.len(), 1);
    for got in [by_lid, by_pn.expect("by pn"), all[0].clone()] {
        assert_eq!(got.lid, want.lid);
        assert_eq!(got.phone_number, want.phone_number);
        assert_eq!(got.created_at, want.created_at);
        assert_eq!(got.updated_at, want.updated_at);
        assert_eq!(got.learning_source, want.learning_source);
    }
}

async fn check_device_list_and_tokens(b: &dyn Backend) {
    let (address, message_id, base_key) = BASE_KEY;
    assert!(
        b.has_same_base_key(address, message_id, base_key)
            .await
            .unwrap()
    );
    let want = device_list();
    let got = b
        .get_devices(DEVICE_LIST_USER)
        .await
        .unwrap()
        .expect("devices");
    assert_eq!(&*got.user, &*want.user);
    assert_eq!(&*got.devices, &*want.devices);
    assert_eq!(got.timestamp, want.timestamp);
    assert_eq!(got.phash, want.phash);
    assert_eq!(got.raw_id, want.raw_id);
    let want = tc_token();
    let got = b
        .get_tc_token(TC_TOKEN_JID)
        .await
        .unwrap()
        .expect("tc token");
    assert_eq!(got.token, want.token);
    assert_eq!(got.token_timestamp, want.token_timestamp);
    assert_eq!(got.sender_timestamp, want.sender_timestamp);
    assert_eq!(b.get_all_tc_token_jids().await.unwrap(), vec![TC_TOKEN_JID]);
}

async fn check_sent_and_secrets(b: &dyn Backend) {
    let sent = b.get_sent_message(SENT_CHAT, SENT_ID).await.unwrap();
    assert_eq!(sent.as_deref(), Some(SENT_PAYLOAD));
    let want = msg_secret();
    let got = b
        .get_msg_secret_with_ts(&want.chat, &want.sender, &want.msg_id)
        .await
        .unwrap();
    assert_eq!(got, Some((want.secret.to_vec(), want.message_ts)));
}

/// After the old rows: a new write on the old account reads back, a new
/// account gets a `device_id` past the old one (the IDENTITY sequence and
/// SQLite's AUTOINCREMENT survived), and deleting the old account cascades.
pub async fn keeps_working(store: &dyn FixtureStore, account: Uuid) {
    let old = store.device_backend(FIXTURE_DEVICE_ID);
    old.put_session("5511900000001.1", &[0x99, 0x00])
        .await
        .unwrap();
    let session = old.get_session("5511900000001.1").await.unwrap();
    assert_eq!(session.as_deref(), Some(&[0x99, 0x00][..]));

    let fresh = store.create_account(Some("fixture-new")).await.unwrap();
    assert!(
        fresh.device_id > FIXTURE_DEVICE_ID,
        "got {}",
        fresh.device_id
    );
    assert!(store.delete_account(fresh.uuid).await.unwrap());

    assert!(store.delete_account(account).await.unwrap());
    assert!(old.load().await.unwrap().is_none(), "device row cascaded");
    assert!(old.get_session(ADDRESS).await.unwrap().is_none());
    assert!(old.load_identity(ADDRESS).await.unwrap().is_none());
    assert!(store.list_accounts().await.unwrap().is_empty());
}
