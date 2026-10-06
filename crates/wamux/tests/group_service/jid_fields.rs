//! #121: every jid in `groups.proto` and `contacts.proto` is a `pb::Jid`. A
//! required one that is unset or empty answers "missing jid" and reaches
//! nothing; a list entry is required too. ContactService rides along here
//! because this fixture is a connected account.

use tonic::Code;
use wamux::proto::v1 as pb;

use crate::captured::{GROUP, REQUESTER_PN};
use crate::common::mock_wire::{iqs_in, sent_iq};
use crate::harness::{G2, fixture, jid, jids};

fn empty() -> Option<pb::Jid> {
    Some(pb::Jid::default())
}

fn assert_missing_jid(results: Vec<(&str, Result<(), tonic::Status>)>) {
    for (case, result) in results {
        let status = result.expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
        assert_eq!(status.message(), "missing jid", "{case}");
    }
}

// Decided 2026-10-05: unset and empty are the same mistake, for the group and
// for each participant; they answered "empty jid" as strings.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unset_or_empty_group_or_participant_answers_missing_jid() {
    let mut f = fixture("an_unset_or_empty_group_or_participant_answers_missing_jid").await;
    let before = iqs_in(&f.mock, G2);
    let a = || Some(f.account.clone());
    let gref = |group: Option<pb::Jid>| pb::GroupRef {
        account: a(),
        group,
    };
    let parts = |participants: Vec<pb::Jid>| pb::ParticipantsRequest {
        account: a(),
        group: jid(GROUP),
        participants,
    };
    let create = pb::CreateGroupRequest {
        account: a(),
        subject: "s".into(),
        participants: vec![pb::Jid::default()],
    };
    let unset_group = gref(None);
    let empty_group = gref(empty());
    let empty_member = parts(vec![jid(REQUESTER_PN).unwrap(), pb::Jid::default()]);
    let g = &mut f.groups;
    let results = vec![
        (
            "metadata: unset group",
            g.get_group_metadata(unset_group).await.map(drop),
        ),
        (
            "leave: empty group",
            g.leave_group(empty_group).await.map(drop),
        ),
        (
            "add: an empty participant",
            g.add_participants(empty_member).await.map(drop),
        ),
        (
            "create: an empty participant",
            g.create_group(create).await.map(drop),
        ),
    ];
    assert_missing_jid(results);
    let probe = pb::GroupTextRequest {
        account: a(),
        group: jid(GROUP),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contact_rpcs_answer_missing_jid_for_an_unset_or_empty_jid() {
    let mut f = fixture("contact_rpcs_answer_missing_jid_for_an_unset_or_empty_jid").await;
    let a = || Some(f.account.clone());
    let about = pb::JidRequest {
        account: a(),
        jid: None,
    };
    let picture = pb::JidRequest {
        account: a(),
        jid: empty(),
    };
    let presence = pb::SubscribePresenceRequest {
        account: a(),
        jid: None,
    };
    let check = pb::CheckOnWhatsAppRequest {
        account: a(),
        jids: vec![pb::Jid::default()],
    };
    let resolve = pb::ResolveLidPnRequest {
        account: a(),
        jids: jids(&["100000000000002@lid", ""]),
    };
    let c = &mut f.contacts;
    let results = vec![
        ("GetAbout: unset jid", c.get_about(about).await.map(drop)),
        (
            "GetProfilePicture: empty jid",
            c.get_profile_picture(picture).await.map(drop),
        ),
        (
            "SubscribePresence: unset jid",
            c.subscribe_presence(presence).await.map(drop),
        ),
        (
            "CheckOnWhatsApp: an empty jid",
            c.check_on_whats_app(check).await.map(drop),
        ),
        (
            "ResolveLidPn: an empty jid",
            c.resolve_lid_pn(resolve).await.map(drop),
        ),
    ];
    assert_missing_jid(results);
    f.cleanup().await;
}

// Decided 2026-10-06: `LidPnResult.query` is a `Jid` carrying the value as it
// was sent. The lookup parses (a `@c.us` query is looked up as a phone), the
// echo does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_lid_pn_echoes_each_query_verbatim_in_a_jid() {
    let mut f = fixture("resolve_lid_pn_echoes_each_query_verbatim_in_a_jid").await;
    let sent = ["5511900000009@c.us", "100000000000009@lid"];
    let results = f
        .contacts
        .resolve_lid_pn(pb::ResolveLidPnRequest {
            account: Some(f.account.clone()),
            jids: jids(&sent),
        })
        .await
        .expect("resolve")
        .into_inner()
        .results;
    let echoed: Vec<Option<pb::Jid>> = results.iter().map(|r| r.query.clone()).collect();
    assert_eq!(echoed, vec![jid(sent[0]), jid(sent[1])]);
    assert!(
        results.iter().all(|r| !r.found && r.mapping.is_none()),
        "{results:?}"
    );
    f.cleanup().await;
}
