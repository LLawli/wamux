//! `DeviceStore` over Postgres. The whole `Device` is stored as one protobuf
//! blob (`blob_codec::encode_device`, #31) keyed by `device_id`.

use async_trait::async_trait;
use wacore::store::Device;
use wacore::store::error::Result;
use wacore::store::traits::DeviceStore;

use super::PgBackend;
use crate::storage::blob_codec::{decode_device, encode_device};
use crate::storage::sqlx_error::db;

#[async_trait]
impl DeviceStore for PgBackend {
    async fn save(&self, device: &Device) -> Result<()> {
        let data = encode_device(device);
        sqlx::query(
            "INSERT INTO device (device_id, data) VALUES ($1, $2)
             ON CONFLICT (device_id) DO UPDATE SET data = EXCLUDED.data",
        )
        .bind(self.device_id)
        .bind(&data)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn load(&self) -> Result<Option<Device>> {
        let row: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT data FROM device WHERE device_id = $1")
                .bind(self.device_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        match row {
            None => Ok(None),
            // decode_device restores the runtime-only fields (device_props
            // included), so what comes back is ready to use.
            Some(bytes) => Ok(Some(decode_device(&bytes)?)),
        }
    }

    async fn exists(&self) -> Result<bool> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM device WHERE device_id = $1)")
                .bind(self.device_id)
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
        Ok(exists)
    }

    async fn create(&self) -> Result<i32> {
        let data = encode_device(&Device::new());
        sqlx::query(
            "INSERT INTO device (device_id, data) VALUES ($1, $2)
             ON CONFLICT (device_id) DO NOTHING",
        )
        .bind(self.device_id)
        .bind(&data)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(self.device_id)
    }
}
