//! The turso half of `storage::bincode_upgrade` (#106): the same neutral
//! planner and proofs as the sql family, with this family's reads and writes.
//! Read, plan, write and flip the marker in ONE transaction: a store is never
//! half bincode and half protobuf, and a row that will not convert changes
//! nothing (the failed `open` then drops the connection, which frees the file).
//!
//! No row lock on the marker: one connection, one daemon per file.

use ::turso::Row;
use wacore::store::error::Result as StoreResult;

use super::TursoConn;
use super::TursoTx;
use super::exec::binds;
use super::row_values::{blob, int32, text};
use crate::storage::bincode_upgrade::{
    BLOB_FORMAT_PROTOBUF, BincodeUpgradeError, BincodeUpgradePlan, LegacyBlobRows, log_upgrade,
    needs_bincode_upgrade, plan_bincode_upgrade, upgrade_db_error,
};
use crate::storage::statements::bincode_upgrade::{
    MARK_BLOB_FORMAT, SELECT_BLOB_FORMAT, SELECT_LEGACY_DEVICES, SELECT_LEGACY_SYNC_KEYS,
    SELECT_LEGACY_VERSIONS, UPDATE_DEVICE_BLOB, UPDATE_SYNC_KEY_BLOB, UPDATE_VERSION_BLOB,
};

/// Convert the store if `blob_format` says it is still bincode. Whatever the
/// outcome the transaction is ended here, so the connection never goes back to
/// the store inside it.
pub(super) async fn upgrade_bincode_blobs(conn: &TursoConn) -> Result<(), BincodeUpgradeError> {
    let tx = TursoTx::begin(conn)
        .await
        .map_err(upgrade_db_error("begin"))?;
    match plan_and_write(&tx).await {
        Ok(Some(plan)) => {
            tx.commit().await.map_err(upgrade_db_error("commit"))?;
            log_upgrade(&plan);
            Ok(())
        }
        Ok(None) => {
            tx.rollback().await;
            Ok(())
        }
        Err(error) => {
            tx.rollback().await;
            Err(error)
        }
    }
}

/// `None`: already protobuf, nothing was written.
async fn plan_and_write(
    tx: &TursoTx<'_>,
) -> Result<Option<BincodeUpgradePlan>, BincodeUpgradeError> {
    let format = read_blob_format(tx).await?;
    if !needs_bincode_upgrade(&format)? {
        return Ok(None);
    }
    let plan = plan_bincode_upgrade(read_legacy_rows(tx).await?)?;
    write_plan(tx, &plan).await?;
    mark_protobuf(tx).await?;
    Ok(Some(plan))
}

async fn read_blob_format(tx: &TursoTx<'_>) -> Result<String, BincodeUpgradeError> {
    let row = tx
        .fetch_optional(SELECT_BLOB_FORMAT, binds![])
        .await
        .map_err(upgrade_db_error("reading blob_format"))?;
    let row =
        row.ok_or_else(|| BincodeUpgradeError::UnknownFormat("<no blob_format row>".to_string()))?;
    text(&row, 0).map_err(upgrade_db_error("reading blob_format"))
}

async fn mark_protobuf(tx: &TursoTx<'_>) -> Result<(), BincodeUpgradeError> {
    tx.execute(MARK_BLOB_FORMAT, binds![BLOB_FORMAT_PROTOBUF])
        .await
        .map(drop)
        .map_err(upgrade_db_error("updating blob_format"))
}

/// `(device_id, data)`-shaped rows: each table's key columns, then its blob.
async fn read_rows<T>(
    tx: &TursoTx<'_>,
    sql: &str,
    context: &'static str,
    decode: fn(&Row) -> StoreResult<T>,
) -> Result<Vec<T>, BincodeUpgradeError> {
    let rows = tx
        .fetch_all(sql, binds![])
        .await
        .map_err(upgrade_db_error(context))?;
    rows.iter()
        .map(decode)
        .collect::<StoreResult<Vec<T>>>()
        .map_err(upgrade_db_error(context))
}

async fn read_legacy_rows(tx: &TursoTx<'_>) -> Result<LegacyBlobRows, BincodeUpgradeError> {
    Ok(LegacyBlobRows {
        devices: read_rows(tx, SELECT_LEGACY_DEVICES, "reading device", |r| {
            Ok((int32(r, 0)?, blob(r, 1)?))
        })
        .await?,
        versions: read_rows(
            tx,
            SELECT_LEGACY_VERSIONS,
            "reading app_state_versions",
            |r| Ok((int32(r, 0)?, text(r, 1)?, blob(r, 2)?)),
        )
        .await?,
        sync_keys: read_rows(tx, SELECT_LEGACY_SYNC_KEYS, "reading app_state_keys", |r| {
            Ok((int32(r, 0)?, blob(r, 1)?, blob(r, 2)?))
        })
        .await?,
    })
}

async fn write_plan(
    tx: &TursoTx<'_>,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    write_devices(tx, plan).await?;
    write_versions(tx, plan).await?;
    write_sync_keys(tx, plan).await
}

async fn write_devices(
    tx: &TursoTx<'_>,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    for (device_id, blob) in &plan.devices {
        let binds = binds![blob.as_slice(), *device_id];
        let written = tx.execute(UPDATE_DEVICE_BLOB, binds).await;
        written.map_err(upgrade_db_error("updating device"))?;
    }
    Ok(())
}

async fn write_versions(
    tx: &TursoTx<'_>,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    for (device_id, name, blob) in &plan.versions {
        let binds = binds![blob.as_slice(), *device_id, name.as_str()];
        let written = tx.execute(UPDATE_VERSION_BLOB, binds).await;
        written.map_err(upgrade_db_error("updating app_state_versions"))?;
    }
    Ok(())
}

async fn write_sync_keys(
    tx: &TursoTx<'_>,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    for (device_id, key_id, blob) in &plan.sync_keys {
        let binds = binds![blob.as_slice(), *device_id, key_id.as_slice()];
        let written = tx.execute(UPDATE_SYNC_KEY_BLOB, binds).await;
        written.map_err(upgrade_db_error("updating app_state_keys"))?;
    }
    Ok(())
}
