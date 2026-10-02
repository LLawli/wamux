//! The companion account behind a real socket, the peers it sends to, and the
//! calls every MessagingService test shares.

use std::time::Duration;

use tonic::transport::Channel;
use wacore_binary::Node;
use wamux::config::Config;
use wamux::proto::v1 as pb;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux::stress::app_state_fixture::{OpenedPatch, open_app_state_patch};
use wamux::stress::{MockCdn, MockIdentity, MockPeer, MockWaServer};
use whatsapp_rust::Jid;

use crate::common::mock_wire::{account_ref, attr, media_conn, operation};
use crate::common::{self, LoggedIn};

/// The one person the DM tests write to.
pub const PEER_PN_USER: &str = "5511900000002";
pub const PEER_LID_USER: &str = "100000000000002";
/// A second person: the other group member and status recipient.
pub const OTHER_PN_USER: &str = "5511900000003";
pub const OTHER_LID_USER: &str = "100000000000003";
/// The account itself: its phone is device 0, the client under test device 7.
pub const OWN_PN_USER: &str = "5511999999999";
pub const OWN_LID_USER: &str = "100000000000099";
pub const OWN_LID: &str = "100000000000099:7@lid";
pub const GROUP: &str = "120363000000000071@g.us";
/// `media_max_bytes` for this suite: small, so the cut-off is cheap to cross.
pub const MEDIA_LIMIT: u64 = 256 * 1024;

pub const APP_STATE: &str = "w:sync:app:state";

/// The daemon serving one logged-in companion, with three peers served.
pub struct Fixture {
    pub mock: MockWaServer,
    pub cdn: MockCdn,
    pub logged: LoggedIn,
    pub messages: MessagingServiceClient<Channel>,
    pub account: pb::AccountRef,
    pub peer: MockPeer,
    pub other: MockPeer,
    /// The account's own phone.
    pub phone: MockPeer,
}

pub async fn fixture(test: &str) -> Fixture {
    let mock = MockWaServer::start_as(MockIdentity {
        lid: OWN_LID.to_string(),
    })
    .await
    .expect("start mock");
    let cdn = MockCdn::start().await.expect("start cdn");
    mock.answer_iq("w:m", "set", "media_conn", media_conn(&cdn.host()));
    let peer = MockPeer::new(PEER_PN_USER, PEER_LID_USER)
        .await
        .expect("peer");
    let other = MockPeer::new(OTHER_PN_USER, OTHER_LID_USER)
        .await
        .expect("other");
    let phone = MockPeer::new(OWN_PN_USER, OWN_LID_USER)
        .await
        .expect("own phone");
    for served in [&peer, &other, &phone] {
        mock.serve_peer(served);
    }
    mock.serve_group(GROUP, &[&peer, &other]);
    let prefix = common::test_prefix("messaging_service", test);
    let logged = common::logged_in_companion(&mock, &prefix).await;
    await_login_presence(&mock).await;
    let config = Config {
        media_max_bytes: MEDIA_LIMIT,
        ..Config::default()
    };
    let channel = common::serve_registry_with(logged.registry.clone(), config).await;
    let account = account_ref(&logged.handle.uuid.to_string());
    Fixture {
        mock,
        cdn,
        logged,
        messages: MessagingServiceClient::new(channel),
        account,
        peer,
        other,
        phone,
    }
}

impl Fixture {
    pub async fn cleanup(self) {
        self.logged.cleanup().await;
    }

    pub fn a(&self) -> Option<pb::AccountRef> {
        Some(self.account.clone())
    }
}

/// The address the client sends from, which is also the key a peer stores its
/// session with this account under. Any fixed address would do for opening a
/// message; this is the one the client's own device has.
pub fn sender() -> Jid {
    OWN_LID.parse().expect("own lid jid")
}

pub fn jid(value: impl ToString) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

pub fn key(remote_jid: impl ToString, id: &str) -> pb::MessageKey {
    pb::MessageKey {
        remote_jid: remote_jid.to_string(),
        id: id.to_string(),
        from_me: true,
        participant: String::new(),
    }
}

/// A plain text to `to`, as `SendText` takes it.
pub fn text(f: &Fixture, to: impl ToString, body: &str) -> pb::SendTextRequest {
    pb::SendTextRequest {
        account: f.a(),
        to: jid(to),
        text: body.to_string(),
        ..Default::default()
    }
}

/// Send `body` to `to` and return the key the RPC answered with, once its
/// stanza has reached the mock. The RPC returns when the client has written
/// the frame, a beat before the mock reads it, so a count taken right after
/// the RPC could miss it (#71).
pub async fn send_text(f: &mut Fixture, to: impl ToString, body: &str) -> pb::MessageKey {
    let request = text(f, to, body);
    let sent = f
        .messages
        .send_text(request)
        .await
        .expect("send text")
        .into_inner()
        .key
        .expect("a send answers with its key");
    crate::common::mock_wire::sent_message(&f.mock, &sent.id).await;
    sent
}

/// Wait for the `<presence type="available">` the library sends on its own
/// once offline delivery ends (`send_automatic_available`, whatsapp-rust
/// node_io.rs:1492). No RPC asked for it and it arrives on the library's
/// clock, so the fixture lets it land before any test counts presences.
async fn await_login_presence(mock: &MockWaServer) {
    common::poll_until(
        "the login <presence type=available>",
        Duration::from_secs(5),
        || async { (!mock.client_stanzas("presence").is_empty()).then_some(()) },
    )
    .await;
}

/// Send `body` to the peer and return the key the RPC answered with.
pub async fn send_to_peer(f: &mut Fixture, body: &str) -> pb::MessageKey {
    let to = f.peer.pn();
    send_text(f, to, body).await
}

/// The newest `<iq xmlns="w:sync:app:state" type="set">` that carries a patch
/// (the library also sends patch-less syncs), opened with the seeded key.
pub async fn sent_patch(mock: &MockWaServer, after: usize) -> OpenedPatch {
    let iq = common::poll_until(
        "an app-state patch on the wire",
        Duration::from_secs(5),
        || async { patch_iqs(mock).into_iter().nth(after) },
    )
    .await;
    open_app_state_patch(&iq).expect("open the patch")
}

/// How many app-state patches the client has sent so far.
pub fn patches_sent(mock: &MockWaServer) -> usize {
    patch_iqs(mock).len()
}

fn patch_iqs(mock: &MockWaServer) -> Vec<Node> {
    mock.client_iqs()
        .into_iter()
        .filter(|iq| attr(iq, "xmlns").as_deref() == Some(APP_STATE))
        .filter(carries_patch)
        .collect()
}

fn carries_patch(iq: &Node) -> bool {
    operation(iq)
        .and_then(|sync| sync.get_optional_child_by_tag(&["collection", "patch"]))
        .is_some()
}

/// The newest stanza with `tag` the client sent, after `before` of them.
pub async fn newest_stanza(mock: &MockWaServer, tag: &str, before: usize) -> Node {
    common::poll_until(
        &format!("a new <{tag}> on the wire"),
        Duration::from_secs(5),
        || async {
            let all = mock.client_stanzas(tag);
            (all.len() > before).then(|| all.last().cloned()).flatten()
        },
    )
    .await
}

/// Everything a refused call could have put on the wire, counted: what the
/// "nothing reached the wire" tests compare before and after.
#[derive(Debug, PartialEq, Eq)]
pub struct WireCount {
    pub messages: usize,
    pub receipts: usize,
    pub presences: usize,
    pub chatstates: usize,
    pub patches: usize,
    pub uploads: usize,
}

pub fn wire_count(f: &Fixture) -> WireCount {
    WireCount {
        messages: f.mock.client_messages().len(),
        receipts: f.mock.client_stanzas("receipt").len(),
        presences: f.mock.client_stanzas("presence").len(),
        chatstates: f.mock.client_stanzas("chatstate").len(),
        patches: patches_sent(&f.mock),
        uploads: f.cdn.uploads().len(),
    }
}

/// The positive signal for "nothing reached the wire" (#67): a valid send made
/// after the refused calls. The websocket is ordered, so once it lands,
/// anything the refused calls had sent would be there too, and the probe must
/// be the only new stanza of any kind.
pub async fn assert_only_the_probe_was_sent(f: &mut Fixture, before: WireCount) {
    let probe = send_to_peer(f, "probe").await;
    crate::common::mock_wire::sent_message(&f.mock, &probe.id).await;
    let after = wire_count(f);
    assert_eq!(
        after,
        WireCount {
            messages: before.messages + 1,
            ..before
        },
        "only the probe reached the wire"
    );
}
