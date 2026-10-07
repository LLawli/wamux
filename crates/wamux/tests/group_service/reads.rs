//! The five reads, answered with what WhatsApp's server sent on 2026-10-02
//! (`captured`), relayed through the socket by value: the typed metadata is
//! compared as the whole message (#132), so a dropped field is a difference.

use wamux::proto::v1 as pb;

use crate::captured::{
    self, GROUP, GROUP_ID, INVITE_CODE, OWNER_LID, OWNER_PN, REQUESTER_LID, SUBJECT,
};
use crate::common::mock_wire::{attr, operation, sent_iq};
use crate::harness::{G2, fixture, jid};

/// `CREATION` (unix seconds, as captured) in the contract's milliseconds (#132).
const CREATION_MS: i64 = 1_790_947_599_000;

/// The owner as the roster carries it: the `@lid` jid AND the phone number
/// beside it (issue #1), the admin role, and, when the answer had them, the
/// join time and history flag.
fn owner(details: Option<pb::GroupParticipantDetails>) -> pb::GroupParticipant {
    pb::GroupParticipant {
        jid: jid(OWNER_LID),
        phone_number: jid(OWNER_PN),
        lid: None,
        username: None,
        r#type: pb::GroupParticipantType::Superadmin as i32,
        details,
    }
}

fn owner_details() -> pb::GroupParticipantDetails {
    pb::GroupParticipantDetails {
        join_time: Some(CREATION_MS),
        group_history_sent: Some(false),
        is_addressable: true,
        ..pb::GroupParticipantDetails::default()
    }
}

/// What every captured `<group>` answer shares: the head attributes and the
/// `<ephemeral expiration="0"/>` and approval-on children. Absent fields stay
/// unset, never guessed (`<description/>` with no body is no description).
fn captured_group_head(participant: pb::GroupParticipant) -> pb::GroupMetadata {
    pb::GroupMetadata {
        id: jid(GROUP),
        subject: Some(SUBJECT.to_string()),
        participants: vec![participant],
        addressing_mode: pb::GroupAddressingMode::Lid as i32,
        creator: jid(OWNER_LID),
        creator_pn: jid(OWNER_PN),
        creator_country_code: Some("BR".to_string()),
        creation_time: Some(CREATION_MS),
        subject_time: Some(CREATION_MS),
        subject_owner: jid(OWNER_LID),
        subject_owner_pn: jid(OWNER_PN),
        ephemeral: Some(pb::GroupEphemeralSettings {
            expiration_seconds: Some(0),
            trigger: None,
        }),
        membership_approval: true,
        ..pb::GroupMetadata::default()
    }
}

/// GetGroupMetadata and ListGroups answered the member's view: the version ids,
/// the three modes, and the full roster entry. Only GetGroupMetadata had `size`.
///
/// TODAY the three modes are UNSPECIFIED, although the server sent
/// `all_member_add`, `admin_link` and `all_member_share` (the capture, and live
/// on 2026-10-07). The values are not protocol tokens, so they cross the binary
/// codec as raw bytes, and the library at the pin (6f07e3a) reads them only as
/// string content and drops them (`wacore/src/iq/groups.rs`). Upstream fixed it
/// in #1651 (2026-10-05, after the pin): at the next bump these become the
/// server's values and this test must be flipped back (#132).
fn captured_member_view(size: Option<u32>) -> pb::GroupMetadata {
    pb::GroupMetadata {
        participant_version_id: Some("1790947897308104".to_string()),
        admin_version_id: Some("1790947599193645".to_string()),
        member_add_mode: pb::GroupMemberAddMode::Unspecified as i32,
        member_link_mode: pb::GroupMemberLinkMode::Unspecified as i32,
        member_share_history_mode: pb::GroupMemberShareHistoryMode::Unspecified as i32,
        size,
        ..captured_group_head(owner(Some(owner_details())))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_group_metadata_relays_the_captured_roster() {
    let mut f = fixture("get_group_metadata_relays_the_captured_roster").await;
    f.mock.answer_iq(G2, "get", "query", captured::metadata());
    let answer = f
        .groups
        .get_group_metadata(f.group_ref())
        .await
        .expect("metadata")
        .into_inner();
    assert_eq!(answer.metadata, Some(captured_member_view(Some(1))));
    let iq = sent_iq(&f.mock, G2, "get", "query").await;
    assert_eq!(attr(&iq, "to").as_deref(), Some(GROUP));
    assert_eq!(
        attr(operation(&iq).unwrap(), "request").as_deref(),
        Some("interactive")
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_invite_link_relays_the_link() {
    let mut f = fixture("get_invite_link_relays_the_link").await;
    f.mock.answer_iq(G2, "get", "invite", captured::invite());
    let answer = f
        .groups
        .get_invite_link(f.group_ref())
        .await
        .expect("link")
        .into_inner();
    assert_eq!(
        answer.link,
        format!("https://chat.whatsapp.com/{INVITE_CODE}")
    );
    let iq = sent_iq(&f.mock, G2, "get", "invite").await;
    assert_eq!(
        attr(&iq, "to").as_deref(),
        Some(GROUP),
        "get, not set: no reset"
    );
    f.cleanup().await;
}

/// #132 settles the shape #96 questioned: each `jid` crosses as a `Jid`, not
/// the library's struct as JSON, and the request time in milliseconds. The
/// server's `phone_number` and `request_method` are not parsed by the library,
/// so they are not relayed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_membership_requests_relays_the_captured_list() {
    let mut f = fixture("get_membership_requests_relays_the_captured_list").await;
    f.mock.answer_iq(
        G2,
        "get",
        "membership_approval_requests",
        captured::membership_requests(),
    );
    let answer = f
        .groups
        .get_membership_requests(f.group_ref())
        .await
        .expect("requests")
        .into_inner();
    let expected = vec![pb::MembershipRequest {
        jid: jid(REQUESTER_LID),
        request_time: Some(1_790_947_901_000),
    }];
    assert_eq!(answer.requests, expected);
    let iq = sent_iq(&f.mock, G2, "get", "membership_approval_requests").await;
    assert_eq!(attr(&iq, "to").as_deref(), Some(GROUP));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_invite_relays_the_captured_metadata() {
    let mut f = fixture("preview_invite_relays_the_captured_metadata").await;
    f.mock
        .answer_iq(G2, "get", "invite", captured::invite_preview());
    let request = pb::PreviewInviteRequest {
        account: Some(f.account.clone()),
        code: INVITE_CODE.into(),
    };
    let answer = f
        .groups
        .preview_invite(request)
        .await
        .expect("preview")
        .into_inner();
    // A non-member's view: no version ids, no modes, a bare roster entry.
    let expected = pb::GroupMetadata {
        size: Some(1),
        ..captured_group_head(owner(None))
    };
    assert_eq!(answer.metadata, Some(expected));
    let iq = sent_iq(&f.mock, G2, "get", "invite").await;
    assert_eq!(
        attr(operation(&iq).unwrap(), "code").as_deref(),
        Some(INVITE_CODE)
    );
    assert!(
        attr(&iq, "to").is_some_and(|to| to.ends_with("g.us") && !to.contains(GROUP_ID)),
        "a preview asks g.us, not the group it does not belong to yet"
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_participating_relays_the_captured_group() {
    let mut f = fixture("list_participating_relays_the_captured_group").await;
    f.mock
        .answer_iq(G2, "get", "participating", captured::participating());
    let answer = f
        .groups
        .list_participating(f.account.clone())
        .await
        .expect("list")
        .into_inner();
    assert_eq!(answer.groups.len(), 1);
    let summary = &answer.groups[0];
    assert_eq!(summary.jid, jid(GROUP));
    assert_eq!(summary.subject, SUBJECT);
    assert_eq!(summary.participants, 1);
    assert_eq!(summary.metadata, Some(captured_member_view(None)));
    let iq = sent_iq(&f.mock, G2, "get", "participating").await;
    let asked: Vec<&str> = operation(&iq)
        .unwrap()
        .children()
        .unwrap_or(&[])
        .iter()
        .map(|c| c.tag.as_ref())
        .collect();
    assert_eq!(
        asked,
        ["participants", "description"],
        "the full roster, not the slim overview"
    );
    f.cleanup().await;
}
