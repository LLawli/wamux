//! Postgres half of `storage::bincode_upgrade`. Mirrors
//! `sqlite/bincode_upgrade.rs` statement for statement; only the dialect differs.

use sqlx::{PgConnection, PgPool};

use crate::storage::bincode_upgrade::{
    BLOB_FORMAT_PROTOBUF, BincodeUpgradeError, BincodeUpgradePlan, LegacyBlobRows, log_upgrade,
    needs_bincode_upgrade, plan_bincode_upgrade, upgrade_db_error,
};

/// Convert the store if `blob_format` says it is still bincode: read, plan and
/// prove every row, write them and flip the marker, all in one transaction.
pub(super) async fn upgrade_bincode_blobs(pool: &PgPool) -> Result<(), BincodeUpgradeError> {
    let mut tx = pool.begin().await.map_err(upgrade_db_error("begin"))?;
    // FOR UPDATE: two processes opening one store at once (a daemon, the
    // test binaries) serialize here, and the second one reads 'protobuf'.
    let format: String =
        sqlx::query_scalar("SELECT format FROM blob_format WHERE id = 1 FOR UPDATE")
            .fetch_one(&mut *tx)
            .await
            .map_err(upgrade_db_error("reading blob_format"))?;
    if !needs_bincode_upgrade(&format)? {
        return Ok(());
    }
    let plan = plan_bincode_upgrade(read_legacy_rows(&mut tx).await?)?;
    write_plan(&mut tx, &plan).await?;
    sqlx::query("UPDATE blob_format SET format = $1 WHERE id = 1")
        .bind(BLOB_FORMAT_PROTOBUF)
        .execute(&mut *tx)
        .await
        .map_err(upgrade_db_error("updating blob_format"))?;
    tx.commit().await.map_err(upgrade_db_error("commit"))?;
    log_upgrade(&plan);
    Ok(())
}

async fn read_legacy_rows(conn: &mut PgConnection) -> Result<LegacyBlobRows, BincodeUpgradeError> {
    let devices = sqlx::query_as("SELECT device_id, data FROM device ORDER BY device_id")
        .fetch_all(&mut *conn)
        .await
        .map_err(upgrade_db_error("reading device"))?;
    let versions = sqlx::query_as(
        "SELECT device_id, name, state_data FROM app_state_versions ORDER BY device_id, name",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(upgrade_db_error("reading app_state_versions"))?;
    let sync_keys = sqlx::query_as(
        "SELECT device_id, key_id, key_data FROM app_state_keys ORDER BY device_id, key_id",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(upgrade_db_error("reading app_state_keys"))?;
    Ok(LegacyBlobRows {
        devices,
        versions,
        sync_keys,
    })
}

async fn write_plan(
    conn: &mut PgConnection,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    write_devices(conn, plan).await?;
    write_versions(conn, plan).await?;
    write_sync_keys(conn, plan).await
}

async fn write_devices(
    conn: &mut PgConnection,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    for (device_id, blob) in &plan.devices {
        sqlx::query("UPDATE device SET data = $1 WHERE device_id = $2")
            .bind(blob)
            .bind(device_id)
            .execute(&mut *conn)
            .await
            .map_err(upgrade_db_error("updating device"))?;
    }
    Ok(())
}

async fn write_versions(
    conn: &mut PgConnection,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    for (device_id, name, blob) in &plan.versions {
        sqlx::query(
            "UPDATE app_state_versions SET state_data = $1 WHERE device_id = $2 AND name = $3",
        )
        .bind(blob)
        .bind(device_id)
        .bind(name)
        .execute(&mut *conn)
        .await
        .map_err(upgrade_db_error("updating app_state_versions"))?;
    }
    Ok(())
}

async fn write_sync_keys(
    conn: &mut PgConnection,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    for (device_id, key_id, blob) in &plan.sync_keys {
        sqlx::query("UPDATE app_state_keys SET key_data = $1 WHERE device_id = $2 AND key_id = $3")
            .bind(blob)
            .bind(device_id)
            .bind(key_id)
            .execute(&mut *conn)
            .await
            .map_err(upgrade_db_error("updating app_state_keys"))?;
    }
    Ok(())
}
