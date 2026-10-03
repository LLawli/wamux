//! `ProtocolStore`, first half: sender-key device tracking, LID-PN mappings,
//! base keys and the device registry. Tc tokens and the sent-message cache are
//! in `protocol_cache`.

use wacore::store::traits::{DeviceInfo, DeviceListRecord, LidPnMappingEntry};

use crate::harness::{self, Harness};

const GROUP_1: &str = "120363000000000001@g.us";
const GROUP_2: &str = "120363000000000002@g.us";
const DEV_X: &str = "5511900000001:1@s.whatsapp.net";
const DEV_Y: &str = "5511900000002:3@s.whatsapp.net";

async fn sorted_devices(
    backend: &dyn wacore::store::traits::Backend,
    group: &str,
) -> Vec<(String, bool)> {
    let mut rows = backend.get_sender_key_devices(group).await.unwrap();
    rows.sort();
    rows
}

async fn sender_key_devices_status_round_trip_and_flip(h: Harness) {
    let t = h.two_accounts("skd-status").await;
    assert!(
        t.ba.get_sender_key_devices(GROUP_1)
            .await
            .unwrap()
            .is_empty()
    );

    t.ba.set_sender_key_status(GROUP_1, &[(DEV_X, true), (DEV_Y, false)])
        .await
        .unwrap();
    t.ba.set_sender_key_status(GROUP_1, &[])
        .await
        .expect("empty is a no-op");
    assert_eq!(
        sorted_devices(&*t.ba, GROUP_1).await,
        vec![(DEV_X.to_string(), true), (DEV_Y.to_string(), false)]
    );

    // WA Web's markForgetSenderKey / markHasSenderKey: the same row flips.
    t.ba.set_sender_key_status(GROUP_1, &[(DEV_X, false), (DEV_Y, true)])
        .await
        .unwrap();
    assert_eq!(
        sorted_devices(&*t.ba, GROUP_1).await,
        vec![(DEV_X.to_string(), false), (DEV_Y.to_string(), true)]
    );
    assert!(
        t.ba.get_sender_key_devices(GROUP_2)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        t.bb.get_sender_key_devices(GROUP_1)
            .await
            .unwrap()
            .is_empty()
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_sender_key_devices_status_round_trip_and_flip() {
    sender_key_devices_status_round_trip_and_flip(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_sender_key_devices_status_round_trip_and_flip() {
    sender_key_devices_status_round_trip_and_flip(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_sender_key_devices_status_round_trip_and_flip() {
    sender_key_devices_status_round_trip_and_flip(harness::turso().await).await;
}

/// Three clears with three different scopes: one group, one device JID across
/// every group, and every group. All three stop at the account boundary.
async fn sender_key_device_clears_are_scoped(h: Harness) {
    let t = h.two_accounts("skd-clears").await;
    for backend in [&t.ba, &t.bb] {
        for group in [GROUP_1, GROUP_2] {
            backend
                .set_sender_key_status(group, &[(DEV_X, true), (DEV_Y, true)])
                .await
                .unwrap();
        }
    }
    let both = vec![(DEV_X.to_string(), true), (DEV_Y.to_string(), true)];
    let only_y = vec![(DEV_Y.to_string(), true)];

    t.ba.clear_sender_key_devices(GROUP_1).await.unwrap();
    assert!(
        t.ba.get_sender_key_devices(GROUP_1)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(sorted_devices(&*t.ba, GROUP_2).await, both);

    t.ba.set_sender_key_status(GROUP_1, &[(DEV_X, true), (DEV_Y, true)])
        .await
        .unwrap();
    t.ba.delete_sender_key_device_rows(&[DEV_X]).await.unwrap();
    t.ba.delete_sender_key_device_rows(&[])
        .await
        .expect("empty is a no-op");
    assert_eq!(sorted_devices(&*t.ba, GROUP_1).await, only_y);
    assert_eq!(sorted_devices(&*t.ba, GROUP_2).await, only_y);

    t.ba.clear_all_sender_key_devices().await.unwrap();
    assert!(
        t.ba.get_sender_key_devices(GROUP_1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        t.ba.get_sender_key_devices(GROUP_2)
            .await
            .unwrap()
            .is_empty()
    );

    for group in [GROUP_1, GROUP_2] {
        assert_eq!(
            sorted_devices(&*t.bb, group).await,
            both,
            "no clear on A may reach B"
        );
    }
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_sender_key_device_clears_are_scoped() {
    sender_key_device_clears_are_scoped(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_sender_key_device_clears_are_scoped() {
    sender_key_device_clears_are_scoped(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_sender_key_device_clears_are_scoped() {
    sender_key_device_clears_are_scoped(harness::turso().await).await;
}

fn mapping(
    lid: &str,
    phone: &str,
    created_at: i64,
    updated_at: i64,
    source: &str,
) -> LidPnMappingEntry {
    LidPnMappingEntry {
        lid: lid.to_string(),
        phone_number: phone.to_string(),
        created_at,
        updated_at,
        learning_source: source.to_string(),
    }
}

/// `LidPnMappingEntry` has no `PartialEq`; compare every field as a tuple.
fn fields(e: &LidPnMappingEntry) -> (String, String, i64, i64, String) {
    (
        e.lid.clone(),
        e.phone_number.clone(),
        e.created_at,
        e.updated_at,
        e.learning_source.clone(),
    )
}

async fn lid_mappings_round_trip_and_pick_most_recent_pn(h: Harness) {
    const PHONE: &str = "5511987654321";
    let t = h.two_accounts("lid-mappings").await;
    assert!(
        t.ba.get_lid_mapping("100000000000001")
            .await
            .unwrap()
            .is_none()
    );
    assert!(t.ba.get_pn_mapping(PHONE).await.unwrap().is_none());

    t.ba.put_lid_mapping(&mapping("100000000000001", PHONE, 10, 20, "usync"))
        .await
        .unwrap();
    t.ba.put_lid_mapping(&mapping(
        "100000000000002",
        PHONE,
        30,
        40,
        "peer_pn_message",
    ))
    .await
    .unwrap();
    t.bb.put_lid_mapping(&mapping("100000000000003", PHONE, 50, 90, "usync"))
        .await
        .unwrap();

    let by_pn = t.ba.get_pn_mapping(PHONE).await.unwrap().expect("mapped");
    assert_eq!(
        by_pn.lid, "100000000000002",
        "the most recent updated_at wins"
    );

    // Upsert: phone, source and updated_at move; created_at is the first sighting.
    t.ba.put_lid_mapping(&mapping("100000000000001", PHONE, 999, 60, "usync-again"))
        .await
        .unwrap();
    let by_lid =
        t.ba.get_lid_mapping("100000000000001")
            .await
            .unwrap()
            .expect("mapped");
    assert_eq!(
        fields(&by_lid),
        fields(&mapping("100000000000001", PHONE, 10, 60, "usync-again"))
    );
    let by_pn = t.ba.get_pn_mapping(PHONE).await.unwrap().expect("mapped");
    assert_eq!(by_pn.lid, "100000000000001", "now the more recent one");

    let mut all: Vec<_> =
        t.ba.get_all_lid_mappings()
            .await
            .unwrap()
            .iter()
            .map(fields)
            .collect();
    all.sort();
    assert_eq!(
        all,
        vec![
            fields(&mapping("100000000000001", PHONE, 10, 60, "usync-again")),
            fields(&mapping(
                "100000000000002",
                PHONE,
                30,
                40,
                "peer_pn_message"
            )),
        ],
        "A lists only its own mappings"
    );
    assert!(
        t.bb.get_lid_mapping("100000000000001")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(t.bb.get_all_lid_mappings().await.unwrap().len(), 1);
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_lid_mappings_round_trip_and_pick_most_recent_pn() {
    lid_mappings_round_trip_and_pick_most_recent_pn(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_lid_mappings_round_trip_and_pick_most_recent_pn() {
    lid_mappings_round_trip_and_pick_most_recent_pn(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_lid_mappings_round_trip_and_pick_most_recent_pn() {
    lid_mappings_round_trip_and_pick_most_recent_pn(harness::turso().await).await;
}

async fn base_keys_detect_same_key_and_delete(h: Harness) {
    const ADDR: &str = "alice@s.whatsapp.net.0";
    let t = h.two_accounts("base-keys").await;
    assert!(
        !t.ba.has_same_base_key(ADDR, "M1", b"k1").await.unwrap(),
        "no saved key is not a collision"
    );

    t.ba.save_base_key(ADDR, "M1", b"k1").await.unwrap();
    t.bb.save_base_key(ADDR, "M1", b"kb").await.unwrap();
    assert!(t.ba.has_same_base_key(ADDR, "M1", b"k1").await.unwrap());
    assert!(!t.ba.has_same_base_key(ADDR, "M1", b"k2").await.unwrap());
    assert!(!t.ba.has_same_base_key(ADDR, "M2", b"k1").await.unwrap());
    assert!(!t.bb.has_same_base_key(ADDR, "M1", b"k1").await.unwrap());

    t.ba.save_base_key(ADDR, "M1", b"k2").await.unwrap();
    assert!(
        t.ba.has_same_base_key(ADDR, "M1", b"k2").await.unwrap(),
        "upsert"
    );
    assert!(!t.ba.has_same_base_key(ADDR, "M1", b"k1").await.unwrap());

    t.ba.delete_base_key(ADDR, "M1").await.unwrap();
    assert!(!t.ba.has_same_base_key(ADDR, "M1", b"k2").await.unwrap());
    assert!(
        t.bb.has_same_base_key(ADDR, "M1", b"kb").await.unwrap(),
        "a delete on A must not reach B"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_base_keys_detect_same_key_and_delete() {
    base_keys_detect_same_key_and_delete(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_base_keys_detect_same_key_and_delete() {
    base_keys_detect_same_key_and_delete(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_base_keys_detect_same_key_and_delete() {
    base_keys_detect_same_key_and_delete(harness::turso().await).await;
}

fn device_list(user: &str, timestamp: i64) -> DeviceListRecord {
    DeviceListRecord {
        user: user.into(),
        devices: vec![DeviceInfo::new(0, None), DeviceInfo::new(2, Some(1))].into(),
        timestamp,
        phash: None,
        raw_id: None,
    }
}

/// Field-by-field round-trip is `device_registry_round_trips_every_field` in
/// `storage_backend.rs`; this pins overwrite, delete and isolation.
async fn device_registry_delete_is_scoped(h: Harness) {
    const USER: &str = "5511900000009";
    let t = h.two_accounts("device-registry").await;
    assert!(t.ba.get_devices(USER).await.unwrap().is_none());

    t.ba.update_device_list(device_list(USER, 100))
        .await
        .unwrap();
    t.ba.update_device_list(device_list(USER, 200))
        .await
        .unwrap();
    t.bb.update_device_list(device_list(USER, 300))
        .await
        .unwrap();
    let back = t.ba.get_devices(USER).await.unwrap().expect("record");
    assert_eq!(back.timestamp, 200, "a second update overwrites");
    assert_eq!(back.phash, None);
    assert_eq!(back.raw_id, None);

    t.ba.delete_devices(USER).await.unwrap();
    assert!(t.ba.get_devices(USER).await.unwrap().is_none());
    t.ba.delete_devices(USER)
        .await
        .expect("deleting an absent record is a no-op");
    assert_eq!(
        t.bb.get_devices(USER).await.unwrap().map(|r| r.timestamp),
        Some(300),
        "a delete on A must not reach B"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_device_registry_delete_is_scoped() {
    device_registry_delete_is_scoped(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_device_registry_delete_is_scoped() {
    device_registry_delete_is_scoped(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_device_registry_delete_is_scoped() {
    device_registry_delete_is_scoped(harness::turso().await).await;
}
