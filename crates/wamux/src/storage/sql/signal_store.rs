//! `SignalStore` for the SQL family. All key material is stored as raw bytes,
//! matching the whatsapp-rust sqlite reference (no transform).

use async_trait::async_trait;
use bytes::Bytes;
use wacore::store::error::{Result, StoreError};
use wacore::store::traits::SignalStore;

use super::{SqlBackend, prekeys_sql};

#[async_trait]
impl SignalStore for SqlBackend {
    // --- Identities ---

    async fn put_identity(&self, address: &str, key: [u8; 32]) -> Result<()> {
        execute_sql!(
            &self.pool,
            "INSERT INTO identities (address, key, device_id) VALUES ($1, $2, $3)
             ON CONFLICT (address, device_id) DO UPDATE SET key = EXCLUDED.key",
            address,
            &key[..],
            self.device_id
        )?;
        Ok(())
    }

    async fn load_identity(&self, address: &str) -> Result<Option<[u8; 32]>> {
        let row = scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            "SELECT key FROM identities WHERE address = $1 AND device_id = $2",
            address,
            self.device_id
        )?;
        match row {
            None => Ok(None),
            Some(bytes) => {
                let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    StoreError::Validation(format!("Invalid identity key length: {}", bytes.len()))
                })?;
                Ok(Some(arr))
            }
        }
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM identities WHERE address = $1 AND device_id = $2",
            address,
            self.device_id
        )?;
        Ok(())
    }

    // --- Sessions ---

    async fn get_session(&self, address: &str) -> Result<Option<Bytes>> {
        let row = scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            "SELECT record FROM sessions WHERE address = $1 AND device_id = $2",
            address,
            self.device_id
        )?;
        Ok(row.map(Bytes::from))
    }

    async fn put_session(&self, address: &str, session: &[u8]) -> Result<()> {
        execute_sql!(
            &self.pool,
            "INSERT INTO sessions (address, record, device_id) VALUES ($1, $2, $3)
             ON CONFLICT (address, device_id) DO UPDATE SET record = EXCLUDED.record",
            address,
            session,
            self.device_id
        )?;
        Ok(())
    }

    async fn delete_session(&self, address: &str) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM sessions WHERE address = $1 AND device_id = $2",
            address,
            self.device_id
        )?;
        Ok(())
    }

    // --- PreKeys ---

    async fn store_prekey(&self, id: u32, record: &[u8], uploaded: bool) -> Result<()> {
        execute_sql!(
            &self.pool,
            "INSERT INTO prekeys (id, key, uploaded, device_id) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id, device_id) DO UPDATE
             SET key = EXCLUDED.key, uploaded = EXCLUDED.uploaded",
            id as i32,
            record,
            uploaded,
            self.device_id
        )?;
        Ok(())
    }

    /// UPDATE, never upsert. A prekey consumed (and deleted) between the upload
    /// snapshot and this call has to stay deleted; an upsert would resurrect it
    /// with an empty record and hand the server a key we can no longer answer.
    /// One of the few statements written per driver (`prekeys_sql`): Postgres
    /// binds an array, SQLite has no array bind.
    async fn mark_prekeys_uploaded(&self, ids: &[u32]) -> Result<()> {
        prekeys_sql::mark_uploaded(&self.pool, self.device_id, ids).await
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Bytes>> {
        let row = scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            "SELECT key FROM prekeys WHERE id = $1 AND device_id = $2",
            id as i32,
            self.device_id
        )?;
        Ok(row.map(Bytes::from))
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM prekeys WHERE id = $1 AND device_id = $2",
            id as i32,
            self.device_id
        )?;
        Ok(())
    }

    async fn get_max_prekey_id(&self) -> Result<u32> {
        let max = scalar_one_sql!(
            i32,
            &self.pool,
            "SELECT COALESCE(MAX(id), 0) FROM prekeys WHERE device_id = $1",
            self.device_id
        )?;
        Ok(max as u32)
    }

    // --- Signed PreKeys ---

    async fn store_signed_prekey(&self, id: u32, record: &[u8]) -> Result<()> {
        execute_sql!(
            &self.pool,
            "INSERT INTO signed_prekeys (id, record, device_id) VALUES ($1, $2, $3)
             ON CONFLICT (id, device_id) DO UPDATE SET record = EXCLUDED.record",
            id as i32,
            record,
            self.device_id
        )?;
        Ok(())
    }

    async fn load_signed_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            "SELECT record FROM signed_prekeys WHERE id = $1 AND device_id = $2",
            id as i32,
            self.device_id
        )
    }

    async fn load_all_signed_prekeys(&self) -> Result<Vec<(u32, Vec<u8>)>> {
        let rows = row_all_sql!(
            (i32, Vec<u8>),
            &self.pool,
            "SELECT id, record FROM signed_prekeys WHERE device_id = $1",
            self.device_id
        )?;
        Ok(rows.into_iter().map(|(id, rec)| (id as u32, rec)).collect())
    }

    async fn remove_signed_prekey(&self, id: u32) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM signed_prekeys WHERE id = $1 AND device_id = $2",
            id as i32,
            self.device_id
        )?;
        Ok(())
    }

    // --- Sender Keys ---

    async fn put_sender_key(&self, address: &str, record: &[u8]) -> Result<()> {
        execute_sql!(
            &self.pool,
            "INSERT INTO sender_keys (address, record, device_id) VALUES ($1, $2, $3)
             ON CONFLICT (address, device_id) DO UPDATE SET record = EXCLUDED.record",
            address,
            record,
            self.device_id
        )?;
        Ok(())
    }

    async fn get_sender_key(&self, address: &str) -> Result<Option<Vec<u8>>> {
        scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            "SELECT record FROM sender_keys WHERE address = $1 AND device_id = $2",
            address,
            self.device_id
        )
    }

    async fn delete_sender_key(&self, address: &str) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM sender_keys WHERE address = $1 AND device_id = $2",
            address,
            self.device_id
        )?;
        Ok(())
    }
}
