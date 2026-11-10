//! `SignalStore::get_signal_addresses_for_user`: every address one user still
//! holds state under, across sessions, identities and sender-key device
//! tracking. The library calls it when a contact's verified identity changes,
//! to clean up devices the registry row no longer lists; the trait default
//! fails, and with it the contact's device-list resolution.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::store_parity::harness::{self, Harness};

const USER: &str = "5511900000001";
const GROUP_1: &str = "120363000000000001@g.us";
const GROUP_2: &str = "120363000000000002@g.us";

async fn addresses(backend: &dyn wacore::store::traits::Backend, user: &str) -> BTreeSet<String> {
    let found: Vec<Arc<str>> = backend.get_signal_addresses_for_user(user).await.unwrap();
    let unique: BTreeSet<String> = found.iter().map(|a| a.to_string()).collect();
    assert_eq!(unique.len(), found.len(), "no address twice: {found:?}");
    unique
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

async fn collects_all_three_sources_once_per_account(h: Harness) {
    let t = h.two_accounts("sig-addr-sources").await;
    assert!(addresses(&*t.ba, USER).await.is_empty());

    t.ba.put_session("5511900000001@s.whatsapp.net.0", b"s0")
        .await
        .unwrap();
    t.ba.put_identity("5511900000001:7@lid.0", [1; 32])
        .await
        .unwrap();
    // The same address in two sources and in two groups is still one entry.
    t.ba.put_identity("5511900000001@s.whatsapp.net.0", [2; 32])
        .await
        .unwrap();
    t.ba.set_sender_key_status(GROUP_1, &[("5511900000001:9@lid", true)])
        .await
        .unwrap();
    t.ba.set_sender_key_status(GROUP_2, &[("5511900000001:9@lid", false)])
        .await
        .unwrap();
    t.bb.put_session("5511900000001:3@s.whatsapp.net.0", b"other")
        .await
        .unwrap();

    assert_eq!(
        addresses(&*t.ba, USER).await,
        set(&[
            "5511900000001:7@lid.0",
            "5511900000001:9@lid",
            "5511900000001@s.whatsapp.net.0",
        ])
    );
    assert_eq!(
        addresses(&*t.bb, USER).await,
        set(&["5511900000001:3@s.whatsapp.net.0"])
    );
    h.drop_accounts(t).await;
}

/// The user must end where the address separator starts: a longer or shorter
/// number is a different person, and `_` / `%` are literal, never wildcards.
async fn matches_the_exact_user_only(h: Harness) {
    let t = h.two_accounts("sig-addr-exact").await;
    for address in [
        "55119000000012@s.whatsapp.net.0",
        "551190000000@s.whatsapp.net.0",
        "5511900000001.0",
        "12x4@lid.0",
        "12ab4@lid.0",
    ] {
        t.ba.put_session(address, b"s").await.unwrap();
    }
    t.ba.put_session("12_4@lid.0", b"s").await.unwrap();

    assert!(addresses(&*t.ba, USER).await.is_empty());
    assert_eq!(addresses(&*t.ba, "12_4").await, set(&["12_4@lid.0"]));
    assert!(addresses(&*t.ba, "12%").await.is_empty());
    h.drop_accounts(t).await;
}

/// Deleted state is not reported back: the cleanup it feeds would otherwise
/// chase devices that are already gone.
async fn forgets_deleted_state(h: Harness) {
    let t = h.two_accounts("sig-addr-deleted").await;
    t.ba.put_session("5511900000001:2@s.whatsapp.net.0", b"s")
        .await
        .unwrap();
    t.ba.set_sender_key_status(GROUP_1, &[("5511900000001:4@lid", true)])
        .await
        .unwrap();
    t.ba.delete_session("5511900000001:2@s.whatsapp.net.0")
        .await
        .unwrap();
    t.ba.clear_sender_key_devices(GROUP_1).await.unwrap();

    assert!(addresses(&*t.ba, USER).await.is_empty());
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_signal_addresses_collect_all_three_sources_once_per_account() {
    collects_all_three_sources_once_per_account(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_signal_addresses_collect_all_three_sources_once_per_account() {
    collects_all_three_sources_once_per_account(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_signal_addresses_collect_all_three_sources_once_per_account() {
    collects_all_three_sources_once_per_account(harness::turso().await).await;
}

#[tokio::test]
async fn postgres_signal_addresses_match_the_exact_user_only() {
    matches_the_exact_user_only(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_signal_addresses_match_the_exact_user_only() {
    matches_the_exact_user_only(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_signal_addresses_match_the_exact_user_only() {
    matches_the_exact_user_only(harness::turso().await).await;
}

#[tokio::test]
async fn postgres_signal_addresses_forget_deleted_state() {
    forgets_deleted_state(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_signal_addresses_forget_deleted_state() {
    forgets_deleted_state(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_signal_addresses_forget_deleted_state() {
    forgets_deleted_state(harness::turso().await).await;
}
