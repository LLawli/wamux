//! `ProtocolStore` statements shared by the one-row methods and the batched
//! ones (#104). Kept out of `protocol_store.rs` for the `*_store.rs` rule and
//! its 500-line cap.

use std::collections::HashMap;
use std::sync::LazyLock;

use wacore::store::error::Result;
use wacore::store::traits::{DeviceListRecord, LidPnMappingEntry, TcTokenEntry};

use super::batch_sql::{READ_CHUNK, in_placeholders, padded_chunks};
use super::protocol_rows::{self, DeviceListRow};
use super::{SqlPool, SqlTx};
use crate::storage::blob_codec::now_secs;

pub(super) const PUT_LID_MAPPING: &str = "INSERT INTO lid_pn_mapping
        (lid, phone_number, created_at, learning_source, updated_at, device_id)
     VALUES ($1, $2, $3, $4, $5, $6)
     ON CONFLICT (lid, device_id) DO UPDATE SET
        phone_number = EXCLUDED.phone_number,
        learning_source = EXCLUDED.learning_source,
        updated_at = EXCLUDED.updated_at";
pub(super) const UPDATE_DEVICE_LIST: &str = "INSERT INTO device_registry
        (user_id, devices_json, timestamp, phash, device_id, updated_at, raw_id)
     VALUES ($1, $2, $3, $4, $5, $6, $7)
     ON CONFLICT (user_id, device_id) DO UPDATE SET
        devices_json = EXCLUDED.devices_json,
        timestamp = EXCLUDED.timestamp,
        phash = EXCLUDED.phash,
        updated_at = EXCLUDED.updated_at,
        raw_id = EXCLUDED.raw_id";

// Same text on every call and never varies by deployment: built once.
static SELECT_DEVICES: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(2, READ_CHUNK);
    format!(
        "SELECT user_id, devices_json, timestamp, phash, raw_id
         FROM device_registry WHERE device_id = $1 AND user_id IN ({list})"
    )
});
static SELECT_TC_TOKENS: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(2, READ_CHUNK);
    format!(
        "SELECT jid, token, token_timestamp, sender_timestamp
         FROM tc_tokens WHERE device_id = $1 AND jid IN ({list})"
    )
});

pub(super) async fn put_lid_mappings(
    pool: &SqlPool,
    device_id: i32,
    entries: &[LidPnMappingEntry],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut tx = SqlTx::begin(pool).await?;
    for e in entries {
        let (lid, phone, source) = (&e.lid, &e.phone_number, &e.learning_source);
        execute_sql!(
            in tx,
            PUT_LID_MAPPING,
            lid,
            phone,
            e.created_at,
            source,
            e.updated_at,
            device_id
        )?;
    }
    tx.commit().await
}

pub(super) async fn update_device_lists(
    pool: &SqlPool,
    device_id: i32,
    records: Vec<DeviceListRecord>,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let now = now_secs();
    let mut tx = SqlTx::begin(pool).await?;
    for record in &records {
        put_device_list_in(&mut tx, device_id, now, record).await?;
    }
    tx.commit().await
}

async fn put_device_list_in(
    mut tx: &mut SqlTx<'_>,
    device_id: i32,
    now: i64,
    record: &DeviceListRecord,
) -> Result<()> {
    let devices_json = protocol_rows::devices_json(record)?;
    let (phash, raw_id) = (record.phash.as_deref(), record.raw_id.map(|r| r as i32));
    execute_sql!(
        in tx,
        UPDATE_DEVICE_LIST,
        &*record.user,
        &devices_json,
        record.timestamp,
        phash,
        device_id,
        now,
        raw_id
    )?;
    Ok(())
}

/// Absent users are left out, in no particular order.
pub(super) async fn get_devices_batch(
    pool: &SqlPool,
    device_id: i32,
    users: &[&str],
) -> Result<Vec<DeviceListRecord>> {
    let mut found: Vec<DeviceListRecord> = Vec::new();
    for chunk in padded_chunks(users) {
        let rows = rows_in_list_sql!(
            DeviceListRow,
            pool,
            SELECT_DEVICES.as_str(),
            [device_id],
            chunk.iter().copied()
        )?;
        for row in rows {
            found.push(protocol_rows::device_list_record(row)?);
        }
    }
    Ok(found)
}

/// In the order asked, `None` where there is no row, repeated positions kept:
/// the rows are queried per chunk of distinct jids, then mapped back by jid.
pub(super) async fn get_tc_tokens(
    pool: &SqlPool,
    device_id: i32,
    jids: &[String],
) -> Result<Vec<Option<TcTokenEntry>>> {
    let mut by_jid: HashMap<String, TcTokenEntry> = HashMap::new();
    for chunk in padded_chunks(jids) {
        let rows = rows_in_list_sql!(
            (String, Vec<u8>, i64, Option<i64>),
            pool,
            SELECT_TC_TOKENS.as_str(),
            [device_id],
            chunk.iter().map(String::as_str)
        )?;
        for (jid, token, token_timestamp, sender_timestamp) in rows {
            let entry = TcTokenEntry {
                token,
                token_timestamp,
                sender_timestamp,
            };
            by_jid.insert(jid, entry);
        }
    }
    Ok(jids.iter().map(|jid| by_jid.get(jid).cloned()).collect())
}
