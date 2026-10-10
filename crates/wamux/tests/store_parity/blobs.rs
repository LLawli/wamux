//! Byte parity of every stored blob: the same writes through both engines must
//! leave the same bytes in every BLOB/BYTEA column (and the same JSON text in
//! `device_registry`). The device blob is `both_engines_persist_byte_identical_device_blobs`
//! in `storage_backend.rs`; this covers every other table.
//!
//! A drift here fails nothing else: each engine keeps reading back what it
//! wrote. What breaks is the claim that a store can move between engines.
//! `both_engines_` (Postgres against SQLite) run only in the Postgres pass;
//! `turso_and_sqlite_` (#106) need no server. Together they cover all three.

use std::collections::HashMap;
use std::sync::Arc;

use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::traits::{
    AppStateSyncKey, Backend, DeviceInfo, DeviceListRecord, MsgSecretEntry, TcTokenEntry,
};

use crate::common;
use crate::harness::{self, Harness, TwoAccounts};

/// One engine, one account (`a` of the pair), ready for identical writes.
struct Side {
    harness: Harness,
    accounts: TwoAccounts,
}

impl Side {
    async fn new(harness: Harness, tag: &str) -> Self {
        let accounts = harness.two_accounts(tag).await;
        Side { harness, accounts }
    }

    fn backend(&self) -> &Arc<dyn Backend> {
        &self.accounts.ba
    }

    async fn bytes(&self, table: &str, column: &str) -> Vec<Vec<u8>> {
        let device_id = self.accounts.a.device_id;
        self.harness
            .raw
            .column_bytes(table, column, device_id)
            .await
    }

    async fn text(&self, table: &str, column: &str) -> Vec<String> {
        let device_id = self.accounts.a.device_id;
        self.harness.raw.column_text(table, column, device_id).await
    }

    async fn drop(self) {
        self.harness.drop_accounts(self.accounts).await;
    }
}

async fn both_sides(tag: &str) -> (Side, Side) {
    let pg = Side::new(harness::postgres().await, tag).await;
    let lite = Side::new(harness::sqlite().await, tag).await;
    (pg, lite)
}

/// SQLite and Turso, each in a file of its own: needs no Postgres (#106).
#[cfg(feature = "turso")]
async fn sqlite_and_turso(tag: &str) -> (Side, Side) {
    let lite = Side::new(harness::sqlite().await, tag).await;
    let turso = Side::new(harness::turso().await, tag).await;
    (lite, turso)
}

/// Compare one BLOB column across engines; an empty column proves nothing.
async fn assert_same_bytes(one: &Side, other: &Side, table: &str, column: &str) {
    let (left, right) = (
        one.bytes(table, column).await,
        other.bytes(table, column).await,
    );
    assert!(
        !left.is_empty(),
        "{table}.{column} must have rows to compare"
    );
    assert_eq!(left, right, "{table}.{column} differs between engines");
}

async fn write_signal_state(b: &dyn Backend) {
    b.put_identity("alice@s.whatsapp.net", [0xa1; 32])
        .await
        .unwrap();
    b.put_session("alice@s.whatsapp.net", &[0x00, 0xff, 0x10, 0x80])
        .await
        .unwrap();
    b.store_prekey(7, &[0x07, 0x00, 0xfe], true).await.unwrap();
    b.store_signed_prekey(3, &[0x53, 0x00, 0x01]).await.unwrap();
    b.put_sender_key("120363000000000001@g.us::alice", &[0x5e, 0x00, 0xee])
        .await
        .unwrap();
}

/// Same signal writes on two engines, same bytes in every column.
async fn signal_blobs_match(left: Side, right: Side) {
    if common::encrypted_tests() {
        // Sealed blobs differ by nonce; `store_encryption` covers what is stored.
        left.drop().await;
        right.drop().await;
        return;
    }
    for side in [&left, &right] {
        write_signal_state(&**side.backend()).await;
    }
    for (table, column) in [
        ("identities", "key"),
        ("sessions", "record"),
        ("prekeys", "key"),
        ("signed_prekeys", "record"),
        ("sender_keys", "record"),
    ] {
        assert_same_bytes(&left, &right, table, column).await;
    }
    left.drop().await;
    right.drop().await;
}

#[tokio::test]
async fn both_engines_persist_byte_identical_signal_blobs() {
    let (pg, lite) = both_sides("blobs-signal").await;
    signal_blobs_match(pg, lite).await;
}

/// With `both_engines_` (Postgres against SQLite), closes the three engines.
#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_and_sqlite_persist_byte_identical_signal_blobs() {
    let (lite, turso) = sqlite_and_turso("blobs-signal").await;
    signal_blobs_match(lite, turso).await;
}

/// A multi-entry `index_value_map` on purpose: the protobuf blob is only
/// deterministic because the codec encodes maps in key order (#31).
fn hash_state() -> HashState {
    let index_value_map: HashMap<String, Vec<u8>> = (0u8..5)
        .map(|i| (format!("index-{i}"), vec![i; 8]))
        .collect();
    HashState {
        version: 42,
        hash: [0x5a; 128],
        index_value_map,
        ..HashState::default()
    }
}

async fn write_app_sync_state(b: &dyn Backend) {
    let key = AppStateSyncKey {
        key_data: vec![0xd0; 32],
        fingerprint: vec![0x01, 0x02, 0x03],
        timestamp: 1_749_400_000,
    };
    b.set_sync_key(&[0x00, 0x01], key).await.unwrap();
    b.set_version("regular_low", hash_state()).await.unwrap();
    let macs = [AppStateMutationMAC {
        index_mac: vec![0x1d; 32],
        value_mac: vec![0x7a; 32],
    }];
    b.put_mutation_macs("regular_low", 42, &macs).await.unwrap();
}

/// Same app_sync writes on two engines, same bytes in every column.
async fn app_sync_blobs_match(left: Side, right: Side) {
    if common::encrypted_tests() {
        // Sealed blobs differ by nonce; `store_encryption` covers what is stored.
        left.drop().await;
        right.drop().await;
        return;
    }
    for side in [&left, &right] {
        write_app_sync_state(&**side.backend()).await;
    }
    for (table, column) in [
        ("app_state_keys", "key_id"),
        ("app_state_keys", "key_data"),
        ("app_state_versions", "state_data"),
        ("app_state_mutation_macs", "index_mac"),
        ("app_state_mutation_macs", "value_mac"),
    ] {
        assert_same_bytes(&left, &right, table, column).await;
    }
    left.drop().await;
    right.drop().await;
}

#[tokio::test]
async fn both_engines_persist_byte_identical_app_sync_blobs() {
    let (pg, lite) = both_sides("blobs-app-sync").await;
    app_sync_blobs_match(pg, lite).await;
}

/// With `both_engines_` (Postgres against SQLite), closes the three engines.
#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_and_sqlite_persist_byte_identical_app_sync_blobs() {
    let (lite, turso) = sqlite_and_turso("blobs-app-sync").await;
    app_sync_blobs_match(lite, turso).await;
}

async fn write_protocol_state(b: &dyn Backend) {
    b.save_base_key("alice.0", "M1", &[0xba, 0x00, 0x5e])
        .await
        .unwrap();
    let token = TcTokenEntry {
        token: vec![0x7c, 0x00, 0xff],
        token_timestamp: 1_749_400_000,
        sender_timestamp: Some(1_749_400_100),
    };
    b.put_tc_token("100000000000001@lid", &token).await.unwrap();
    b.store_sent_message("5511900000001@s.whatsapp.net", "M1", &[0x0a, 0x00, 0x01])
        .await
        .unwrap();
    let secret = MsgSecretEntry {
        chat: Arc::from("5511900000001@s.whatsapp.net"),
        sender: Arc::from("5511900000000@s.whatsapp.net"),
        msg_id: Arc::from("M1"),
        secret: [0x5c; 32],
        expires_at: 0,
        message_ts: 1_749_400_000,
    };
    b.put_msg_secrets(vec![secret]).await.unwrap();
    let devices = [
        DeviceInfo::new(0, None),
        DeviceInfo::new(4, Some(0)).with_hosting(true),
    ];
    b.update_device_list(DeviceListRecord {
        user: "5511900000001".into(),
        devices: devices.into(),
        timestamp: 1_749_400_000,
        phash: Some("2:abc".into()),
        raw_id: Some(5),
    })
    .await
    .unwrap();
}

/// Same protocol writes on two engines, same bytes in every column.
async fn protocol_blobs_match(left: Side, right: Side) {
    if common::encrypted_tests() {
        // Sealed blobs differ by nonce; `store_encryption` covers what is stored.
        left.drop().await;
        right.drop().await;
        return;
    }
    for side in [&left, &right] {
        write_protocol_state(&**side.backend()).await;
    }
    for (table, column) in [
        ("base_keys", "base_key"),
        ("tc_tokens", "token"),
        ("sent_messages", "payload"),
        ("msg_secrets", "secret"),
    ] {
        assert_same_bytes(&left, &right, table, column).await;
    }
    let left_json = left.text("device_registry", "devices_json").await;
    assert_eq!(left_json.len(), 1, "one registry row");
    assert_eq!(
        left_json,
        right.text("device_registry", "devices_json").await,
        "device_registry.devices_json differs between engines"
    );
    left.drop().await;
    right.drop().await;
}

#[tokio::test]
async fn both_engines_persist_byte_identical_protocol_blobs() {
    let (pg, lite) = both_sides("blobs-protocol").await;
    protocol_blobs_match(pg, lite).await;
}

/// With `both_engines_` (Postgres against SQLite), closes the three engines.
#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_and_sqlite_persist_byte_identical_protocol_blobs() {
    let (lite, turso) = sqlite_and_turso("blobs-protocol").await;
    protocol_blobs_match(lite, turso).await;
}
