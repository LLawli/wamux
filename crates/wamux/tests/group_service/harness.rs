//! One logged-in account behind a real socket, and the wire helpers every
//! GroupService test shares.

use std::time::Duration;

use tonic::transport::Channel;
use wacore_binary::Node;
use wamux::proto::v1 as pb;
use wamux::proto::v1::group_service_client::GroupServiceClient;
use wamux::stress::MockWaServer;

use crate::captured::{GROUP, INVITE_CODE, REQUESTER_PN};
use crate::common::{self, LoggedIn};

pub const G2: &str = "w:g2";
pub const PICTURE: &str = "w:profile:picture";

/// The daemon serving a registry whose account is logged in against `mock`.
pub struct Fixture {
    pub mock: MockWaServer,
    pub logged: LoggedIn,
    pub groups: GroupServiceClient<Channel>,
    pub account: pb::AccountRef,
}

pub async fn fixture(test: &str) -> Fixture {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix("group_service", test);
    let logged = common::logged_in_client(&mock, &prefix).await;
    let channel = common::serve_registry(logged.registry.clone()).await;
    let account = account_ref(&logged.handle.uuid.to_string());
    Fixture {
        mock,
        logged,
        groups: GroupServiceClient::new(channel),
        account,
    }
}

impl Fixture {
    pub async fn cleanup(self) {
        self.logged.cleanup().await;
    }

    pub fn group_ref(&self) -> pb::GroupRef {
        pb::GroupRef {
            account: Some(self.account.clone()),
            group_jid: GROUP.to_string(),
        }
    }

    pub fn participants(&self, participants: &[&str]) -> pb::ParticipantsRequest {
        pb::ParticipantsRequest {
            account: Some(self.account.clone()),
            group_jid: GROUP.to_string(),
            participants: participants.iter().map(|p| p.to_string()).collect(),
        }
    }
}

pub fn account_ref(uuid: &str) -> pb::AccountRef {
    pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::Uuid(uuid.to_string())),
    }
}

pub fn attr(node: &Node, key: &str) -> Option<String> {
    node.attrs.get(key).map(|value| value.as_str().into_owned())
}

/// The first child of an `<iq>`: the operation it carries.
pub fn operation(iq: &Node) -> Option<&Node> {
    iq.children().and_then(|children| children.first())
}

fn is_iq(iq: &Node, xmlns: &str, iq_type: &str, child: &str) -> bool {
    let tag = operation(iq).map(|c| c.tag.as_ref()).unwrap_or("");
    attr(iq, "xmlns").as_deref() == Some(xmlns)
        && attr(iq, "type").as_deref() == Some(iq_type)
        && tag == child
}

/// The newest `<iq>` the client sent with this namespace, type and first
/// child (`""` for an IQ with no child), waiting for it to reach the mock.
pub async fn sent_iq(mock: &MockWaServer, xmlns: &str, iq_type: &str, child: &str) -> Node {
    common::poll_until(
        &format!("an <iq xmlns={xmlns} type={iq_type}><{child}>"),
        Duration::from_secs(5),
        || async {
            mock.client_iqs()
                .into_iter()
                .rev()
                .find(|iq| is_iq(iq, xmlns, iq_type, child))
        },
    )
    .await
}

/// How many `<iq>` of a namespace the client has sent so far.
pub fn iqs_in(mock: &MockWaServer, xmlns: &str) -> usize {
    mock.client_iqs()
        .iter()
        .filter(|iq| attr(iq, "xmlns").as_deref() == Some(xmlns))
        .count()
}

/// The `jid` of every `<participant>` under the operation, in order.
pub fn participant_jids(operation: &Node) -> Vec<String> {
    operation
        .get_children_by_tag("participant")
        .filter_map(|p| attr(p, "jid"))
        .collect()
}

/// Every one of the 21 RPCs, called with arguments that are valid in shape,
/// for the tests that check a status common to all of them. The name is the
/// proto RPC, so a failure says which one.
pub async fn call_every_rpc(
    groups: &mut GroupServiceClient<Channel>,
    account: &pb::AccountRef,
) -> Vec<(&'static str, tonic::Status)> {
    let a = || Some(account.clone());
    let group = || GROUP.to_string();
    let people = || vec![REQUESTER_PN.to_string()];
    let gref = || pb::GroupRef {
        account: a(),
        group_jid: group(),
    };
    let parts = || pb::ParticipantsRequest {
        account: a(),
        group_jid: group(),
        participants: people(),
    };
    let text = |t: &str| pb::GroupTextRequest {
        account: a(),
        group_jid: group(),
        text: t.to_string(),
    };
    let toggle = || pb::GroupToggleRequest {
        account: a(),
        group_jid: group(),
        enabled: true,
    };
    let mut out = Vec::new();
    let mut note = |name: &'static str, r: Result<(), tonic::Status>| {
        out.push((name, r.expect_err(&format!("{name} must fail here"))));
    };
    note(
        "CreateGroup",
        groups
            .create_group(pb::CreateGroupRequest {
                account: a(),
                subject: "s".into(),
                participants: people(),
            })
            .await
            .map(drop),
    );
    note(
        "AddParticipants",
        groups.add_participants(parts()).await.map(drop),
    );
    note(
        "RemoveParticipants",
        groups.remove_participants(parts()).await.map(drop),
    );
    note(
        "PromoteAdmins",
        groups.promote_admins(parts()).await.map(drop),
    );
    note(
        "DemoteAdmins",
        groups.demote_admins(parts()).await.map(drop),
    );
    note(
        "SetGroupSubject",
        groups.set_group_subject(text("subject")).await.map(drop),
    );
    note(
        "SetGroupDescription",
        groups
            .set_group_description(text("description"))
            .await
            .map(drop),
    );
    note(
        "GetGroupMetadata",
        groups.get_group_metadata(gref()).await.map(drop),
    );
    note(
        "GetInviteLink",
        groups.get_invite_link(gref()).await.map(drop),
    );
    note(
        "RevokeInviteLink",
        groups.revoke_invite_link(gref()).await.map(drop),
    );
    note(
        "JoinWithInvite",
        groups
            .join_with_invite(pb::JoinWithInviteRequest {
                account: a(),
                code: INVITE_CODE.into(),
            })
            .await
            .map(drop),
    );
    note(
        "ListParticipating",
        groups.list_participating(account.clone()).await.map(drop),
    );
    note("LeaveGroup", groups.leave_group(gref()).await.map(drop));
    note(
        "SetGroupAnnounce",
        groups.set_group_announce(toggle()).await.map(drop),
    );
    note(
        "SetGroupLocked",
        groups.set_group_locked(toggle()).await.map(drop),
    );
    note(
        "SetGroupEphemeral",
        groups
            .set_group_ephemeral(pb::GroupEphemeralRequest {
                account: a(),
                group_jid: group(),
                expiration_seconds: 86_400,
            })
            .await
            .map(drop),
    );
    note(
        "PreviewInvite",
        groups
            .preview_invite(pb::PreviewInviteRequest {
                account: a(),
                code: INVITE_CODE.into(),
            })
            .await
            .map(drop),
    );
    note(
        "SetGroupPhoto",
        groups
            .set_group_photo(pb::SetGroupPhotoRequest {
                account: a(),
                group_jid: group(),
                image: vec![0xff, 0xd8, 0xff],
            })
            .await
            .map(drop),
    );
    note(
        "GetMembershipRequests",
        groups.get_membership_requests(gref()).await.map(drop),
    );
    note(
        "ApproveMembershipRequests",
        groups.approve_membership_requests(parts()).await.map(drop),
    );
    note(
        "RejectMembershipRequests",
        groups.reject_membership_requests(parts()).await.map(drop),
    );
    out
}
