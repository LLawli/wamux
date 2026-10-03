//! What the fixture store holds: the values `0f40e34` wrote through the store
//! traits, before #65 unified the SQL. Every reader check compares against
//! these, so a value here is a claim about bytes already on disk in
//! `tests/fixtures/store-0f40e34/`. Never change one without regenerating the
//! fixture, and never regenerate it (see its README).

use std::collections::HashMap;
use std::sync::Arc;

use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::traits::{
    AppStateSyncKey, DeviceInfo, DeviceListRecord, LidPnMappingEntry, MsgSecretEntry, TcTokenEntry,
};

/// The one account in the fixture.
pub const ACCOUNT_REF: &str = "fixture-0f40e34";

pub const ADDRESS: &str = "5511900000001.0";
pub const IDENTITY: [u8; 32] = [0x11; 32];
pub const SESSION: &[u8] = &[0x22, 0x00, 0x22, 0xff, 0x22];

/// `(id, record, uploaded)`. One of each flag: `uploaded` has no getter and is
/// read below the trait.
pub const PREKEYS: [(u32, &[u8], bool); 2] = [(7, &[0x33; 24], true), (8, &[0x34; 24], false)];
pub const SIGNED_PREKEY: (u32, &[u8]) = (3, &[0x44; 50]);

pub const SENDER_KEY_ADDRESS: &str = "120363000000000001@g.us::5511900000001::0";
pub const SENDER_KEY: &[u8] = &[0x55; 30];

pub const SYNC_KEY_ID: &[u8] = &[0x00, 0x00, 0x00, 0x09];
pub const COLLECTION: &str = "regular_low";
pub const MAC_VERSION: u64 = 42;

pub const GROUP: &str = "120363000000000001@g.us";
pub const SENDER_KEY_DEVICES: [(&str, bool); 2] = [
    ("5511900000001:1@s.whatsapp.net", true),
    ("5511900000002:3@s.whatsapp.net", false),
];

pub const BASE_KEY: (&str, &str, &[u8]) = ("5511900000001.0", "M1", &[0xba, 0x00, 0x5e]);

pub const TC_TOKEN_JID: &str = "100000000000001@lid";

pub const SENT_CHAT: &str = "5511900000001@s.whatsapp.net";
pub const SENT_ID: &str = "M1";
pub const SENT_PAYLOAD: &[u8] = &[0x0a, 0x00, 0x01];

pub const DEVICE_LIST_USER: &str = "5511900000001";

pub fn sync_key() -> AppStateSyncKey {
    AppStateSyncKey {
        key_data: vec![0x66; 32],
        fingerprint: vec![0x01, 0x02, 0x03],
        timestamp: 1_749_400_000,
    }
}

/// Several map entries on purpose: the blob is deterministic only because the
/// codec encodes maps in key order, which the comparison relies on.
pub fn hash_state() -> HashState {
    let index_value_map: HashMap<String, Vec<u8>> = (0u8..3)
        .map(|i| (format!("index-{i}"), vec![i; 8]))
        .collect();
    HashState {
        version: MAC_VERSION,
        hash: [0x5a; 128],
        index_value_map,
        ..HashState::default()
    }
}

pub fn mutation_mac() -> AppStateMutationMAC {
    AppStateMutationMAC {
        index_mac: vec![0x1d; 32],
        value_mac: vec![0x7a; 32],
    }
}

pub fn lid_mapping() -> LidPnMappingEntry {
    LidPnMappingEntry {
        lid: "100000000000001".to_string(),
        phone_number: "5511900000001".to_string(),
        created_at: 1_749_400_000,
        updated_at: 1_749_400_100,
        learning_source: "usync".to_string(),
    }
}

pub fn device_list() -> DeviceListRecord {
    let devices = [
        DeviceInfo::new(0, None),
        DeviceInfo::new(4, Some(0)).with_hosting(true),
    ];
    DeviceListRecord {
        user: DEVICE_LIST_USER.into(),
        devices: devices.into(),
        timestamp: 1_749_400_000,
        phash: Some("2:abc".into()),
        raw_id: Some(5),
    }
}

pub fn tc_token() -> TcTokenEntry {
    TcTokenEntry {
        token: vec![0x7c, 0x00, 0xff],
        token_timestamp: 1_749_400_000,
        sender_timestamp: Some(1_749_400_100),
    }
}

pub fn msg_secret() -> MsgSecretEntry {
    MsgSecretEntry {
        chat: Arc::from(SENT_CHAT),
        sender: Arc::from("5511900000000@s.whatsapp.net"),
        msg_id: Arc::from(SENT_ID),
        secret: [0x5c; 32],
        expires_at: 0,
        message_ts: 1_749_400_000,
    }
}
