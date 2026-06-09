//! `DeviceStore` over Postgres. The whole `Device` is stored as one
//! bincode-standard blob keyed by `device_id`.

use async_trait::async_trait;
use wacore::store::Device;
use wacore::store::error::Result;
use wacore::store::traits::DeviceStore;

use super::error_map::db;
use super::{PgBackend, bincode_decode, bincode_encode};

#[async_trait]
impl DeviceStore for PgBackend {
    async fn save(&self, device: &Device) -> Result<()> {
        let data = bincode_encode(device)?;
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
            Some(bytes) => {
                let mut device: Device = bincode_decode(&bytes)?;
                // Runtime-only fields are #[serde(skip)] -> Default on decode.
                // Restore device_props like the reference does; ClientProfile's
                // Default already equals ::web().
                device.device_props = wacore::store::device::DEVICE_PROPS.clone();
                Ok(Some(device))
            }
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
        let data = bincode_encode(&Device::new())?;
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
