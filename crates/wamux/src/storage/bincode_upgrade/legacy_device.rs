//! The `wacore::store::Device` bincode layout of pin 23846f7e, frozen (#86).
//!
//! Bincode is positional: field order and types are the format. whatsapp-rust
//! #1565 inserted `status_privacy` in the middle of `Device`, so a blob written
//! before it no longer reads as the current `Device`. No legacy store has that
//! field, so this mirror lists exactly what 23846f7e serialized, in order, with
//! the same `serde(with)` helpers (still public and unchanged in 6f07e3a).
//! `skip` fields are not part of the layout and are absent here.

use std::sync::Arc;

use serde::Deserialize;
use wacore::client_profile::ClientProfile;
use wacore::libsignal::protocol::KeyPair;
use wacore::store::Device;
use wacore::store::device::{
    CachedServerCertChain, DEVICE_PROPS, ServerClientExpiration, account_serde, key_pair_serde,
};
use whatsapp_rust::Jid;

#[derive(Deserialize)]
pub(super) struct LegacyDevice {
    pn: Option<Jid>,
    lid: Option<Jid>,
    registration_id: u32,
    #[serde(with = "key_pair_serde")]
    noise_key: KeyPair,
    #[serde(with = "key_pair_serde")]
    identity_key: KeyPair,
    #[serde(with = "key_pair_serde")]
    signed_pre_key: KeyPair,
    signed_pre_key_id: u32,
    /// `serde_big_array` writes a 64-byte array as a bare 64-tuple, with no
    /// length prefix. Two 32-tuples are byte-identical and need no extra crate.
    signed_pre_key_signature: [[u8; 32]; 2],
    adv_secret_key: [u8; 32],
    #[serde(with = "account_serde", default)]
    account: Option<Arc<whatsapp_rust::waproto::whatsapp::ADVSignedDeviceIdentity>>,
    push_name: String,
    app_version_primary: u32,
    app_version_secondary: u32,
    app_version_tertiary: u32,
    app_version_last_fetched_ms: i64,
    #[serde(default)]
    edge_routing_info: Option<Vec<u8>>,
    #[serde(default)]
    props_hash: Option<String>,
    #[serde(default)]
    next_pre_key_id: u32,
    #[serde(default)]
    first_unupload_pre_key_id: u32,
    #[serde(default)]
    server_has_prekeys: bool,
    #[serde(default)]
    nct_salt: Option<Vec<u8>>,
    #[serde(default)]
    server_cert_chain: Option<CachedServerCertChain>,
    #[serde(default)]
    login_counter: i32,
    #[serde(default)]
    lid_migrated: bool,
    #[serde(default)]
    last_signed_pre_key_rotation_ms: i64,
    #[serde(default)]
    server_client_expiration: Option<ServerClientExpiration>,
    #[serde(default)]
    read_receipts_disabled: bool,
}

/// Runtime-only fields come back as `decode_device` restores them.
impl From<LegacyDevice> for Device {
    fn from(old: LegacyDevice) -> Device {
        let [sig_head, sig_tail] = old.signed_pre_key_signature;
        let mut signed_pre_key_signature = [0u8; 64];
        signed_pre_key_signature[..32].copy_from_slice(&sig_head);
        signed_pre_key_signature[32..].copy_from_slice(&sig_tail);
        Device {
            pn: old.pn,
            lid: old.lid,
            registration_id: old.registration_id,
            noise_key: old.noise_key,
            identity_key: old.identity_key,
            signed_pre_key: old.signed_pre_key,
            signed_pre_key_id: old.signed_pre_key_id,
            signed_pre_key_signature,
            adv_secret_key: old.adv_secret_key,
            account: old.account,
            push_name: old.push_name,
            app_version_primary: old.app_version_primary,
            app_version_secondary: old.app_version_secondary,
            app_version_tertiary: old.app_version_tertiary,
            app_version_last_fetched_ms: old.app_version_last_fetched_ms,
            device_props: Arc::new(DEVICE_PROPS.clone()),
            client_profile: ClientProfile::default(),
            edge_routing_info: old.edge_routing_info,
            props_hash: old.props_hash,
            next_pre_key_id: old.next_pre_key_id,
            first_unupload_pre_key_id: old.first_unupload_pre_key_id,
            server_has_prekeys: old.server_has_prekeys,
            nct_salt: old.nct_salt,
            nct_salt_sync_seen: false,
            server_cert_chain: old.server_cert_chain,
            login_counter: old.login_counter,
            lid_migrated: old.lid_migrated,
            last_signed_pre_key_rotation_ms: old.last_signed_pre_key_rotation_ms,
            server_client_expiration: old.server_client_expiration,
            read_receipts_disabled: old.read_receipts_disabled,
            status_privacy: None,
        }
    }
}
