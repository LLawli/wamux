I have everything needed. Here is the complete verbatim answer.

---

## 1. The `Device` struct

**File:** `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/store/device.rs` (lines 196-253)

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub pn: Option<Jid>,
    pub lid: Option<Jid>,
    pub registration_id: u32,
    #[serde(with = "key_pair_serde")]
    pub noise_key: KeyPair,
    #[serde(with = "key_pair_serde")]
    pub identity_key: KeyPair,
    #[serde(with = "key_pair_serde")]
    pub signed_pre_key: KeyPair,
    pub signed_pre_key_id: u32,
    #[serde(with = "BigArray")]
    pub signed_pre_key_signature: [u8; 64],
    pub adv_secret_key: [u8; 32],
    #[serde(with = "account_serde", default)]
    pub account: Option<wa::AdvSignedDeviceIdentity>,
    pub push_name: String,
    pub app_version_primary: u32,
    pub app_version_secondary: u32,
    pub app_version_tertiary: u32,
    pub app_version_last_fetched_ms: i64,
    #[serde(skip)]
    pub device_props: wa::DeviceProps,
    /// Runtime-only. Set before `connect()` on every process start.
    #[serde(skip)]
    pub client_profile: ClientProfile,
    /// Edge routing info received from server, used for optimized reconnection.
    /// When present, this should be sent as a pre-intro before the Noise handshake.
    #[serde(default)]
    pub edge_routing_info: Option<Vec<u8>>,
    /// Hash from the last props (A/B experiment config) fetch.
    /// Sent on subsequent connects to enable delta updates instead of full fetches.
    #[serde(default)]
    pub props_hash: Option<String>,
    /// Monotonically increasing counter for one-time pre-key ID generation.
    /// Matches WhatsApp Web's `NEXT_PK_ID` pattern: only increases, never resets.
    /// Prevents prekey ID collisions when prekeys are consumed non-sequentially.
    #[serde(default)]
    pub next_pre_key_id: u32,
    /// Persisted flag matching WA Web's `signal_sever_has_pre_keys` metadata.
    #[serde(default)]
    pub server_has_prekeys: bool,
    /// NCT salt provisioned by the server via app state sync or history sync.
    #[serde(default)]
    pub nct_salt: Option<Vec<u8>>,
    /// Runtime-only marker that an authoritative nct_salt_sync mutation was seen.
    /// This prevents stale history sync data from resurrecting a cleared salt.
    #[serde(skip)]
    pub nct_salt_sync_seen: bool,
    /// Server cert chain cached from the last successful XX (or XX-fallback)
    /// handshake. Enables Noise IK on the next connect by exposing
    /// `leaf.key` as the server's static public key, and lets us reject
    /// stale entries via `not_after` before even attempting IK.
    /// `None` forces XX on the next connect.
    #[serde(default)]
    pub server_cert_chain: Option<CachedServerCertChain>,
}
```

Note: the imports that bind these names (same file, lines 1-8) are `use crate::libsignal::protocol::{IdentityKeyPair, KeyPair};`, `use serde_big_array::BigArray;`, `use wacore_binary::Jid;`, `use waproto::whatsapp as wa;`.

---

## 2. The `KeyPair` type and its 64-byte serialization

### `KeyPair` type definition

`Device` imports `KeyPair` from `crate::libsignal::protocol`, which is `wacore_libsignal` (`pub use wacore_libsignal as libsignal;` in `wacore-0.6.0/src/lib.rs:21`).

**File:** `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-libsignal-0.6.0/src/core/curve.rs` (lines 423-455)

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct KeyPair {
    pub public_key: PublicKey,
    pub private_key: PrivateKey,
}

impl KeyPair {
    pub fn generate<R: Rng + CryptoRng>(csprng: &mut R) -> Self {
        // Generate key WITHOUT computing Edwards cache (lazy initialization).
        // The Edwards point computation is deferred until first signature.
        let temp = curve25519::PrivateKey::new_without_cache(csprng);
        let key = temp.private_key_bytes();

        let public_key =
            PublicKey::from(PublicKeyData::DjbPublicKey(temp.derive_public_key_bytes()));
        // Edwards cache will be computed lazily on first signature
        let private_key = PrivateKey::from(PrivateKeyData::DjbPrivateKey {
            key,
            edwards_cache: OnceLock::new(),
        });

        Self {
            public_key,
            private_key,
        }
    }

    pub fn new(public_key: PublicKey, private_key: PrivateKey) -> Self {
        Self {
            public_key,
            private_key,
        }
    }
```

The underlying key types (same file):

```rust
#[derive(Clone, Copy, Eq, derive_more::From)]
pub struct PublicKey {
    key: PublicKeyData,
}
```
```rust
#[derive(Clone, Eq, PartialEq)]
pub struct PrivateKey {
    key: PrivateKeyData,
}
```

The two byte accessors used by the helper (same file):

```rust
    pub fn public_key_bytes(&self) -> &[u8] {
        match &self.key {
            PublicKeyData::DjbPublicKey(v) => v,
        }
    }
```
```rust
    pub fn from_djb_public_key_bytes(bytes: &[u8]) -> Result<Self, CurveError> {
        match <[u8; curve25519::PUBLIC_KEY_LENGTH]>::try_from(bytes) {
            Err(_) => Err(CurveError::BadKeyLength(KeyType::Djb, bytes.len())),
            Ok(key) => Ok(PublicKey {
                key: PublicKeyData::DjbPublicKey(key),
            }),
        }
    }
```
```rust
    pub fn serialize(&self) -> &[u8; 32] {  // PrivateKey::serialize — returns the raw 32-byte scalar
        match &self.key {
            PrivateKeyData::DjbPrivateKey { key, .. } => key,
        }
    }
```

Important detail: `KeyPair` itself derives `serde::Serialize, serde::Deserialize`, but the `Device` struct does NOT use that derive. Every `KeyPair` field in `Device` is annotated `#[serde(with = "key_pair_serde")]`, which overrides the derived impl with the 64-byte concat below.

### The 64-byte (priv 32 + pub 32) concat serde helper

**File:** `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/store/device.rs` (lines 44-79)

```rust
pub mod key_pair_serde {
    use super::KeyPair;
    use crate::libsignal::protocol::{PrivateKey, PublicKey};
    use serde::{self, Deserializer, Serializer};

    pub fn serialize<S>(key_pair: &KeyPair, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let bytes: Vec<u8> = key_pair
            .private_key
            .serialize()
            .iter()
            .copied()
            .chain(key_pair.public_key.public_key_bytes().iter().copied())
            .collect();
        serializer.serialize_bytes(&bytes)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<KeyPair, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: Vec<u8> = serde::Deserialize::deserialize(deserializer)?;
        if bytes.len() != 64 {
            return Err(serde::de::Error::invalid_length(bytes.len(), &"64"));
        }
        // reason: serde::de::Error::custom flattens to a String at the boundary —
        // serde's error model has no source-chain preservation.
        let private_key = PrivateKey::deserialize(&bytes[0..32])
            .map_err(|e| serde::de::Error::custom(e.to_string()))?;
        let public_key = PublicKey::from_djb_public_key_bytes(&bytes[32..64])
            .map_err(|e| serde::de::Error::custom(e.to_string()))?;
        Ok(KeyPair::new(public_key, private_key))
    }
}
```

Layout: bytes `[0..32]` = private key (raw 32-byte X25519 scalar, from `PrivateKey::serialize()`), bytes `[32..64]` = public key (raw 32-byte DJB public key bytes, from `PublicKey::public_key_bytes()`, i.e. WITHOUT the leading `0x05` type byte that `PublicKey::serialize()` would prepend).

---

## 3. Persistence helpers / From-Into for the other Device fields

### `account: Option<AdvSignedDeviceIdentity>` — `account_serde` (protobuf-bytes via prost)

**File:** `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/store/device.rs` (lines 10-42)

```rust
/// Protobuf-bytes serde for `AdvSignedDeviceIdentity` (prost types lack `Deserialize`).
pub mod account_serde {
    use prost::Message;
    use waproto::whatsapp as wa;

    pub fn to_bytes(account: &wa::AdvSignedDeviceIdentity) -> Vec<u8> {
        account.encode_to_vec()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<wa::AdvSignedDeviceIdentity, prost::DecodeError> {
        wa::AdvSignedDeviceIdentity::decode(bytes)
    }

    pub fn serialize<S: serde::Serializer>(
        val: &Option<wa::AdvSignedDeviceIdentity>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match val {
            Some(v) => s.serialize_some(&to_bytes(v)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<Option<wa::AdvSignedDeviceIdentity>, D::Error> {
        let bytes: Option<Vec<u8>> = serde::Deserialize::deserialize(d)?;
        match bytes {
            Some(b) => from_bytes(&b).map(Some).map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}
```

### `signed_pre_key_signature: [u8; 64]` — serialized via `serde_big_array::BigArray`

`#[serde(with = "BigArray")]` (no custom helper; the `serde_big_array` crate handles the 64-byte fixed array).

### `server_cert_chain: Option<CachedServerCertChain>` — plain derive + `From`

These types derive `Serialize/Deserialize` directly (no byte helper). Verbatim definitions and the conversion used to populate it from a verified Noise handshake.

**File:** `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/store/device.rs` (lines 255-291)

```rust
/// Minimal cached form of a Noise certificate. Mirrors the JSON shape WA Web
/// persists in `waNoiseInfo.certificateChainBuffer` (only `key` plus the
/// validity window — signatures and issuer_serial are intentionally dropped).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedNoiseCert {
    /// 32-byte X25519 public key from `NoiseCertificate.Details.key`.
    pub key: [u8; 32],
    /// Unix epoch seconds. Validation window from `NoiseCertificate.Details`.
    pub not_before: i64,
    pub not_after: i64,
}

/// Cached form of the server's two-cert chain. `leaf.key` is the server
/// static public key consumed by Noise IK; the intermediate is kept solely
/// to mirror WA Web's expiry checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedServerCertChain {
    pub intermediate: CachedNoiseCert,
    pub leaf: CachedNoiseCert,
}

impl From<wacore_noise::VerifiedServerCertChain> for CachedServerCertChain {
    fn from(v: wacore_noise::VerifiedServerCertChain) -> Self {
        Self {
            intermediate: CachedNoiseCert {
                key: v.intermediate_key,
                not_before: v.intermediate_not_before,
                not_after: v.intermediate_not_after,
            },
            leaf: CachedNoiseCert {
                key: v.leaf_key,
                not_before: v.leaf_not_before,
                not_after: v.leaf_not_after,
            },
        }
    }
}
```

### Fields persisted with no custom helper (plain serde derive)

`edge_routing_info: Option<Vec<u8>>`, `nct_salt: Option<Vec<u8>>`, `props_hash: Option<String>`, `next_pre_key_id: u32`, `server_has_prekeys: bool`, `adv_secret_key: [u8; 32]`, the `pn`/`lid` (`Option<Jid>`), and the `u32`/`i64`/`String` scalars all serialize via the standard derived impl, each guarded by `#[serde(default)]` (for backward-compat with older records) where annotated above. The runtime-only fields `device_props`, `client_profile`, and `nct_salt_sync_seen` are `#[serde(skip)]` and are NOT persisted.

### Summary of what each persisted field uses

| Field | Serialization mechanism |
|---|---|
| `noise_key`, `identity_key`, `signed_pre_key` (`KeyPair`) | `key_pair_serde` — 32-byte priv + 32-byte pub concat = 64 bytes, via `serialize_bytes` |
| `signed_pre_key_signature: [u8; 64]` | `serde_big_array::BigArray` |
| `account: Option<AdvSignedDeviceIdentity>` | `account_serde` — prost `encode_to_vec()` / `decode()`, wrapped in `Option<Vec<u8>>` |
| `server_cert_chain: Option<CachedServerCertChain>` | plain derive; populated from `wacore_noise::VerifiedServerCertChain` via `From` |
| `adv_secret_key: [u8;32]`, `edge_routing_info`, `nct_salt`, `props_hash`, scalars, `pn`/`lid` | plain serde derive (`#[serde(default)]` where shown) |
| `device_props`, `client_profile`, `nct_salt_sync_seen` | `#[serde(skip)]` — not persisted |

There is no `DeviceStore::save`/`load` named method in this crate's `device.rs`; persistence is driven entirely through serde (e.g. the tests use `serde_json::to_string`/`from_str` round-trips on the whole `Device`). The `DeviceStore` trait itself is not in `wacore-0.6.0/src/store/device.rs` — only the `Device` model and its serde helpers shown above.