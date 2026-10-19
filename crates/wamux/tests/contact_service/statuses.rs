//! #197: every one of the 15 ContactService RPCs answers a status that comes
//! from the account, not from the operation, before any wire work.

use tonic::Code;
use tonic::transport::Channel;
use wamux::proto::v1 as pb;
use wamux::proto::v1::contact_service_client::ContactServiceClient;

use crate::common;
use crate::common::mock_wire::account_ref;
use crate::harness::{CONTACT_LID, CONTACT_PN, fixture, jid};

type Outcome = (&'static str, Result<(), tonic::Status>);

/// The four RPCs #197 added, with arguments valid in shape.
async fn call_new_rpcs(
    contacts: &mut ContactServiceClient<Channel>,
    account: &pb::AccountRef,
) -> Vec<Outcome> {
    let a = || Some(account.clone());
    let lid = || pb::JidRequest {
        account: a(),
        jid: jid(CONTACT_LID),
    };
    vec![
        (
            "SetAbout",
            contacts
                .set_about(pb::SetAboutRequest {
                    account: a(),
                    about: "about".into(),
                })
                .await
                .map(drop),
        ),
        (
            "BlockContact",
            contacts.block_contact(lid()).await.map(drop),
        ),
        (
            "UnblockContact",
            contacts.unblock_contact(lid()).await.map(drop),
        ),
        (
            "GetBlocklist",
            contacts.get_blocklist(account.clone()).await.map(drop),
        ),
    ]
}

/// The eleven RPCs that predate #197, with arguments valid in shape.
async fn call_older_rpcs(
    contacts: &mut ContactServiceClient<Channel>,
    account: &pb::AccountRef,
) -> Vec<Outcome> {
    let a = || Some(account.clone());
    let pn = || pb::JidRequest {
        account: a(),
        jid: jid(CONTACT_PN),
    };
    let pns = || {
        vec![pb::Jid {
            value: CONTACT_PN.into(),
        }]
    };
    vec![
        (
            "CheckOnWhatsApp",
            contacts
                .check_on_whats_app(pb::CheckOnWhatsAppRequest {
                    account: a(),
                    jids: pns(),
                })
                .await
                .map(drop),
        ),
        (
            "GetProfilePicture",
            contacts.get_profile_picture(pn()).await.map(drop),
        ),
        (
            "SetProfilePicture",
            contacts
                .set_profile_picture(pb::SetProfilePictureRequest {
                    account: a(),
                    image: vec![0xff, 0xd8],
                })
                .await
                .map(drop),
        ),
        (
            "RemoveProfilePicture",
            contacts
                .remove_profile_picture(account.clone())
                .await
                .map(drop),
        ),
        (
            "GetPushName",
            contacts.get_push_name(account.clone()).await.map(drop),
        ),
        (
            "SetPushName",
            contacts
                .set_push_name(pb::SetPushNameRequest {
                    account: a(),
                    push_name: "name".into(),
                })
                .await
                .map(drop),
        ),
        ("GetAbout", contacts.get_about(pn()).await.map(drop)),
        (
            "GetBusinessProfile",
            contacts.get_business_profile(pn()).await.map(drop),
        ),
        (
            "SubscribePresence",
            contacts
                .subscribe_presence(pb::SubscribePresenceRequest {
                    account: a(),
                    jid: jid(CONTACT_PN),
                })
                .await
                .map(drop),
        ),
        (
            "ResolveLidPn",
            contacts
                .resolve_lid_pn(pb::ResolveLidPnRequest {
                    account: a(),
                    jids: pns(),
                })
                .await
                .map(drop),
        ),
        (
            "ListLidMappings",
            contacts.list_lid_mappings(account.clone()).await.map(drop),
        ),
    ]
}

fn assert_every(outcomes: Vec<Outcome>, expected: usize, code: Code) {
    assert_eq!(outcomes.len(), expected, "RPCs called");
    for (rpc, outcome) in outcomes {
        let status = outcome.expect_err(&format!("{rpc} must fail here"));
        assert_eq!(status.code(), code, "{rpc}: {status:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_rpc_answers_not_found_for_an_unknown_account() {
    let mut f = fixture("every_rpc_answers_not_found_for_an_unknown_account").await;
    let unknown = account_ref(&uuid::Uuid::new_v4().to_string());
    let mut outcomes = call_older_rpcs(&mut f.contacts, &unknown).await;
    outcomes.extend(call_new_rpcs(&mut f.contacts, &unknown).await);
    assert_every(outcomes, 15, Code::NotFound);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_rpcs_answer_failed_precondition_when_not_connected() {
    let mut f = fixture("new_rpcs_answer_failed_precondition_when_not_connected").await;
    let prefix = common::test_prefix("contact_service", "new_rpcs_not_connected_idle");
    common::sweep_orphans(f.logged.registry.storage(), &prefix).await;
    let idle = f
        .logged
        .registry
        .create_account(Some(&format!("{prefix}idle")))
        .await
        .expect("idle account");
    let idle_ref = account_ref(&idle.uuid.to_string());
    let outcomes = call_new_rpcs(&mut f.contacts, &idle_ref).await;
    assert_every(outcomes, 4, Code::FailedPrecondition);
    f.logged
        .registry
        .delete(&idle)
        .await
        .expect("delete the idle account");
    f.cleanup().await;
}
