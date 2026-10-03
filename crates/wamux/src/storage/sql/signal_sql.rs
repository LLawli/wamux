//! `SignalStore` statements shared by the one-row methods and the batches
//! (#104). Kept out of `signal_store.rs`: the coverage checks read every
//! `async fn` in `*_store.rs` as a trait method.
//!
//! Writes: the one-row statement in a loop inside ONE transaction, so a batch
//! is one connection and one commit, all or nothing. A key repeated in a batch
//! is written in order, so the last one wins. Reads: one query per
//! `READ_CHUNK` values (`batch_sql`).

use std::sync::{Arc, LazyLock};

use bytes::Bytes;
use wacore::store::error::Result;

use super::SqlPool;
use super::SqlTx;
use super::batch_sql::{READ_CHUNK, in_placeholders, padded_chunks};

pub(super) const PUT_IDENTITY: &str = "INSERT INTO identities (address, key, device_id)
     VALUES ($1, $2, $3)
     ON CONFLICT (address, device_id) DO UPDATE SET key = EXCLUDED.key";
pub(super) const DELETE_IDENTITY: &str =
    "DELETE FROM identities WHERE address = $1 AND device_id = $2";
pub(super) const PUT_SESSION: &str = "INSERT INTO sessions (address, record, device_id)
     VALUES ($1, $2, $3)
     ON CONFLICT (address, device_id) DO UPDATE SET record = EXCLUDED.record";
pub(super) const DELETE_SESSION: &str =
    "DELETE FROM sessions WHERE address = $1 AND device_id = $2";
pub(super) const STORE_PREKEY: &str = "INSERT INTO prekeys (id, key, uploaded, device_id)
     VALUES ($1, $2, $3, $4)
     ON CONFLICT (id, device_id) DO UPDATE
     SET key = EXCLUDED.key, uploaded = EXCLUDED.uploaded";
pub(super) const DELETE_PREKEY: &str = "DELETE FROM prekeys WHERE id = $1 AND device_id = $2";
pub(super) const PUT_SENDER_KEY: &str = "INSERT INTO sender_keys (address, record, device_id)
     VALUES ($1, $2, $3)
     ON CONFLICT (address, device_id) DO UPDATE SET record = EXCLUDED.record";
pub(super) const DELETE_SENDER_KEY: &str =
    "DELETE FROM sender_keys WHERE address = $1 AND device_id = $2";

// The statement text is the same for every call and never varies by
// deployment, so it is built once, not per call.
static SELECT_SESSIONS: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(2, READ_CHUNK);
    format!("SELECT address, record FROM sessions WHERE device_id = $1 AND address IN ({list})")
});
static SELECT_PREKEYS: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(2, READ_CHUNK);
    format!("SELECT id, key FROM prekeys WHERE device_id = $1 AND id IN ({list})")
});

pub(super) async fn put_identities(
    pool: &SqlPool,
    device_id: i32,
    identities: &[(Arc<str>, [u8; 32])],
) -> Result<()> {
    if identities.is_empty() {
        return Ok(());
    }
    let mut tx = SqlTx::begin(pool).await?;
    for (address, key) in identities {
        execute_sql!(in tx, PUT_IDENTITY, &**address, &key[..], device_id)?;
    }
    tx.commit().await
}

pub(super) async fn delete_identities(
    pool: &SqlPool,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<()> {
    delete_by_address(pool, DELETE_IDENTITY, device_id, addresses).await
}

pub(super) async fn put_sessions(
    pool: &SqlPool,
    device_id: i32,
    sessions: &[(Arc<str>, Bytes)],
) -> Result<()> {
    put_address_records(pool, PUT_SESSION, device_id, sessions).await
}

pub(super) async fn delete_sessions(
    pool: &SqlPool,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<()> {
    delete_by_address(pool, DELETE_SESSION, device_id, addresses).await
}

pub(super) async fn put_sender_keys(
    pool: &SqlPool,
    device_id: i32,
    sender_keys: &[(Arc<str>, Bytes)],
) -> Result<()> {
    put_address_records(pool, PUT_SENDER_KEY, device_id, sender_keys).await
}

pub(super) async fn delete_sender_keys(
    pool: &SqlPool,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<()> {
    delete_by_address(pool, DELETE_SENDER_KEY, device_id, addresses).await
}

pub(super) async fn store_prekeys(
    pool: &SqlPool,
    device_id: i32,
    keys: &[(u32, Bytes)],
    uploaded: bool,
) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let mut tx = SqlTx::begin(pool).await?;
    for (id, record) in keys {
        execute_sql!(in tx, STORE_PREKEY, *id as i32, &record[..], uploaded, device_id)?;
    }
    tx.commit().await
}

/// The one-row DELETE in a loop: `mark_prekeys_uploaded` has a per-driver
/// statement, but a delete by id is portable as is.
pub(super) async fn remove_prekeys(pool: &SqlPool, device_id: i32, ids: &[u32]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = SqlTx::begin(pool).await?;
    for id in ids {
        execute_sql!(in tx, DELETE_PREKEY, *id as i32, device_id)?;
    }
    tx.commit().await
}

/// Sessions and sender keys share a shape: `(address, bytes)` upserted by address.
async fn put_address_records(
    pool: &SqlPool,
    sql: &str,
    device_id: i32,
    records: &[(Arc<str>, Bytes)],
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let mut tx = SqlTx::begin(pool).await?;
    for (address, record) in records {
        execute_sql!(in tx, sql, &**address, &record[..], device_id)?;
    }
    tx.commit().await
}

async fn delete_by_address(
    pool: &SqlPool,
    sql: &str,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<()> {
    if addresses.is_empty() {
        return Ok(());
    }
    let mut tx = SqlTx::begin(pool).await?;
    for address in addresses {
        execute_sql!(in tx, sql, &**address, device_id)?;
    }
    tx.commit().await
}

/// Only the addresses that exist, in no particular order.
pub(super) async fn get_sessions(
    pool: &SqlPool,
    device_id: i32,
    addresses: &[Arc<str>],
) -> Result<Vec<(Arc<str>, Bytes)>> {
    let mut found: Vec<(Arc<str>, Bytes)> = Vec::new();
    for chunk in padded_chunks(addresses) {
        let rows = rows_in_list_sql!(
            (String, Vec<u8>),
            pool,
            SELECT_SESSIONS.as_str(),
            [device_id],
            chunk.iter().map(|address| &**address)
        )?;
        found.extend(
            rows.into_iter()
                .map(|(a, r)| (Arc::from(a), Bytes::from(r))),
        );
    }
    Ok(found)
}

/// Only the ids that exist, in no particular order.
pub(super) async fn load_prekeys(
    pool: &SqlPool,
    device_id: i32,
    ids: &[u32],
) -> Result<Vec<(u32, Bytes)>> {
    let mut found: Vec<(u32, Bytes)> = Vec::new();
    for chunk in padded_chunks(ids) {
        let rows = rows_in_list_sql!(
            (i32, Vec<u8>),
            pool,
            SELECT_PREKEYS.as_str(),
            [device_id],
            chunk.iter().map(|id| *id as i32)
        )?;
        found.extend(rows.into_iter().map(|(id, k)| (id as u32, Bytes::from(k))));
    }
    Ok(found)
}
