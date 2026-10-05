//! `SignalStore` statements shared by the one-row methods and the batches
//! (#104). Kept out of `signal_store.rs`: the coverage checks read every
//! `async fn` in `*_store.rs` as a trait method.
//!
//! Writes: the one-row statement in a loop inside ONE transaction, so a batch
//! is one connection and one commit, all or nothing. A key repeated in a batch
//! is written in order, so the last one wins. Reads: one query per
//! `READ_CHUNK` values (`batch_chunks`).

use std::sync::Arc;

use bytes::Bytes;
use wacore::store::error::Result;

use super::SqlPool;
use super::SqlTx;
use crate::storage::batch_chunks::padded_chunks;
use crate::storage::statements::signal::{
    DELETE_IDENTITY, DELETE_PREKEY, DELETE_SENDER_KEY, DELETE_SESSION, PUT_IDENTITY,
    PUT_SENDER_KEY, PUT_SESSION, SELECT_PREKEYS, SELECT_SESSIONS, STORE_PREKEY,
};

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
