//! #197: BlockContact and UnblockContact. A block always carries both sides of
//! the pair (modern WhatsApp rejects a PN-only block), so it needs the mapping;
//! a jid that is not a user never reaches the wire.

use tonic::Code;

use crate::common::mock_wire::{attr, sent_iq};
use crate::contact_service::harness::{
    BLOCKLIST, CONTACT_LID, CONTACT_PN, STRANGER_PN, blocklist_item, fixture,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_contact_by_pn_sends_lid_and_pn() {
    let mut f = fixture("block_contact_by_pn_sends_lid_and_pn").await;
    f.seed_contact_mapping().await;
    f.contacts
        .block_contact(f.jid_request(CONTACT_PN))
        .await
        .expect("block by pn");
    let iq = sent_iq(&f.mock, BLOCKLIST, "set", "item").await;
    let item = blocklist_item(&iq);
    assert_eq!(attr(item, "action").as_deref(), Some("block"));
    assert_eq!(attr(item, "jid").as_deref(), Some(CONTACT_LID));
    assert_eq!(attr(item, "pn_jid").as_deref(), Some(CONTACT_PN));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_contact_by_lid_sends_lid_and_pn() {
    let mut f = fixture("block_contact_by_lid_sends_lid_and_pn").await;
    f.seed_contact_mapping().await;
    f.contacts
        .block_contact(f.jid_request(CONTACT_LID))
        .await
        .expect("block by lid");
    let iq = sent_iq(&f.mock, BLOCKLIST, "set", "item").await;
    let item = blocklist_item(&iq);
    assert_eq!(attr(item, "action").as_deref(), Some("block"));
    assert_eq!(attr(item, "jid").as_deref(), Some(CONTACT_LID));
    assert_eq!(attr(item, "pn_jid").as_deref(), Some(CONTACT_PN));
    f.cleanup().await;
}

/// No pair learned: the library refuses before any IQ. FailedPrecondition
/// tells the edge to learn the pair first; Unavailable would invite a retry
/// that can never work.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_contact_without_mapping_is_failed_precondition() {
    let mut f = fixture("block_contact_without_mapping_is_failed_precondition").await;
    let status = f
        .contacts
        .block_contact(f.jid_request(STRANGER_PN))
        .await
        .expect_err("no mapping, no block");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    assert!(
        status.message().contains("mapping"),
        "says what is missing: {status:?}"
    );
    assert!(f.blocklist_iqs().is_empty(), "nothing reached the wire");
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_contact_of_non_user_jid_is_invalid_argument() {
    let mut f = fixture("block_contact_of_non_user_jid_is_invalid_argument").await;
    f.seed_contact_mapping().await;
    for value in ["120363000000000001@g.us", "13135550002@bot"] {
        let status = f
            .contacts
            .block_contact(f.jid_request(value))
            .await
            .expect_err("not a user jid");
        assert_eq!(status.code(), Code::InvalidArgument, "{value}: {status:?}");
        assert!(status.message().contains(value), "{value}: {status:?}");
    }
    assert!(f.blocklist_iqs().is_empty(), "nothing reached the wire");
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_contact_server_refusal_is_relayed() {
    let mut f = fixture("block_contact_server_refusal_is_relayed").await;
    f.seed_contact_mapping().await;
    f.mock
        .answer_iq_error(BLOCKLIST, "set", "item", 403, "forbidden");
    let status = f
        .contacts
        .block_contact(f.jid_request(CONTACT_PN))
        .await
        .expect_err("the server refused");
    assert_eq!(
        status.code(),
        Code::PermissionDenied,
        "403 maps: {status:?}"
    );
    let code = status
        .metadata()
        .get("wa-code")
        .and_then(|v| v.to_str().ok());
    assert_eq!(code, Some("403"), "raw code rides along: {status:?}");
    f.cleanup().await;
}

/// The unblock stanza carries only the LID, so a LID needs no mapping.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unblock_contact_by_lid_needs_no_mapping() {
    let mut f = fixture("unblock_contact_by_lid_needs_no_mapping").await;
    f.contacts
        .unblock_contact(f.jid_request(CONTACT_LID))
        .await
        .expect("unblock by lid");
    let iq = sent_iq(&f.mock, BLOCKLIST, "set", "item").await;
    let item = blocklist_item(&iq);
    assert_eq!(attr(item, "action").as_deref(), Some("unblock"));
    assert_eq!(attr(item, "jid").as_deref(), Some(CONTACT_LID));
    assert_eq!(attr(item, "pn_jid"), None, "unblock sends the lid only");
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unblock_contact_by_pn_without_mapping_is_failed_precondition() {
    let mut f = fixture("unblock_contact_by_pn_without_mapping_is_failed_precondition").await;
    let status = f
        .contacts
        .unblock_contact(f.jid_request(STRANGER_PN))
        .await
        .expect_err("a pn needs its lid");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    assert!(f.blocklist_iqs().is_empty(), "nothing reached the wire");
    f.cleanup().await;
}
