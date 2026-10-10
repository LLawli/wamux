//! `ProtocolStore` helpers shared by the one-row methods and the batched ones
//! (#106, the turso twin of `sql::protocol_batch_sql`). Kept out of
//! `protocol_store.rs` for the `*_store.rs` rule and its 500-line cap.

use std::collections::HashMap;

use wacore::store::error::Result;
use wacore::store::traits::{DeviceListRecord, LidPnMappingEntry, TcTokenEntry};

use super::exec::{binds, with_list};
use super::protocol_decode::{device_list_row, tc_token_entry};
use super::row_values::text;
use super::{TursoConn, TursoTx};
use crate::storage::batch_chunks::padded_chunks;
use crate::storage::blob_cipher::BlobCipher;
use crate::storage::blob_codec::now_secs;
use crate::storage::protocol_rows;
use crate::storage::statements::protocol::{
    PUT_LID_MAPPING, SELECT_DEVICES, SELECT_TC_TOKENS, UPDATE_DEVICE_LIST,
};

pub(super) async fn put_lid_mappings(
    conn: &TursoConn,
    device_id: i32,
    entries: &[LidPnMappingEntry],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let tx = TursoTx::begin(conn).await?;
    for e in entries {
        let (lid, phone, source) = (&e.lid, &e.phone_number, &e.learning_source);
        let binds = binds![
            lid.as_str(),
            phone.as_str(),
            e.created_at,
            source.as_str(),
            e.updated_at,
            device_id
        ];
        tx.execute(PUT_LID_MAPPING, binds).await?;
    }
    tx.commit().await
}

pub(super) async fn update_device_lists(
    conn: &TursoConn,
    device_id: i32,
    records: Vec<DeviceListRecord>,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let now = now_secs();
    let tx = TursoTx::begin(conn).await?;
    for record in &records {
        put_device_list_in(&tx, device_id, now, record).await?;
    }
    tx.commit().await
}

async fn put_device_list_in(
    tx: &TursoTx<'_>,
    device_id: i32,
    now: i64,
    record: &DeviceListRecord,
) -> Result<()> {
    let devices_json = protocol_rows::devices_json(record)?;
    let (phash, raw_id) = (record.phash.as_deref(), record.raw_id.map(|r| r as i32));
    let binds = binds![
        &*record.user,
        devices_json,
        record.timestamp,
        phash,
        device_id,
        now,
        raw_id
    ];
    tx.execute(UPDATE_DEVICE_LIST, binds).await?;
    Ok(())
}

/// Absent users are left out, in no particular order.
pub(super) async fn get_devices_batch(
    conn: &TursoConn,
    device_id: i32,
    users: &[&str],
) -> Result<Vec<DeviceListRecord>> {
    let mut found: Vec<DeviceListRecord> = Vec::new();
    for chunk in padded_chunks(users) {
        let binds = with_list(binds![device_id], chunk.iter().map(|user| (*user).into()));
        for row in conn.fetch_all(SELECT_DEVICES.as_str(), binds).await? {
            found.push(protocol_rows::device_list_record(device_list_row(&row)?)?);
        }
    }
    Ok(found)
}

/// In the order asked, `None` where there is no row, repeated positions kept:
/// the rows are queried per chunk of distinct jids, then mapped back by jid.
pub(super) async fn get_tc_tokens(
    conn: &TursoConn,
    cipher: &BlobCipher,
    device_id: i32,
    jids: &[String],
) -> Result<Vec<Option<TcTokenEntry>>> {
    let mut by_jid: HashMap<String, TcTokenEntry> = HashMap::new();
    for chunk in padded_chunks(jids) {
        let binds = with_list(
            binds![device_id],
            chunk.iter().map(|jid| jid.as_str().into()),
        );
        for row in conn.fetch_all(SELECT_TC_TOKENS.as_str(), binds).await? {
            let jid = text(&row, 0)?;
            let entry = tc_token_entry(&row, 1, cipher, device_id, &jid)?;
            by_jid.insert(jid, entry);
        }
    }
    Ok(jids.iter().map(|jid| by_jid.get(jid).cloned()).collect())
}
