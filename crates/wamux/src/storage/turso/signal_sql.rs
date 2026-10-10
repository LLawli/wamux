//! `SignalStore` helpers shared by the one-row methods and the batches (#106,
//! the turso twin of `sql::signal_sql`). Kept out of `signal_store.rs`: the
//! coverage checks read every `async fn` in `*_store.rs` as a trait method.
//!
//! Writes: the one-row statement in a loop inside ONE transaction, so a batch
//! is one lock and one commit, all or nothing; a key repeated in a batch is
//! written in order, so the last one wins. Reads: one query per `READ_CHUNK`
//! values (`batch_chunks`).

use std::sync::Arc;

use bytes::Bytes;
use wacore::store::error::Result;

use super::exec::{binds, with_list};
use super::row_values::{blob, int32, text};
use super::{TursoConn, TursoTx};
use crate::storage::batch_chunks::padded_chunks;
use crate::storage::blob_cipher::BlobCipher;
use crate::storage::statements::signal::{
    DELETE_IDENTITY, DELETE_PREKEY, DELETE_SENDER_KEY, DELETE_SESSION, MARK_PREKEY_UPLOADED,
    PUT_IDENTITY, PUT_SENDER_KEY, PUT_SESSION, SELECT_PREKEYS, SELECT_SESSIONS, STORE_PREKEY,
};

/// One UPDATE per id in one transaction: no array bind here, the SQLite shape.
pub(super) async fn mark_uploaded(conn: &TursoConn, device_id: i32, ids: &[u32]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let tx = TursoTx::begin(conn).await?;
    for id in ids {
        tx.execute(MARK_PREKEY_UPLOADED, binds![*id as i32, device_id])
            .await?;
    }
    tx.commit().await
}

pub(super) async fn put_identities(
    conn: &TursoConn,
    cipher: &BlobCipher,
    device_id: i32,
    identities: &[(Arc<str>, [u8; 32])],
) -> Result<()> {
    if identities.is_empty() {
        return Ok(());
    }
    let tx = TursoTx::begin(conn).await?;
    for (address, key) in identities {
        let sealed = cipher.seal_at(device_id, "identities", "key", address.as_bytes(), key)?;
        let binds = binds![&**address, sealed, device_id];
        tx.execute(PUT_IDENTITY, binds).await?;
    }
    tx.commit().await
}

pub(super) async fn delete_identities(
    conn: &TursoConn,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<()> {
    delete_by_address(conn, DELETE_IDENTITY, device_id, addresses).await
}

pub(super) async fn put_sessions(
    conn: &TursoConn,
    cipher: &BlobCipher,
    device_id: i32,
    sessions: &[(Arc<str>, Bytes)],
) -> Result<()> {
    let target = SealedColumn::new(PUT_SESSION, "sessions", "record");
    put_address_records(conn, cipher, target, device_id, sessions).await
}

pub(super) async fn delete_sessions(
    conn: &TursoConn,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<()> {
    delete_by_address(conn, DELETE_SESSION, device_id, addresses).await
}

pub(super) async fn put_sender_keys(
    conn: &TursoConn,
    cipher: &BlobCipher,
    device_id: i32,
    sender_keys: &[(Arc<str>, Bytes)],
) -> Result<()> {
    let target = SealedColumn::new(PUT_SENDER_KEY, "sender_keys", "record");
    put_address_records(conn, cipher, target, device_id, sender_keys).await
}

pub(super) async fn delete_sender_keys(
    conn: &TursoConn,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<()> {
    delete_by_address(conn, DELETE_SENDER_KEY, device_id, addresses).await
}

pub(super) async fn store_prekeys(
    conn: &TursoConn,
    cipher: &BlobCipher,
    device_id: i32,
    keys: &[(u32, Bytes)],
    uploaded: bool,
) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let tx = TursoTx::begin(conn).await?;
    for (id, record) in keys {
        let sealed = cipher.seal_at(device_id, "prekeys", "key", &id.to_be_bytes(), record)?;
        let binds = binds![*id as i32, sealed, uploaded, device_id];
        tx.execute(STORE_PREKEY, binds).await?;
    }
    tx.commit().await
}

pub(super) async fn remove_prekeys(conn: &TursoConn, device_id: i32, ids: &[u32]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let tx = TursoTx::begin(conn).await?;
    for id in ids {
        tx.execute(DELETE_PREKEY, binds![*id as i32, device_id])
            .await?;
    }
    tx.commit().await
}

/// The statement and the sealed column of an `(address, bytes)` upsert.
#[derive(Clone, Copy)]
struct SealedColumn {
    sql: &'static str,
    table: &'static str,
    column: &'static str,
}

impl SealedColumn {
    fn new(sql: &'static str, table: &'static str, column: &'static str) -> Self {
        Self { sql, table, column }
    }
}

/// Sessions and sender keys share a shape: `(address, bytes)` upserted by address.
async fn put_address_records(
    conn: &TursoConn,
    cipher: &BlobCipher,
    target: SealedColumn,
    device_id: i32,
    records: &[(Arc<str>, Bytes)],
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let tx = TursoTx::begin(conn).await?;
    for (address, record) in records {
        let row = address.as_bytes();
        let sealed = cipher.seal_at(device_id, target.table, target.column, row, record)?;
        let binds = binds![&**address, sealed, device_id];
        tx.execute(target.sql, binds).await?;
    }
    tx.commit().await
}

async fn delete_by_address(
    conn: &TursoConn,
    sql: &str,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<()> {
    if addresses.is_empty() {
        return Ok(());
    }
    let tx = TursoTx::begin(conn).await?;
    for address in addresses {
        tx.execute(sql, binds![&**address, device_id]).await?;
    }
    tx.commit().await
}

/// Only the addresses that exist, in no particular order.
pub(super) async fn get_sessions(
    conn: &TursoConn,
    cipher: &BlobCipher,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<Vec<(Arc<str>, Bytes)>> {
    let mut found: Vec<(Arc<str>, Bytes)> = Vec::new();
    for chunk in padded_chunks(addresses) {
        let list = chunk.iter().map(|address| (&**address).into());
        let binds = with_list(binds![device_id], list);
        for row in conn.fetch_all(SELECT_SESSIONS.as_str(), binds).await? {
            let address = text(&row, 0)?;
            let stored = blob(&row, 1)?;
            let record =
                cipher.open_at(device_id, "sessions", "record", address.as_bytes(), &stored)?;
            found.push((Arc::from(address), Bytes::from(record)));
        }
    }
    Ok(found)
}

/// Only the ids that exist, in no particular order.
pub(super) async fn load_prekeys(
    conn: &TursoConn,
    cipher: &BlobCipher,
    device_id: i32,
    ids: &[u32],
) -> Result<Vec<(u32, Bytes)>> {
    let mut found: Vec<(u32, Bytes)> = Vec::new();
    for chunk in padded_chunks(ids) {
        let list = chunk.iter().map(|id| (*id as i32).into());
        let binds = with_list(binds![device_id], list);
        for row in conn.fetch_all(SELECT_PREKEYS.as_str(), binds).await? {
            let id = int32(&row, 0)?;
            let stored = blob(&row, 1)?;
            let record = cipher.open_at(device_id, "prekeys", "key", &id.to_be_bytes(), &stored)?;
            found.push((id as u32, Bytes::from(record)));
        }
    }
    Ok(found)
}
