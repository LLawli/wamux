//! Status codes: the account resolution every RPC shares, the server's own
//! refusal, and each InvalidArgument branch the core has today. The last two
//! tests pin behavior #96 questions, so a change there shows up here.

use tonic::Code;
use wacore_binary::builder::NodeBuilder;
use wamux::proto::v1 as pb;

use crate::captured::{GROUP, GROUP_ID, REQUESTER_PN};
use crate::common;
use crate::common::mock_wire::{account_ref, iqs_in, sent_iq};
use crate::harness::{Fixture, G2, PICTURE, call_every_rpc, fixture};

fn assert_every(statuses: &[(&str, tonic::Status)], code: Code) {
    assert_eq!(statuses.len(), 21, "all 21 RPCs");
    for (rpc, status) in statuses {
        assert_eq!(status.code(), code, "{rpc}: {status:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_rpc_answers_not_found_for_an_unknown_account() {
    let mut f = fixture("every_rpc_answers_not_found_for_an_unknown_account").await;
    let unknown = account_ref(&uuid::Uuid::new_v4().to_string());
    assert_every(
        &call_every_rpc(&mut f.groups, &unknown).await,
        Code::NotFound,
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_rpc_answers_failed_precondition_when_not_connected() {
    let mut f = fixture("every_rpc_answers_failed_precondition_when_not_connected").await;
    let prefix = common::test_prefix("group_service", "every_rpc_not_connected_idle");
    common::sweep_orphans(f.logged.registry.storage(), &prefix).await;
    let idle = f
        .logged
        .registry
        .create_account(Some(&format!("{prefix}idle")))
        .await
        .expect("idle account");
    let idle_ref = account_ref(&idle.uuid.to_string());
    assert_every(
        &call_every_rpc(&mut f.groups, &idle_ref).await,
        Code::FailedPrecondition,
    );
    f.logged
        .registry
        .delete(&idle)
        .await
        .expect("delete the idle account");
    f.cleanup().await;
}

/// The server's refusal reaches the edge as itself: the code maps (403 is
/// PermissionDenied) and the raw code and text ride as trailers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_rpc_relays_a_server_refusal_as_permission_denied() {
    let mut f = fixture("every_rpc_relays_a_server_refusal_as_permission_denied").await;
    f.mock.answer_iq_error(G2, "*", "*", 403, "forbidden");
    f.mock.answer_iq_error(PICTURE, "*", "*", 403, "forbidden");
    let statuses = call_every_rpc(&mut f.groups, &f.account.clone()).await;
    assert_every(&statuses, Code::PermissionDenied);
    for (rpc, status) in &statuses {
        let trailer = |key| {
            status
                .metadata()
                .get(key)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        assert_eq!(trailer("wa-code").as_deref(), Some("403"), "{rpc}");
        assert_eq!(trailer("wa-text").as_deref(), Some("forbidden"), "{rpc}");
    }
    f.cleanup().await;
}

/// A group-scoped request with no usable group never leaves the core. The
/// positive signal: a valid call sent after them is the only `w:g2` IQ.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_group_rpc_rejects_an_empty_or_malformed_group_jid() {
    let mut f = fixture("every_group_rpc_rejects_an_empty_or_malformed_group_jid").await;
    let before = iqs_in(&f.mock, G2);
    for bad in ["", "not a jid"] {
        let checked = group_scoped_statuses(&mut f, bad).await;
        assert_eq!(checked.len(), 17, "every RPC that names a group");
        for (rpc, status) in checked {
            assert_eq!(
                status.code(),
                Code::InvalidArgument,
                "{rpc} with group {bad:?}"
            );
        }
    }
    let probe = pb::GroupTextRequest {
        account: Some(f.account.clone()),
        group_jid: GROUP.into(),
        text: "probe".into(),
    };
    f.groups
        .set_group_subject(probe)
        .await
        .expect("the valid probe");
    sent_iq(&f.mock, G2, "set", "subject").await;
    assert_eq!(
        iqs_in(&f.mock, G2),
        before + 1,
        "only the probe reached the wire"
    );
    f.cleanup().await;
}

/// The 17 RPCs that take a `group_jid`, all with `bad` as the group.
async fn group_scoped_statuses(f: &mut Fixture, bad: &str) -> Vec<(&'static str, tonic::Status)> {
    let a = || Some(f.account.clone());
    let gref = || pb::GroupRef {
        account: a(),
        group_jid: bad.into(),
    };
    let parts = || pb::ParticipantsRequest {
        account: a(),
        group_jid: bad.into(),
        participants: vec![REQUESTER_PN.into()],
    };
    let text = || pb::GroupTextRequest {
        account: a(),
        group_jid: bad.into(),
        text: "t".into(),
    };
    let toggle = || pb::GroupToggleRequest {
        account: a(),
        group_jid: bad.into(),
        enabled: true,
    };
    let g = &mut f.groups;
    vec![
        (
            "AddParticipants",
            g.add_participants(parts()).await.map(drop).unwrap_err(),
        ),
        (
            "RemoveParticipants",
            g.remove_participants(parts()).await.map(drop).unwrap_err(),
        ),
        (
            "PromoteAdmins",
            g.promote_admins(parts()).await.map(drop).unwrap_err(),
        ),
        (
            "DemoteAdmins",
            g.demote_admins(parts()).await.map(drop).unwrap_err(),
        ),
        (
            "SetGroupSubject",
            g.set_group_subject(text()).await.map(drop).unwrap_err(),
        ),
        (
            "SetGroupDescription",
            g.set_group_description(text()).await.map(drop).unwrap_err(),
        ),
        (
            "GetGroupMetadata",
            g.get_group_metadata(gref()).await.map(drop).unwrap_err(),
        ),
        (
            "GetInviteLink",
            g.get_invite_link(gref()).await.map(drop).unwrap_err(),
        ),
        (
            "RevokeInviteLink",
            g.revoke_invite_link(gref()).await.map(drop).unwrap_err(),
        ),
        (
            "LeaveGroup",
            g.leave_group(gref()).await.map(drop).unwrap_err(),
        ),
        (
            "SetGroupAnnounce",
            g.set_group_announce(toggle()).await.map(drop).unwrap_err(),
        ),
        (
            "SetGroupLocked",
            g.set_group_locked(toggle()).await.map(drop).unwrap_err(),
        ),
        (
            "SetGroupEphemeral",
            g.set_group_ephemeral(pb::GroupEphemeralRequest {
                account: a(),
                group_jid: bad.into(),
                expiration_seconds: 60,
            })
            .await
            .map(drop)
            .unwrap_err(),
        ),
        (
            "SetGroupPhoto",
            g.set_group_photo(pb::SetGroupPhotoRequest {
                account: a(),
                group_jid: bad.into(),
                image: vec![1],
            })
            .await
            .map(drop)
            .unwrap_err(),
        ),
        (
            "GetMembershipRequests",
            g.get_membership_requests(gref())
                .await
                .map(drop)
                .unwrap_err(),
        ),
        (
            "ApproveMembershipRequests",
            g.approve_membership_requests(parts())
                .await
                .map(drop)
                .unwrap_err(),
        ),
        (
            "RejectMembershipRequests",
            g.reject_membership_requests(parts())
                .await
                .map(drop)
                .unwrap_err(),
        ),
    ]
}

/// The seven RPCs that take participants refuse one that is not a jid.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn participant_rpcs_reject_a_malformed_participant() {
    let mut f = fixture("participant_rpcs_reject_a_malformed_participant").await;
    let a = Some(f.account.clone());
    let people = vec![REQUESTER_PN.to_string(), "not a jid".to_string()];
    let parts = || pb::ParticipantsRequest {
        account: a.clone(),
        group_jid: GROUP.into(),
        participants: people.clone(),
    };
    let create = pb::CreateGroupRequest {
        account: a.clone(),
        subject: "s".into(),
        participants: people.clone(),
    };
    let g = &mut f.groups;
    let statuses = [
        (
            "CreateGroup",
            g.create_group(create).await.map(drop).unwrap_err(),
        ),
        (
            "AddParticipants",
            g.add_participants(parts()).await.map(drop).unwrap_err(),
        ),
        (
            "RemoveParticipants",
            g.remove_participants(parts()).await.map(drop).unwrap_err(),
        ),
        (
            "PromoteAdmins",
            g.promote_admins(parts()).await.map(drop).unwrap_err(),
        ),
        (
            "DemoteAdmins",
            g.demote_admins(parts()).await.map(drop).unwrap_err(),
        ),
        (
            "ApproveMembershipRequests",
            g.approve_membership_requests(parts())
                .await
                .map(drop)
                .unwrap_err(),
        ),
        (
            "RejectMembershipRequests",
            g.reject_membership_requests(parts())
                .await
                .map(drop)
                .unwrap_err(),
        ),
    ];
    for (rpc, status) in statuses {
        assert_eq!(status.code(), Code::InvalidArgument, "{rpc}: {status:?}");
    }
    f.cleanup().await;
}

/// The limit is 100 characters, counted as chars: 100 multi-byte ones pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_subject_rejects_more_than_100_chars() {
    let mut f = fixture("set_group_subject_rejects_more_than_100_chars").await;
    let request = |text: String| pb::GroupTextRequest {
        account: Some(f.account.clone()),
        group_jid: GROUP.into(),
        text,
    };
    let too_long = f
        .groups
        .set_group_subject(request("é".repeat(101)))
        .await
        .unwrap_err();
    assert_eq!(too_long.code(), Code::InvalidArgument);
    f.groups
        .set_group_subject(request("é".repeat(100)))
        .await
        .expect("100 chars is the limit, not over it");
    sent_iq(&f.mock, G2, "set", "subject").await;
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_group_description_rejects_more_than_2048_chars() {
    let mut f = fixture("set_group_description_rejects_more_than_2048_chars").await;
    let request = pb::GroupTextRequest {
        account: Some(f.account.clone()),
        group_jid: GROUP.into(),
        text: "d".repeat(2049),
    };
    let status = f.groups.set_group_description(request).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert_eq!(
        iqs_in(&f.mock, G2),
        0,
        "not even the description-id read went out"
    );
    f.cleanup().await;
}

/// #96: an empty invite code is refused by the library, not the core, and the
/// edge gets Unavailable ("core down") for a malformed request. Pinned as is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_invite_code_answers_unavailable_today() {
    let mut f = fixture("an_empty_invite_code_answers_unavailable_today").await;
    let a = Some(f.account.clone());
    let join = f
        .groups
        .join_with_invite(pb::JoinWithInviteRequest {
            account: a.clone(),
            code: String::new(),
        })
        .await
        .unwrap_err();
    let preview = f
        .groups
        .preview_invite(pb::PreviewInviteRequest {
            account: a,
            code: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(join.code(), Code::Unavailable);
    assert_eq!(preview.code(), Code::Unavailable);
    f.cleanup().await;
}

/// #96: a 409 on the description set is rewritten by the library into an
/// error without a code, so it is the one refusal that is not relayed as
/// itself. Pinned as is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_description_conflict_answers_unavailable_today() {
    let mut f = fixture("a_description_conflict_answers_unavailable_today").await;
    f.mock.answer_iq(
        G2,
        "get",
        "query",
        NodeBuilder::new("group").attr("id", GROUP_ID).build(),
    );
    f.mock
        .answer_iq_error(G2, "set", "description", 409, "conflict");
    let request = pb::GroupTextRequest {
        account: Some(f.account.clone()),
        group_jid: GROUP.into(),
        text: "x".into(),
    };
    let status = f.groups.set_group_description(request).await.unwrap_err();
    assert_eq!(status.code(), Code::Unavailable);
    assert!(status.metadata().get("wa-code").is_none());
    f.cleanup().await;
}
