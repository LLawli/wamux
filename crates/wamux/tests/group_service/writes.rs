//! The writes: what each one put on the wire, and what it relayed back. The
//! answers are shaped after the whatsapp-rust 6f07e3a parser (no live capture
//! of a write was taken, #68).

use wacore_binary::builder::NodeBuilder;
use wacore_binary::{Node, NodeContent};
use wamux::proto::v1 as pb;

use crate::captured::{GROUP, GROUP_ID, INVITE_CODE, OWNER_LID, OWNER_PN, REQUESTER_PN};
use crate::harness::{G2, PICTURE, attr, fixture, operation, participant_jids, sent_iq};

fn op(iq: &Node) -> &Node {
    operation(iq).expect("the iq carries an operation")
}

fn text_of(node: &Node) -> Option<String> {
    node.content_as_string().map(|s| s.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_group_relays_the_new_jid_and_metadata() {
    let mut f = fixture("create_group_relays_the_new_jid_and_metadata").await;
    let created = NodeBuilder::new("group")
        .attr("id", GROUP_ID)
        .attr("subject", "new group")
        .attr("addressing_mode", "lid")
        .children([NodeBuilder::new("participant")
            .attr("jid", OWNER_LID)
            .attr("phone_number", OWNER_PN)
            .attr("type", "superadmin")
            .build()])
        .build();
    f.mock.answer_iq(G2, "set", "create", created);
    let request = pb::CreateGroupRequest {
        account: Some(f.account.clone()),
        subject: "new group".into(),
        participants: vec![REQUESTER_PN.into()],
    };
    let answer = f
        .groups
        .create_group(request)
        .await
        .expect("create")
        .into_inner();
    assert_eq!(answer.group_jid, GROUP);
    let metadata: serde_json::Value = serde_json::from_slice(&answer.metadata).expect("json");
    assert_eq!(metadata["id"], GROUP);
    assert_eq!(metadata["subject"], "new group");
    assert_eq!(metadata["participants"][0]["phone_number"], OWNER_PN);
    let iq = sent_iq(&f.mock, G2, "set", "create").await;
    assert_eq!(attr(op(&iq), "subject").as_deref(), Some("new group"));
    assert_eq!(participant_jids(op(&iq)), [REQUESTER_PN]);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_subject_puts_the_subject_on_the_wire() {
    let mut f = fixture("set_group_subject_puts_the_subject_on_the_wire").await;
    let request = pb::GroupTextRequest {
        account: Some(f.account.clone()),
        group_jid: GROUP.into(),
        text: "renamed".into(),
    };
    f.groups.set_group_subject(request).await.expect("subject");
    let iq = sent_iq(&f.mock, G2, "set", "subject").await;
    assert_eq!(attr(&iq, "to").as_deref(), Some(GROUP));
    assert_eq!(text_of(op(&iq)).as_deref(), Some("renamed"));
    f.cleanup().await;
}

/// `PreviousDescription::Resolve`: the library reads the current description
/// id first and sends it as `prev`, the server's optimistic-concurrency token.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_description_reads_the_id_then_sets() {
    let mut f = fixture("set_group_description_reads_the_id_then_sets").await;
    let current = NodeBuilder::new("group")
        .attr("id", GROUP_ID)
        .children([NodeBuilder::new("description")
            .attr("id", "D1D2D3D4")
            .children([NodeBuilder::new("body").string_content("old").build()])
            .build()])
        .build();
    f.mock.answer_iq(G2, "get", "query", current);
    let request = pb::GroupTextRequest {
        account: Some(f.account.clone()),
        group_jid: GROUP.into(),
        text: "new text".into(),
    };
    f.groups
        .set_group_description(request)
        .await
        .expect("description");
    let iq = sent_iq(&f.mock, G2, "set", "description").await;
    let description = op(&iq);
    assert_eq!(attr(description, "prev").as_deref(), Some("D1D2D3D4"));
    let id = attr(description, "id").expect("a new description id");
    assert!(
        id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit()),
        "id {id:?}"
    );
    let body = description.get_optional_child("body").expect("<body>");
    assert_eq!(text_of(body).as_deref(), Some("new text"));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoke_invite_link_resets_and_relays_the_new_link() {
    let mut f = fixture("revoke_invite_link_resets_and_relays_the_new_link").await;
    let fresh = NodeBuilder::new("invite")
        .attr("code", "ZyXwVuTsRqPoNmLkJiHgFe")
        .build();
    f.mock.answer_iq(G2, "set", "invite", fresh);
    let answer = f
        .groups
        .revoke_invite_link(f.group_ref())
        .await
        .expect("revoke")
        .into_inner();
    assert_eq!(
        answer.link,
        "https://chat.whatsapp.com/ZyXwVuTsRqPoNmLkJiHgFe"
    );
    let iq = sent_iq(&f.mock, G2, "set", "invite").await;
    assert_eq!(
        attr(&iq, "to").as_deref(),
        Some(GROUP),
        "set on the group: a reset"
    );
    f.cleanup().await;
}

/// Joined and PendingApproval relay the same `group_jid`: pinned as it is,
/// the collapse is #96's to decide.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn join_with_invite_relays_the_group_jid() {
    let mut f = fixture("join_with_invite_relays_the_group_jid").await;
    let joined = NodeBuilder::new("group").attr("jid", GROUP).build();
    let pending = NodeBuilder::new("membership_approval_request")
        .attr("jid", GROUP)
        .build();
    for answer in [joined, pending] {
        f.mock.answer_iq(G2, "set", "invite", answer);
        let request = pb::JoinWithInviteRequest {
            account: Some(f.account.clone()),
            code: INVITE_CODE.into(),
        };
        let relayed = f
            .groups
            .join_with_invite(request)
            .await
            .expect("join")
            .into_inner();
        assert_eq!(relayed.group_jid, GROUP);
        assert!(relayed.metadata.is_empty());
    }
    let iq = sent_iq(&f.mock, G2, "set", "invite").await;
    assert_eq!(attr(op(&iq), "code").as_deref(), Some(INVITE_CODE));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leave_group_puts_the_leave_on_the_wire() {
    let mut f = fixture("leave_group_puts_the_leave_on_the_wire").await;
    f.groups.leave_group(f.group_ref()).await.expect("leave");
    let iq = sent_iq(&f.mock, G2, "set", "leave").await;
    let group = op(&iq).get_optional_child("group").expect("<group>");
    assert_eq!(attr(group, "id").as_deref(), Some(GROUP));
    f.cleanup().await;
}

/// A toggle sends one tag for on and its opposite for off.
async fn toggle_round(f: &mut crate::harness::Fixture, rpc: &str, on: &str, off: &str) {
    for (enabled, tag) in [(true, on), (false, off)] {
        let request = pb::GroupToggleRequest {
            account: Some(f.account.clone()),
            group_jid: GROUP.into(),
            enabled,
        };
        let result = match rpc {
            "announce" => f.groups.set_group_announce(request).await,
            _ => f.groups.set_group_locked(request).await,
        };
        result.unwrap_or_else(|e| panic!("{rpc} {enabled}: {e}"));
        let iq = sent_iq(&f.mock, G2, "set", tag).await;
        assert_eq!(attr(&iq, "to").as_deref(), Some(GROUP));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_announce_sends_announcement_and_its_opposite() {
    let mut f = fixture("set_group_announce_sends_announcement_and_its_opposite").await;
    toggle_round(&mut f, "announce", "announcement", "not_announcement").await;
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_locked_sends_locked_and_unlocked() {
    let mut f = fixture("set_group_locked_sends_locked_and_unlocked").await;
    toggle_round(&mut f, "locked", "locked", "unlocked").await;
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_ephemeral_sends_the_timer_and_zero_disables() {
    let mut f = fixture("set_group_ephemeral_sends_the_timer_and_zero_disables").await;
    for seconds in [86_400u32, 0] {
        let request = pb::GroupEphemeralRequest {
            account: Some(f.account.clone()),
            group_jid: GROUP.into(),
            expiration_seconds: seconds,
        };
        f.groups
            .set_group_ephemeral(request)
            .await
            .expect("ephemeral");
    }
    let on = sent_iq(&f.mock, G2, "set", "ephemeral").await;
    assert_eq!(attr(op(&on), "expiration").as_deref(), Some("86400"));
    let off = sent_iq(&f.mock, G2, "set", "not_ephemeral").await;
    assert_eq!(attr(&off, "to").as_deref(), Some(GROUP));
    f.cleanup().await;
}

/// Non-empty bytes set the photo; empty bytes must route to `remove_group`,
/// because `set_group` panics on an empty image.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_photo_sets_and_empty_removes() {
    let mut f = fixture("set_group_photo_sets_and_empty_removes").await;
    f.mock.answer_iq(
        PICTURE,
        "set",
        "picture",
        NodeBuilder::new("picture").attr("id", "987654321").build(),
    );
    let image = vec![0xff, 0xd8, 0xff, 0xe0];
    let set = pb::SetGroupPhotoRequest {
        account: Some(f.account.clone()),
        group_jid: GROUP.into(),
        image: image.clone(),
    };
    f.groups.set_group_photo(set).await.expect("set photo");
    let iq = sent_iq(&f.mock, PICTURE, "set", "picture").await;
    assert_eq!(attr(&iq, "target").as_deref(), Some(GROUP));
    assert_eq!(attr(op(&iq), "type").as_deref(), Some("image"));
    assert_eq!(op(&iq).content, Some(NodeContent::Bytes(image)));
    let remove = pb::SetGroupPhotoRequest {
        account: Some(f.account.clone()),
        group_jid: GROUP.into(),
        image: Vec::new(),
    };
    f.groups
        .set_group_photo(remove)
        .await
        .expect("remove photo");
    let iq = sent_iq(&f.mock, PICTURE, "set", "").await;
    assert_eq!(attr(&iq, "target").as_deref(), Some(GROUP));
    f.cleanup().await;
}
