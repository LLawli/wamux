//! `DeviceStore` for the SQL family. The whole `Device` is stored as one
//! protobuf blob (`blob_codec::encode_device`, #31) keyed by `device_id`.

use async_trait::async_trait;
use wacore::store::Device;
use wacore::store::error::Result;
use wacore::store::traits::DeviceStore;

use super::{SqlBackend, maintenance_sql};
use crate::storage::blob_codec::{decode_device, encode_device};
use crate::storage::statements::device::{CREATE_DEVICE, DEVICE_EXISTS, LOAD_DEVICE, SAVE_DEVICE};

#[async_trait]
impl DeviceStore for SqlBackend {
    async fn save(&self, device: &Device) -> Result<()> {
        let data = self.seal_at("device", "data", b"", &encode_device(device))?;
        execute_sql!(&self.pool, SAVE_DEVICE, self.device_id, &data)?;
        Ok(())
    }

    async fn load(&self) -> Result<Option<Device>> {
        let row = scalar_optional_sql!(Vec<u8>, &self.pool, LOAD_DEVICE, self.device_id)?;
        match row {
            None => Ok(None),
            // decode_device restores the runtime-only fields (device_props
            // included), so what comes back is ready to use.
            Some(bytes) => Ok(Some(decode_device(
                &self.open_at("device", "data", b"", &bytes)?,
            )?)),
        }
    }

    async fn exists(&self) -> Result<bool> {
        scalar_one_sql!(bool, &self.pool, DEVICE_EXISTS, self.device_id)
    }

    async fn create(&self) -> Result<i32> {
        let data = self.seal_at("device", "data", b"", &encode_device(&Device::new()))?;
        execute_sql!(&self.pool, CREATE_DEVICE, self.device_id, &data)?;
        Ok(self.device_id)
    }

    /// Periodic upkeep, per engine in `maintenance_sql` (#104).
    async fn maintenance(&self) -> Result<()> {
        maintenance_sql::run(&self.pool).await
    }
}
