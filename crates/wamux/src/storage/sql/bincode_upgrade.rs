//! The SQL half of `storage::bincode_upgrade`, written once for both drivers.
//! The only statement that differs is the `blob_format` read (see
//! `select_blob_format_sql`).

use super::{SqlPool, SqlTx};
use crate::storage::bincode_upgrade::{
    BLOB_FORMAT_PROTOBUF, BincodeUpgradeError, BincodeUpgradePlan, LegacyBlobRows, log_upgrade,
    needs_bincode_upgrade, plan_bincode_upgrade, upgrade_db_error,
};

/// Convert the store if `blob_format` says it is still bincode: read, plan and
/// prove every row, write them and flip the marker, all in one transaction.
pub(super) async fn upgrade_bincode_blobs(pool: &SqlPool) -> Result<(), BincodeUpgradeError> {
    let mut tx = SqlTx::begin_raw(pool)
        .await
        .map_err(upgrade_db_error("begin"))?;
    let format = read_blob_format(pool, &mut tx).await?;
    if !needs_bincode_upgrade(&format)? {
        return Ok(());
    }
    let plan = plan_bincode_upgrade(read_legacy_rows(&mut tx).await?)?;
    write_plan(&mut tx, &plan).await?;
    mark_protobuf(&mut tx).await?;
    tx.commit_raw().await.map_err(upgrade_db_error("commit"))?;
    log_upgrade(&plan);
    Ok(())
}

/// The one statement written per driver here. Postgres locks the row with
/// FOR UPDATE: two processes opening one store at once (a daemon, the test
/// binaries) serialize here, and the second one reads 'protobuf'. SQLite has no
/// row lock, and needs none: the pool is one connection (see `connect_sqlite`),
/// and the file has one daemon.
fn select_blob_format_sql(pool: &SqlPool) -> &'static str {
    match pool {
        SqlPool::Pg(_) => "SELECT format FROM blob_format WHERE id = 1 FOR UPDATE",
        SqlPool::Sqlite(_) => "SELECT format FROM blob_format WHERE id = 1",
    }
}

async fn read_blob_format(
    pool: &SqlPool,
    tx: &mut SqlTx<'_>,
) -> Result<String, BincodeUpgradeError> {
    let sql = select_blob_format_sql(pool);
    on_sql_tx!(*tx, |conn| {
        sqlx::query_scalar(sql).fetch_one(conn).await
    })
    .map_err(upgrade_db_error("reading blob_format"))
}

async fn mark_protobuf(tx: &mut SqlTx<'_>) -> Result<(), BincodeUpgradeError> {
    on_sql_tx!(*tx, |conn| {
        sqlx::query("UPDATE blob_format SET format = $1 WHERE id = 1")
            .bind(BLOB_FORMAT_PROTOBUF)
            .execute(conn)
            .await
            .map(drop)
    })
    .map_err(upgrade_db_error("updating blob_format"))
}

async fn read_legacy_rows(tx: &mut SqlTx<'_>) -> Result<LegacyBlobRows, BincodeUpgradeError> {
    let devices = on_sql_tx!(*tx, |conn| {
        sqlx::query_as("SELECT device_id, data FROM device ORDER BY device_id")
            .fetch_all(conn)
            .await
    })
    .map_err(upgrade_db_error("reading device"))?;
    let versions = on_sql_tx!(*tx, |conn| {
        sqlx::query_as(
            "SELECT device_id, name, state_data FROM app_state_versions ORDER BY device_id, name",
        )
        .fetch_all(conn)
        .await
    })
    .map_err(upgrade_db_error("reading app_state_versions"))?;
    let sync_keys = on_sql_tx!(*tx, |conn| {
        sqlx::query_as(
            "SELECT device_id, key_id, key_data FROM app_state_keys ORDER BY device_id, key_id",
        )
        .fetch_all(conn)
        .await
    })
    .map_err(upgrade_db_error("reading app_state_keys"))?;
    Ok(LegacyBlobRows {
        devices,
        versions,
        sync_keys,
    })
}

async fn write_plan(
    tx: &mut SqlTx<'_>,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    write_devices(tx, plan).await?;
    write_versions(tx, plan).await?;
    write_sync_keys(tx, plan).await
}

async fn write_devices(
    tx: &mut SqlTx<'_>,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    for (device_id, blob) in &plan.devices {
        on_sql_tx!(*tx, |conn| {
            sqlx::query("UPDATE device SET data = $1 WHERE device_id = $2")
                .bind(blob)
                .bind(device_id)
                .execute(conn)
                .await
                .map(drop)
        })
        .map_err(upgrade_db_error("updating device"))?;
    }
    Ok(())
}

async fn write_versions(
    tx: &mut SqlTx<'_>,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    for (device_id, name, blob) in &plan.versions {
        on_sql_tx!(*tx, |conn| {
            sqlx::query(
                "UPDATE app_state_versions SET state_data = $1 WHERE device_id = $2 AND name = $3",
            )
            .bind(blob)
            .bind(device_id)
            .bind(name)
            .execute(conn)
            .await
            .map(drop)
        })
        .map_err(upgrade_db_error("updating app_state_versions"))?;
    }
    Ok(())
}

async fn write_sync_keys(
    tx: &mut SqlTx<'_>,
    plan: &BincodeUpgradePlan,
) -> Result<(), BincodeUpgradeError> {
    for (device_id, key_id, blob) in &plan.sync_keys {
        on_sql_tx!(*tx, |conn| {
            sqlx::query(
                "UPDATE app_state_keys SET key_data = $1 WHERE device_id = $2 AND key_id = $3",
            )
            .bind(blob)
            .bind(device_id)
            .bind(key_id)
            .execute(conn)
            .await
            .map(drop)
        })
        .map_err(upgrade_db_error("updating app_state_keys"))?;
    }
    Ok(())
}
