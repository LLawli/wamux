//! `Device` <-> `DeviceBlob`. The `Device` is the account's identity: its key
//! pairs, registration id and ADV secret are what WhatsApp knows this companion
//! by, so every field crosses explicitly and the key material is length-checked
//! on the way back in.

use std::sync::Arc;

use prost::Message as _;
use wacore::client_profile::ClientProfile;
use wacore::libsignal::protocol::{KeyPair, PrivateKey, PublicKey};
use wacore::store::Device;
use wacore::store::device::{
    CachedNoiseCert, CachedServerCertChain, DEVICE_PROPS, ServerClientExpiration, account_serde,
};
use wacore::store::error::StoreError;
use whatsapp_rust::{Jid, Server};

use super::wire::{DeviceBlob, JidWire, NoiseCert, ServerCertChain, ServerClientExpirationWire};
use super::{decode_error, fixed_bytes, wrong_length};

/// private (32) || public (32): wacore's `key_pair_serde` layout.
const KEY_PAIR_LEN: usize = 64;
const KEY_HALF_LEN: usize = 32;

/// `device.data`.
pub fn encode_device(device: &Device) -> Vec<u8> {
    DeviceBlob {
        pn: device.pn.as_ref().map(jid_to_wire),
        lid: device.lid.as_ref().map(jid_to_wire),
        registration_id: device.registration_id,
        noise_key: key_pair_to_bytes(&device.noise_key),
        identity_key: key_pair_to_bytes(&device.identity_key),
        signed_pre_key: key_pair_to_bytes(&device.signed_pre_key),
        signed_pre_key_id: device.signed_pre_key_id,
        signed_pre_key_signature: device.signed_pre_key_signature.to_vec(),
        adv_secret_key: device.adv_secret_key.to_vec(),
        account: device.account.as_deref().map(account_serde::to_bytes),
        push_name: device.push_name.clone(),
        app_version_primary: device.app_version_primary,
        app_version_secondary: device.app_version_secondary,
        app_version_tertiary: device.app_version_tertiary,
        app_version_last_fetched_ms: device.app_version_last_fetched_ms,
        edge_routing_info: device.edge_routing_info.clone(),
        props_hash: device.props_hash.clone(),
        next_pre_key_id: device.next_pre_key_id,
        first_unupload_pre_key_id: device.first_unupload_pre_key_id,
        server_has_prekeys: device.server_has_prekeys,
        nct_salt: device.nct_salt.clone(),
        server_cert_chain: device.server_cert_chain.as_ref().map(cert_chain_to_wire),
        login_counter: device.login_counter,
        lid_migrated: device.lid_migrated,
        last_signed_pre_key_rotation_ms: device.last_signed_pre_key_rotation_ms,
        server_client_expiration: device
            .server_client_expiration
            .as_ref()
            .map(expiration_to_wire),
        read_receipts_disabled: device.read_receipts_disabled,
    }
    .encode_to_vec()
}

/// The inverse of `encode_device`. The three runtime-only fields are not
/// stored: `device_props` is restored to `DEVICE_PROPS` (what the sqlite
/// reference does on load), `client_profile` to its default (`web()`, set again
/// before every connect) and `nct_salt_sync_seen` to false.
pub fn decode_device(bytes: &[u8]) -> Result<Device, StoreError> {
    let blob = DeviceBlob::decode(bytes).map_err(decode_error)?;
    Ok(Device {
        pn: blob.pn.map(jid_from_wire).transpose()?,
        lid: blob.lid.map(jid_from_wire).transpose()?,
        registration_id: blob.registration_id,
        noise_key: key_pair_from_bytes("device.noise_key", &blob.noise_key)?,
        identity_key: key_pair_from_bytes("device.identity_key", &blob.identity_key)?,
        signed_pre_key: key_pair_from_bytes("device.signed_pre_key", &blob.signed_pre_key)?,
        signed_pre_key_id: blob.signed_pre_key_id,
        signed_pre_key_signature: fixed_bytes(
            "device.signed_pre_key_signature",
            blob.signed_pre_key_signature,
        )?,
        adv_secret_key: fixed_bytes("device.adv_secret_key", blob.adv_secret_key)?,
        account: blob
            .account
            .as_deref()
            .map(account_from_bytes)
            .transpose()?,
        push_name: blob.push_name,
        app_version_primary: blob.app_version_primary,
        app_version_secondary: blob.app_version_secondary,
        app_version_tertiary: blob.app_version_tertiary,
        app_version_last_fetched_ms: blob.app_version_last_fetched_ms,
        device_props: Arc::new(DEVICE_PROPS.clone()),
        client_profile: ClientProfile::default(),
        edge_routing_info: blob.edge_routing_info,
        props_hash: blob.props_hash,
        next_pre_key_id: blob.next_pre_key_id,
        first_unupload_pre_key_id: blob.first_unupload_pre_key_id,
        server_has_prekeys: blob.server_has_prekeys,
        nct_salt: blob.nct_salt,
        nct_salt_sync_seen: false,
        server_cert_chain: blob
            .server_cert_chain
            .map(cert_chain_from_wire)
            .transpose()?,
        login_counter: blob.login_counter,
        lid_migrated: blob.lid_migrated,
        last_signed_pre_key_rotation_ms: blob.last_signed_pre_key_rotation_ms,
        server_client_expiration: blob.server_client_expiration.map(expiration_from_wire),
        read_receipts_disabled: blob.read_receipts_disabled,
    })
}

fn jid_to_wire(jid: &Jid) -> JidWire {
    JidWire {
        user: jid.user.to_string(),
        server: jid.server.as_str().to_string(),
        agent: u32::from(jid.agent),
        device: u32::from(jid.device),
        integrator: u32::from(jid.integrator),
    }
}

fn jid_from_wire(wire: JidWire) -> Result<Jid, StoreError> {
    let server = Server::try_from(wire.server.as_str())
        .map_err(|e| StoreError::Validation(format!("jid.server '{}': {e}", wire.server)))?;
    Ok(Jid {
        user: wire.user.into(),
        server,
        agent: narrow("jid.agent", wire.agent)?,
        device: narrow("jid.device", wire.device)?,
        integrator: narrow("jid.integrator", wire.integrator)?,
    })
}

/// A protobuf uint32 back into the narrower integer wacore holds.
fn narrow<T: TryFrom<u32>>(field: &str, value: u32) -> Result<T, StoreError> {
    T::try_from(value).map_err(|_| StoreError::Validation(format!("{field}: {value} out of range")))
}

fn key_pair_to_bytes(key_pair: &KeyPair) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(KEY_PAIR_LEN);
    bytes.extend_from_slice(key_pair.private_key.serialize().as_ref());
    bytes.extend_from_slice(key_pair.public_key.public_key_bytes());
    bytes
}

fn key_pair_from_bytes(field: &str, bytes: &[u8]) -> Result<KeyPair, StoreError> {
    if bytes.len() != KEY_PAIR_LEN {
        return Err(wrong_length(field, KEY_PAIR_LEN, bytes.len()));
    }
    let (private, public) = bytes.split_at(KEY_HALF_LEN);
    let private = PrivateKey::deserialize(private)
        .map_err(|e| StoreError::Validation(format!("{field} private half: {e}")))?;
    let public = PublicKey::from_djb_public_key_bytes(public)
        .map_err(|e| StoreError::Validation(format!("{field} public half: {e}")))?;
    Ok(KeyPair::new(public, private))
}

fn account_from_bytes(
    bytes: &[u8],
) -> Result<Arc<whatsapp_rust::waproto::whatsapp::ADVSignedDeviceIdentity>, StoreError> {
    account_serde::from_bytes(bytes)
        .map(Arc::new)
        .map_err(|e| StoreError::Serialization(Box::new(e)))
}

fn cert_to_wire(cert: &CachedNoiseCert) -> NoiseCert {
    NoiseCert {
        key: cert.key.to_vec(),
        not_before: cert.not_before,
        not_after: cert.not_after,
    }
}

fn cert_from_wire(field: &str, wire: Option<NoiseCert>) -> Result<CachedNoiseCert, StoreError> {
    let wire = wire.ok_or_else(|| StoreError::Validation(format!("{field}: missing")))?;
    Ok(CachedNoiseCert {
        key: fixed_bytes(field, wire.key)?,
        not_before: wire.not_before,
        not_after: wire.not_after,
    })
}

fn cert_chain_to_wire(chain: &CachedServerCertChain) -> ServerCertChain {
    ServerCertChain {
        intermediate: Some(cert_to_wire(&chain.intermediate)),
        leaf: Some(cert_to_wire(&chain.leaf)),
        signature_verified: chain.signature_verified,
    }
}

fn cert_chain_from_wire(wire: ServerCertChain) -> Result<CachedServerCertChain, StoreError> {
    Ok(CachedServerCertChain {
        intermediate: cert_from_wire("server_cert_chain.intermediate", wire.intermediate)?,
        leaf: cert_from_wire("server_cert_chain.leaf", wire.leaf)?,
        signature_verified: wire.signature_verified,
    })
}

fn expiration_to_wire(expiration: &ServerClientExpiration) -> ServerClientExpirationWire {
    let (primary, secondary, tertiary) = expiration.version;
    ServerClientExpirationWire {
        expires_at: expiration.expires_at,
        version_primary: primary,
        version_secondary: secondary,
        version_tertiary: tertiary,
    }
}

fn expiration_from_wire(wire: ServerClientExpirationWire) -> ServerClientExpiration {
    ServerClientExpiration {
        expires_at: wire.expires_at,
        version: (
            wire.version_primary,
            wire.version_secondary,
            wire.version_tertiary,
        ),
    }
}
