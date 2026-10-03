//! Row shapes and conversions for `ProtocolStore`. Plain functions, kept out of
//! `protocol_store.rs` (which holds only the trait methods the CI coverage
//! check counts) and so that file stays far from the 500-line cap.

use wacore::store::error::{Result, StoreError};
use wacore::store::traits::{DeviceInfo, DeviceListRecord, LidPnMappingEntry};

/// `lid, phone_number, created_at, learning_source, updated_at`, in the column
/// order every `lid_pn_mapping` SELECT uses.
pub(super) type LidMappingRow = (String, String, i64, String, i64);

/// `user_id, devices_json, timestamp, phash, raw_id` of `device_registry`.
pub(super) type DeviceListRow = (String, String, i64, Option<String>, Option<i32>);

pub(super) fn lid_entry(row: LidMappingRow) -> LidPnMappingEntry {
    let (lid, phone_number, created_at, learning_source, updated_at) = row;
    LidPnMappingEntry {
        lid,
        phone_number,
        created_at,
        updated_at,
        learning_source,
    }
}

/// `record.devices` is `Box<[DeviceInfo]>` on main; a slice serializes to the
/// same JSON array a `Vec` did, so the stored blob is unchanged.
pub(super) fn devices_json(record: &DeviceListRecord) -> Result<String> {
    serde_json::to_string(&*record.devices).map_err(|e| StoreError::Serialization(Box::new(e)))
}

pub(super) fn device_list_record(row: DeviceListRow) -> Result<DeviceListRecord> {
    let (user, devices_json, timestamp, phash, raw_id) = row;
    let devices: Vec<DeviceInfo> =
        serde_json::from_str(&devices_json).map_err(|e| StoreError::Serialization(Box::new(e)))?;
    Ok(DeviceListRecord {
        user: user.into(),
        devices: devices.into_boxed_slice(),
        timestamp,
        phash: phash.map(String::into_boxed_str),
        raw_id: raw_id.map(|r| r as u32),
    })
}
