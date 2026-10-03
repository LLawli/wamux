//! `AppSyncStore` and `ProtocolStore` batch methods (#104), with 250 items so
//! a batched read crosses three 100-value chunks and pads the last one.

use std::collections::HashMap;

use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::traits::{DeviceInfo, DeviceListRecord, LidPnMappingEntry, TcTokenEntry};

use crate::harness::{self, Harness};

const MANY: usize = 250;
const LOW: &str = "regular_low";
const HIGH: &str = "regular_high";

fn index_mac(i: usize) -> [u8; 32] {
    let mut mac = [0x1d; 32];
    mac[..8].copy_from_slice(&(i as u64).to_be_bytes());
    mac
}

fn mac(i: usize, value: u8) -> AppStateMutationMAC {
    AppStateMutationMAC {
        index_mac: index_mac(i).to_vec(),
        value_mac: vec![value; 32],
    }
}

async fn mutation_macs_batch_reads_only_the_asked_collection(h: Harness) {
    let t = h.two_accounts("batch-macs").await;
    let macs: Vec<AppStateMutationMAC> = (0..MANY).map(|i| mac(i, 0x7a)).collect();
    t.ba.put_mutation_macs(LOW, 1, &macs).await.unwrap();
    t.ba.put_mutation_macs(HIGH, 1, &[mac(MANY, 0x55)])
        .await
        .unwrap();
    t.bb.put_mutation_macs(LOW, 1, &[mac(MANY + 1, 0x66)])
        .await
        .unwrap();

    let mut asked: Vec<[u8; 32]> = (0..=MANY + 1).map(index_mac).collect();
    asked.push(index_mac(0));
    let got = t.ba.get_mutation_macs(LOW, &asked).await.unwrap();
    let want: HashMap<[u8; 32], Vec<u8>> =
        (0..MANY).map(|i| (index_mac(i), vec![0x7a; 32])).collect();
    assert_eq!(
        got, want,
        "other collection, other account and absent MACs are left out"
    );
    assert!(t.ba.get_mutation_macs(LOW, &[]).await.unwrap().is_empty());
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_mutation_macs_batch_reads_only_the_asked_collection() {
    mutation_macs_batch_reads_only_the_asked_collection(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_mutation_macs_batch_reads_only_the_asked_collection() {
    mutation_macs_batch_reads_only_the_asked_collection(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_mutation_macs_batch_reads_only_the_asked_collection() {
    mutation_macs_batch_reads_only_the_asked_collection(harness::turso().await).await;
}

fn hash_state(version: u64) -> HashState {
    HashState {
        version,
        hash: [version as u8; 128],
        ..HashState::default()
    }
}

async fn commit_patch_round_trips(h: Harness) {
    let t = h.two_accounts("commit-patch").await;
    t.ba.commit_patch(LOW, hash_state(1), &[], &[mac(1, 1), mac(2, 2)])
        .await
        .unwrap();
    t.ba.commit_patch(LOW, hash_state(2), &[index_mac(1).to_vec()], &[mac(3, 3)])
        .await
        .unwrap();

    assert_eq!(
        t.ba.get_version(LOW).await.unwrap().map(|s| s.version),
        Some(2)
    );
    assert_eq!(
        t.ba.get_mutation_mac(LOW, &index_mac(1)).await.unwrap(),
        None
    );
    assert_eq!(
        t.ba.get_mutation_mac(LOW, &index_mac(2)).await.unwrap(),
        Some(vec![2; 32])
    );
    assert_eq!(
        t.ba.get_mutation_mac(LOW, &index_mac(3)).await.unwrap(),
        Some(vec![3; 32])
    );
    assert!(t.bb.get_version(LOW).await.unwrap().is_none());
    assert_eq!(
        t.bb.get_mutation_mac(LOW, &index_mac(2)).await.unwrap(),
        None
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_commit_patch_round_trips() {
    commit_patch_round_trips(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_commit_patch_round_trips() {
    commit_patch_round_trips(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_commit_patch_round_trips() {
    commit_patch_round_trips(harness::turso().await).await;
}

fn lid_entry(i: usize, source: &str) -> LidPnMappingEntry {
    LidPnMappingEntry {
        lid: format!("1000000000{i:05}"),
        phone_number: format!("55119{i:08}"),
        created_at: 1_749_400_000,
        updated_at: 1_749_400_000 + i as i64,
        learning_source: source.to_string(),
    }
}

async fn lid_mappings_batch_round_trips(h: Harness) {
    let t = h.two_accounts("batch-lid").await;
    let mut batch: Vec<LidPnMappingEntry> = (0..MANY).map(|i| lid_entry(i, "usync")).collect();
    batch.push(lid_entry(0, "last"));
    t.ba.put_lid_mappings(&batch).await.unwrap();
    t.ba.put_lid_mappings(&[]).await.expect("empty is a no-op");

    assert_eq!(t.ba.get_all_lid_mappings().await.unwrap().len(), MANY);
    let first =
        t.ba.get_lid_mapping(&lid_entry(0, "").lid)
            .await
            .unwrap()
            .expect("mapped");
    assert_eq!(first.learning_source, "last", "last wins");
    let by_pn =
        t.ba.get_pn_mapping(&lid_entry(7, "").phone_number)
            .await
            .unwrap();
    assert_eq!(by_pn.map(|e| e.lid), Some(lid_entry(7, "").lid));
    assert!(t.bb.get_all_lid_mappings().await.unwrap().is_empty());
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_lid_mappings_batch_round_trips() {
    lid_mappings_batch_round_trips(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_lid_mappings_batch_round_trips() {
    lid_mappings_batch_round_trips(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_lid_mappings_batch_round_trips() {
    lid_mappings_batch_round_trips(harness::turso().await).await;
}

fn device_list(i: usize, timestamp: i64) -> DeviceListRecord {
    DeviceListRecord {
        user: format!("55119{i:08}").into(),
        devices: vec![DeviceInfo::new(0, None), DeviceInfo::new(2, Some(1))].into(),
        timestamp,
        phash: Some(format!("2:{i}").into()),
        raw_id: Some(i as u32),
    }
}

async fn device_lists_batch_round_trips(h: Harness) {
    let t = h.two_accounts("batch-devices").await;
    let mut batch: Vec<DeviceListRecord> = (0..MANY).map(|i| device_list(i, 1)).collect();
    batch.push(device_list(0, 99));
    t.ba.update_device_lists(batch).await.unwrap();
    t.ba.update_device_lists(Vec::new())
        .await
        .expect("empty is a no-op");

    let users: Vec<String> = (0..MANY)
        .map(|i| device_list(i, 0).user.to_string())
        .collect();
    let mut asked: Vec<&str> = users.iter().map(String::as_str).collect();
    asked.push("absent");
    let mut got = t.ba.get_devices_batch(&asked).await.unwrap();
    got.sort_by(|a, b| a.user.cmp(&b.user));
    assert_eq!(got.len(), MANY, "absent users are left out");
    assert_eq!(got[0].timestamp, 99, "last wins");
    assert_eq!(&*got[42].devices, &*device_list(42, 1).devices);
    assert_eq!(got[42].phash.as_deref(), Some("2:42"));
    assert_eq!(got[42].raw_id, Some(42));
    assert!(t.bb.get_devices_batch(&asked).await.unwrap().is_empty());
    assert!(t.ba.get_devices_batch(&[]).await.unwrap().is_empty());
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_device_lists_batch_round_trips() {
    device_lists_batch_round_trips(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_device_lists_batch_round_trips() {
    device_lists_batch_round_trips(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_device_lists_batch_round_trips() {
    device_lists_batch_round_trips(harness::turso().await).await;
}

fn jid(i: usize) -> String {
    format!("1000000000{i:05}@lid")
}

async fn tc_tokens_batch_keeps_the_asked_order(h: Harness) {
    let t = h.two_accounts("batch-tc").await;
    for i in (0..MANY).step_by(2) {
        let entry = TcTokenEntry {
            token: vec![i as u8; 3],
            token_timestamp: 1_749_400_000 + i as i64,
            sender_timestamp: None,
        };
        t.ba.put_tc_token(&jid(i), &entry).await.unwrap();
    }
    let mut asked: Vec<String> = (0..MANY).rev().map(jid).collect();
    asked.push(jid(0));
    let got = t.ba.get_tc_tokens(&asked).await.unwrap();
    assert_eq!(
        got.len(),
        asked.len(),
        "one answer per asked jid, repeats included"
    );
    for (jid_asked, answer) in asked.iter().zip(&got) {
        let i: usize = jid_asked[10..15].parse().unwrap();
        match answer {
            Some(entry) => assert_eq!(
                (i % 2, entry.token_timestamp),
                (0, 1_749_400_000 + i as i64)
            ),
            None => assert_eq!(i % 2, 1, "{jid_asked} has a row"),
        }
    }
    assert!(
        t.bb.get_tc_tokens(&asked)
            .await
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
    assert!(t.ba.get_tc_tokens(&[]).await.unwrap().is_empty());
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_tc_tokens_batch_keeps_the_asked_order() {
    tc_tokens_batch_keeps_the_asked_order(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_tc_tokens_batch_keeps_the_asked_order() {
    tc_tokens_batch_keeps_the_asked_order(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_tc_tokens_batch_keeps_the_asked_order() {
    tc_tokens_batch_keeps_the_asked_order(harness::turso().await).await;
}
