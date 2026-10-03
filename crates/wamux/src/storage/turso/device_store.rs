//! `DeviceStore` for the turso family (#106). The whole `Device` is stored as
//! one protobuf blob (`blob_codec::encode_device`, #31) keyed by `device_id`.

use async_trait::async_trait;
use wacore::store::Device;
use wacore::store::error::Result;
use wacore::store::traits::DeviceStore;

use super::exec::binds;
use super::row_values::first_blob;
use super::{TursoBackend, maintenance_sql};
use crate::storage::blob_codec::{decode_device, encode_device};
use crate::storage::statements::device::{CREATE_DEVICE, DEVICE_EXISTS, LOAD_DEVICE, SAVE_DEVICE};

#[async_trait]
impl DeviceStore for TursoBackend {
    async fn save(&self, device: &Device) -> Result<()> {
        let data = encode_device(device);
        let binds = binds![self.device_id, data];
        self.conn.execute(SAVE_DEVICE, binds).await?;
        Ok(())
    }

    async fn load(&self) -> Result<Option<Device>> {
        let row = self
            .conn
            .fetch_optional(LOAD_DEVICE, binds![self.device_id])
            .await?;
        match first_blob(row)? {
            None => Ok(None),
            // decode_device restores the runtime-only fields (device_props
            // included), so what comes back is ready to use.
            Some(bytes) => Ok(Some(decode_device(&bytes)?)),
        }
    }

    async fn exists(&self) -> Result<bool> {
        let binds = binds![self.device_id];
        let row = self.conn.fetch_optional(DEVICE_EXISTS, binds).await?;
        Ok(row.map(|row| super::row_values::int(&row, 0)).transpose()? == Some(1))
    }

    async fn create(&self) -> Result<i32> {
        let data = encode_device(&Device::new());
        let binds = binds![self.device_id, data];
        self.conn.execute(CREATE_DEVICE, binds).await?;
        Ok(self.device_id)
    }

    /// Periodic upkeep, per engine in `maintenance_sql` (#104).
    async fn maintenance(&self) -> Result<()> {
        maintenance_sql::run(&self.conn).await
    }
}
