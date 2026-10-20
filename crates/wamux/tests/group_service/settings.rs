//! The group settings an admin toggles (#198): membership approval and who may
//! add members go out as `w:g2` IQs, who may share the link and the history as
//! a MEX `update_group_property` mutation. Each test asserts what reached the
//! wire; the IQ shapes and the MEX variables are whatsapp-rust 6f07e3a's.

use tonic::Code;
use wacore_binary::{Node, NodeContent};
use wamux::proto::v1 as pb;

use crate::captured::GROUP;
use crate::common::mock_wire::{attr, iqs_in, operation, sent_iq};
use crate::harness::{Fixture, G2, MEX, fixture, jid};

/// The answer the server gives a property update it applied.
const APPLIED: &str = r#"{"data":{"xwa2_group_update_property":{"state":"ACTIVE"}}}"#;

fn op(iq: &Node) -> &Node {
    operation(iq).expect("the iq carries an operation")
}

/// The `variables` of the MEX mutation the client sent last.
async fn sent_mex_variables(f: &Fixture) -> serde_json::Value {
    let iq = sent_iq(&f.mock, MEX, "get", "query").await;
    let bytes: Vec<u8> = match &op(&iq).content {
        Some(NodeContent::Bytes(bytes)) => bytes.clone(),
        Some(NodeContent::String(text)) => text.as_bytes().to_vec(),
        other => panic!("the MEX query carries no payload: {other:?}"),
    };
    let payload: serde_json::Value = serde_json::from_slice(&bytes).expect("MEX payload is JSON");
    payload["variables"].clone()
}

fn add_mode(f: &Fixture, mode: i32) -> pb::GroupMemberAddModeRequest {
    pb::GroupMemberAddModeRequest {
        account: Some(f.account.clone()),
        group: jid(GROUP),
        mode,
    }
}

fn link_mode(f: &Fixture, mode: i32) -> pb::GroupMemberLinkModeRequest {
    pb::GroupMemberLinkModeRequest {
        account: Some(f.account.clone()),
        group: jid(GROUP),
        mode,
    }
}

fn share_mode(f: &Fixture, mode: i32) -> pb::GroupMemberShareHistoryModeRequest {
    pb::GroupMemberShareHistoryModeRequest {
        account: Some(f.account.clone()),
        group: jid(GROUP),
        mode,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_membership_approval_sends_on_and_off() {
    let mut f = fixture("set_group_membership_approval_sends_on_and_off").await;
    for (enabled, state) in [(true, "on"), (false, "off")] {
        let request = pb::GroupToggleRequest {
            account: Some(f.account.clone()),
            group: jid(GROUP),
            enabled,
        };
        f.groups
            .set_group_membership_approval(request)
            .await
            .unwrap_or_else(|e| panic!("approval {enabled}: {e}"));
        let iq = sent_iq(&f.mock, G2, "set", "membership_approval_mode").await;
        assert_eq!(attr(&iq, "to").as_deref(), Some(GROUP));
        let join = op(&iq)
            .get_optional_child("group_join")
            .expect("<group_join>");
        assert_eq!(attr(join, "state").as_deref(), Some(state), "{enabled}");
    }
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_member_add_mode_sends_each_mode() {
    let mut f = fixture("set_group_member_add_mode_sends_each_mode").await;
    for (mode, token) in [
        (pb::GroupMemberAddMode::AdminAdd, "admin_add"),
        (pb::GroupMemberAddMode::AllMemberAdd, "all_member_add"),
    ] {
        let request = add_mode(&f, mode as i32);
        f.groups
            .set_group_member_add_mode(request)
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        let iq = sent_iq(&f.mock, G2, "set", "member_add_mode").await;
        assert_eq!(attr(&iq, "to").as_deref(), Some(GROUP));
        assert_eq!(
            op(&iq)
                .content_as_string()
                .map(|s| s.to_string())
                .as_deref(),
            Some(token),
            "{mode:?}"
        );
    }
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_member_link_mode_sends_the_property_update() {
    let mut f = fixture("set_group_member_link_mode_sends_the_property_update").await;
    f.mock.answer_mex_with(APPLIED);
    for (mode, value) in [
        (pb::GroupMemberLinkMode::AdminLink, "ADMIN_LINK"),
        (pb::GroupMemberLinkMode::AllMemberLink, "ALL_MEMBER_LINK"),
    ] {
        let request = link_mode(&f, mode as i32);
        f.groups
            .set_group_member_link_mode(request)
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        let variables = sent_mex_variables(&f).await;
        assert_eq!(variables["group_id"], GROUP, "{mode:?}");
        assert_eq!(variables["update"]["member_link_mode"], value, "{mode:?}");
    }
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_member_share_history_mode_sends_the_property_update() {
    let mut f = fixture("set_group_member_share_history_mode_sends_the_property_update").await;
    f.mock.answer_mex_with(APPLIED);
    for (mode, value) in [
        (pb::GroupMemberShareHistoryMode::AdminShare, "ADMIN_SHARE"),
        (
            pb::GroupMemberShareHistoryMode::AllMemberShare,
            "ALL_MEMBER_SHARE",
        ),
    ] {
        let request = share_mode(&f, mode as i32);
        f.groups
            .set_group_member_share_history_mode(request)
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        let variables = sent_mex_variables(&f).await;
        assert_eq!(variables["group_id"], GROUP, "{mode:?}");
        assert_eq!(
            variables["update"]["member_share_group_history_mode"], value,
            "{mode:?}"
        );
    }
    f.cleanup().await;
}

/// UNSPECIFIED (an omitted field), UNKNOWN and a number the enum does not name
/// are refused by name, and none of them reaches the server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mode_rpcs_refuse_an_unspecified_or_unknown_mode_before_the_wire() {
    let mut f = fixture("mode_rpcs_refuse_an_unspecified_or_unknown_mode_before_the_wire").await;
    let (g2_before, mex_before) = (iqs_in(&f.mock, G2), iqs_in(&f.mock, MEX));
    for value in [0, 1, 42] {
        let statuses = [
            (
                "SetGroupMemberAddMode",
                f.groups
                    .set_group_member_add_mode(add_mode(&f, value))
                    .await,
            ),
            (
                "SetGroupMemberLinkMode",
                f.groups
                    .set_group_member_link_mode(link_mode(&f, value))
                    .await,
            ),
            (
                "SetGroupMemberShareHistoryMode",
                f.groups
                    .set_group_member_share_history_mode(share_mode(&f, value))
                    .await,
            ),
        ];
        for (rpc, result) in statuses {
            let status = result.expect_err(&format!("{rpc} with mode {value}"));
            assert_eq!(status.code(), Code::InvalidArgument, "{rpc} {value}");
            assert!(
                status.message().starts_with("mode must be one of "),
                "{rpc} {value}: {}",
                status.message()
            );
        }
    }
    assert_eq!(iqs_in(&f.mock, G2), g2_before, "no w:g2 IQ");
    assert_eq!(iqs_in(&f.mock, MEX), mex_before, "no MEX query");
    f.cleanup().await;
}

/// The server answered the mutation with a state other than ACTIVE: the update
/// was not applied. The library reports that as a payload error with no code,
/// which reaches the edge as Unavailable today. Pinned so a change to that
/// mapping is a decision, not an accident (#198 left it out of scope).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_property_update_the_server_did_not_apply_answers_unavailable_today() {
    let mut f =
        fixture("a_property_update_the_server_did_not_apply_answers_unavailable_today").await;
    f.mock
        .answer_mex_with(r#"{"data":{"xwa2_group_update_property":{"state":"SUSPENDED"}}}"#);
    let link = link_mode(&f, pb::GroupMemberLinkMode::AdminLink as i32);
    let status = f
        .groups
        .set_group_member_link_mode(link)
        .await
        .expect_err("not applied");
    assert_eq!(status.code(), Code::Unavailable, "{status:?}");
    // The mutation did go out: the refusal is the server's answer, not the core's.
    let variables = sent_mex_variables(&f).await;
    assert_eq!(variables["update"]["member_link_mode"], "ADMIN_LINK");
    let share = share_mode(&f, pb::GroupMemberShareHistoryMode::AdminShare as i32);
    let status = f
        .groups
        .set_group_member_share_history_mode(share)
        .await
        .expect_err("not applied");
    assert_eq!(status.code(), Code::Unavailable, "{status:?}");
    let variables = sent_mex_variables(&f).await;
    assert_eq!(
        variables["update"]["member_share_group_history_mode"],
        "ADMIN_SHARE"
    );
    f.cleanup().await;
}
