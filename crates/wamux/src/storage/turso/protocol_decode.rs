//! Turso rows into the neutral row shapes of `storage::protocol_rows` (#106).

use ::turso::Row;
use wacore::store::error::Result;
use wacore::store::traits::TcTokenEntry;

use super::row_values::{blob, int, int32, opt_int, opt_int32, opt_text, text};
use crate::storage::blob_cipher::BlobCipher;
use crate::storage::protocol_rows::{DeviceListRow, LidMappingRow};

/// `lid, phone_number, created_at, learning_source, updated_at`.
pub(super) fn lid_mapping_row(row: &Row) -> Result<LidMappingRow> {
    Ok((
        text(row, 0)?,
        text(row, 1)?,
        int(row, 2)?,
        text(row, 3)?,
        int(row, 4)?,
    ))
}

/// `user_id, devices_json, timestamp, phash, raw_id`.
pub(super) fn device_list_row(row: &Row) -> Result<DeviceListRow> {
    Ok((
        text(row, 0)?,
        text(row, 1)?,
        int(row, 2)?,
        opt_text(row, 3)?,
        opt_int32(row, 4)?,
    ))
}

/// `token, token_timestamp, sender_timestamp` starting at column `first`. The
/// token is opened for `jid` (its row key) with the account's cipher (#164).
pub(super) fn tc_token_entry(
    row: &Row,
    first: usize,
    cipher: &BlobCipher,
    device_id: i32,
    jid: &str,
) -> Result<TcTokenEntry> {
    let stored = blob(row, first)?;
    Ok(TcTokenEntry {
        token: cipher.open_at(device_id, "tc_tokens", "token", jid.as_bytes(), &stored)?,
        token_timestamp: int(row, first + 1)?,
        sender_timestamp: opt_int(row, first + 2)?,
    })
}

/// `device_jid, has_key` of `sender_key_devices`.
pub(super) fn sender_key_device(row: &Row) -> Result<(String, bool)> {
    Ok((text(row, 0)?, int32(row, 1)? != 0))
}
