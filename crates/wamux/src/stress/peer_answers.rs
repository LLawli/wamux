//! What the mock answers so a client can send to a `MockPeer` (#71): who the
//! client is (`<success lid>`), that offline delivery is over, the peer's
//! devices and prekey bundle, a group's members. Also records the stanzas a
//! send leaves besides `<message>` and `<iq>`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use wacore_binary::Node;
use wacore_binary::builder::NodeBuilder;
use whatsapp_rust::Jid;

use super::iq_table::{ComputedAnswer, IqAnswer};
use super::mock_peer::MockPeer;
use super::mock_wa_server::MockWaServer;
use super::node_read::{children_named, jid_attr, str_attr};

/// Who the client under test is, as `<success>` tells it.
#[derive(Clone, Debug)]
pub struct MockIdentity {
    /// The client's own LID device jid, e.g. `"100000000000099:7@lid"`, sent
    /// as `<success lid>`. Group and status sends refuse without one.
    pub lid: String,
}

/// A group the mock answers `w:g2` metadata for.
struct ServedGroup {
    jid: String,
    members: Vec<MockPeer>,
}

/// The peers and groups served so far; read by the computed IQ answers.
#[derive(Default)]
struct Directory {
    peers: Vec<MockPeer>,
    groups: Vec<ServedGroup>,
}

/// Everything the peer-aware mock shares with its connection tasks: who the
/// client is, who is served, and the stanzas recorded. A server from plain
/// `start()` has an empty one and behaves as it always did.
#[derive(Default)]
pub(super) struct PeerWorld {
    own_lid: Option<Jid>,
    directory: std::sync::Mutex<Directory>,
    stanzas: std::sync::Mutex<Vec<Node>>,
    rules_installed: AtomicBool,
}

impl PeerWorld {
    fn as_identity(identity: &MockIdentity) -> anyhow::Result<Self> {
        let own_lid: Jid = identity
            .lid
            .parse()
            .with_context(|| format!("MockIdentity::lid {:?} is not a jid", identity.lid))?;
        Ok(Self {
            own_lid: Some(own_lid),
            ..Self::default()
        })
    }

    /// The `lid` a `<success>` carries, when the mock was started `as` someone.
    pub(super) fn success_lid(&self) -> Option<&Jid> {
        self.own_lid.as_ref()
    }

    /// Whether `tag` is a stanza `client_stanzas` records (`<message>` and
    /// `<iq>` have their own lists).
    pub(super) fn records(tag: &str) -> bool {
        matches!(tag, "receipt" | "presence" | "chatstate")
    }

    /// Keep a stanza for `client_stanzas`.
    pub(super) fn record(&self, node: Node) {
        // Poisoning needs a panic while the lock is held; nothing here panics.
        if let Ok(mut stanzas) = self.stanzas.lock() {
            stanzas.push(node);
        }
    }

    /// The client's `<iq xmlns="passive"><active/></iq>` is what starts its wait
    /// for offline delivery to end, so a mock started `as` an identity ends it
    /// by sending `<ib><offline count="0"/></ib>` after the answer
    /// (CallFixture, whatsapp-rust src/test_support/call.rs:530-546).
    pub(super) fn ends_offline_after(&self, iq: &Node) -> bool {
        self.own_lid.is_some()
            && str_attr(iq, "xmlns").as_deref() == Some("passive")
            && iq.get_optional_child("active").is_some()
    }

    /// One `<notification type="contacts"><modify>` per served peer, carrying
    /// its PN and LID. The library resolves a status recipient given by phone
    /// number to a LID from what it already knows (`resolve_recipient_to_lid`,
    /// whatsapp-rust src/client/lid_pn.rs:1234: the local store, no IQ), and a
    /// recipient it cannot resolve is dropped silently. This is the stanza the
    /// server uses to hand a client a contact's LID outside any usync
    /// (`handle_contacts_notification`, src/handlers/notification/profile.rs:
    /// both pairs, `old` = `new`, so nothing "changes"). Side effect: the client
    /// also dispatches a `ContactNumberChanged` event with old == new. Sent at
    /// login, so only peers served before the client connects are known to it.
    pub(super) fn contact_notifications(&self) -> Vec<Node> {
        let Ok(directory) = self.directory.lock() else {
            return Vec::new();
        };
        directory.peers.iter().map(contact_notification).collect()
    }

    fn peer_for(&self, jid: &Jid) -> Option<MockPeer> {
        let directory = self.directory.lock().ok()?;
        directory
            .peers
            .iter()
            .find(|peer| names_user(peer, jid))
            .cloned()
    }
}

/// Whether `jid` is `peer`'s user, by PN or by LID, on any of its devices.
fn names_user(peer: &MockPeer, jid: &Jid) -> bool {
    let (pn, lid) = (peer.pn(), peer.lid());
    (jid.server == pn.server && jid.user == pn.user)
        || (jid.server == lid.server && jid.user == lid.user)
}

fn contact_notification(peer: &MockPeer) -> Node {
    let (pn, lid) = (peer.pn(), peer.lid());
    let modify = NodeBuilder::new("modify")
        .attr("old", &pn)
        .attr("new", &pn)
        .attr("old_lid", &lid)
        .attr("new_lid", &lid)
        .build();
    NodeBuilder::new("notification")
        .attr("type", "contacts")
        .attr("from", "s.whatsapp.net")
        .attr("id", format!("mock-contact-{}", pn.user))
        .children([modify])
        .build()
}

/// `<ib><offline count="0"/></ib>`: the server saying offline delivery is over.
pub(super) fn offline_over() -> Node {
    NodeBuilder::new("ib")
        .children([NodeBuilder::new("offline").attr("count", "0").build()])
        .build()
}

impl MockWaServer {
    /// Like `start`, but `<success>` carries `identity.lid`, and the mock
    /// answers the client's `<active>` IQ and then sends
    /// `<ib><offline count="0"/></ib>`. Without that, the library holds every
    /// first encrypted send for its 60 s offline-delivery timeout
    /// (`ensure_e2e_sessions_resolved`). `start()` is unchanged.
    pub async fn start_as(identity: MockIdentity) -> anyhow::Result<Self> {
        Self::start_with(PeerWorld::as_identity(&identity)?).await
    }

    /// Answer, for `peer`'s user by PN and by LID: the usync device query
    /// (`<devices>`: device 0), the usync LID query (`<lid val>`), and the
    /// prekey fetch (`MockPeer::prekey_user`). Answers depend on the jids the
    /// IQ asks for, so several peers (and the client's own phone, served as a
    /// peer) coexist; a jid no peer serves gets no `<user>` back.
    pub fn serve_peer(&self, peer: &MockPeer) {
        self.install_peer_rules();
        // Poisoning needs a panic while the lock is held; nothing here panics.
        if let Ok(mut directory) = self.world().directory.lock() {
            directory.peers.push(peer.clone());
        }
    }

    /// Answer the `w:g2` metadata query for `group` (e.g.
    /// `"120363000000000071@g.us"`) with a LID-addressed group whose members
    /// are the client (its `MockIdentity::lid`, as admin) and `members`.
    pub fn serve_group(&self, group: &str, members: &[&MockPeer]) {
        self.install_peer_rules();
        let served = ServedGroup {
            jid: group.to_string(),
            members: members.iter().map(|peer| (*peer).clone()).collect(),
        };
        // Poisoning needs a panic while the lock is held; nothing here panics.
        if let Ok(mut directory) = self.world().directory.lock() {
            directory.groups.push(served);
        }
    }

    /// Every stanza with this tag a client sent after login, oldest first:
    /// `"receipt"`, `"presence"`, `"chatstate"` (and `"message"`/`"iq"`, the
    /// same as `client_messages`/`client_iqs`).
    pub fn client_stanzas(&self, tag: &str) -> Vec<Node> {
        match tag {
            "message" => self.client_messages(),
            "iq" => self.client_iqs(),
            _ => self
                .world()
                .stanzas
                .lock()
                .map(|all| all.iter().filter(|n| n.tag == tag).cloned().collect())
                .unwrap_or_default(),
        }
    }

    /// Register the three computed answers once, on the first peer or group.
    /// Not at start: a plain `start()` server must keep answering as before.
    fn install_peer_rules(&self) {
        if self.world().rules_installed.swap(true, Ordering::SeqCst) {
            return;
        }
        let rules: [(&str, &str, ComputedAnswer); 3] = [
            ("usync", "usync", computed(self.world(), usync_answer)),
            ("encrypt", "key", computed(self.world(), prekey_answer)),
            ("w:g2", "query", computed(self.world(), group_answer)),
        ];
        for (xmlns, child, answer) in rules {
            self.push_rule(xmlns, "get", child, IqAnswer::Computed(answer));
        }
    }
}

fn computed(
    world: &Arc<PeerWorld>,
    build: fn(&PeerWorld, &Node) -> Option<Node>,
) -> ComputedAnswer {
    let world = world.clone();
    Arc::new(move |request: &Node| build(&world, request))
}

/// `<usync><list><user jid>..</user></list></usync>` for the users asked about
/// (the shape of `build_usync_response`, wacore/src/usync.rs:107-153): the
/// device list (device 0 only, so no key-index is needed) when the query asks
/// for `<devices>`, the LID mapping when it asks for `<lid>`.
fn usync_answer(world: &PeerWorld, request: &Node) -> Option<Node> {
    let usync = request.get_optional_child("usync")?;
    let query = usync.get_optional_child("query")?;
    let asks = |tag: &str| query.get_optional_child(tag).is_some();
    let users: Vec<Node> = children_named(usync.get_optional_child("list")?, "user")
        .filter_map(|user| jid_attr(user, "jid"))
        .filter_map(|jid| Some((world.peer_for(&jid)?, jid)))
        .map(|(peer, jid)| usync_user(&peer, &jid, asks("devices"), asks("lid")))
        .collect();
    let list = NodeBuilder::new("list").children(users).build();
    Some(NodeBuilder::new("usync").children([list]).build())
}

fn usync_user(peer: &MockPeer, jid: &Jid, devices: bool, lid: bool) -> Node {
    let mut children = Vec::new();
    if devices {
        let device = NodeBuilder::new("device").attr("id", 0u16).build();
        let list = NodeBuilder::new("device-list").children([device]).build();
        children.push(NodeBuilder::new("devices").children([list]).build());
    }
    if lid {
        children.push(NodeBuilder::new("lid").attr("val", peer.lid()).build());
    }
    NodeBuilder::new("user")
        .attr("jid", jid)
        .children(children)
        .build()
}

/// `<list><user jid>..bundle..</user></list>` for the devices asked about
/// (wacore/src/prekeys.rs:196). Only device 0 of a served peer has a bundle;
/// any other jid is left out, which the library reads as "no bundle".
fn prekey_answer(world: &PeerWorld, request: &Node) -> Option<Node> {
    let key = request.get_optional_child("key")?;
    let users: Vec<Node> = children_named(key, "user")
        .filter_map(|user| jid_attr(user, "jid"))
        .filter(|jid| jid.device == 0)
        .filter_map(|jid| world.peer_for(&jid)?.prekey_user(&jid).ok())
        .collect();
    Some(NodeBuilder::new("list").children(users).build())
}

/// The `<group>` of a `w:g2` metadata answer, LID addressed, in the shape of
/// the 2026-10-02 capture (tests/group_service/captured.rs): the client as
/// superadmin, then each member with its phone number.
fn group_answer(world: &PeerWorld, request: &Node) -> Option<Node> {
    let to = str_attr(request, "to")?;
    let directory = world.directory.lock().ok()?;
    let group = directory.groups.iter().find(|group| group.jid == to)?;
    let own = world.own_lid.as_ref()?.to_non_ad();
    let mut roster = vec![participant(&own, None, Some("superadmin"))];
    roster.extend(
        group
            .members
            .iter()
            .map(|peer| participant(&peer.lid(), Some(&peer.pn()), None)),
    );
    let id = to.split('@').next().unwrap_or_default();
    Some(group_node(id, &own, roster))
}

fn participant(lid: &Jid, pn: Option<&Jid>, role: Option<&str>) -> Node {
    let mut builder = NodeBuilder::new("participant").attr("jid", lid);
    if let Some(pn) = pn {
        builder = builder.attr("phone_number", pn);
    }
    if let Some(role) = role {
        builder = builder.attr("type", role);
    }
    builder.build()
}

fn group_node(id: &str, creator: &Jid, roster: Vec<Node>) -> Node {
    const CREATION: &str = "1790947599";
    NodeBuilder::new("group")
        .attr("addressing_mode", "lid")
        .attr("subject", "wamux mock group")
        .attr("id", id)
        .attr("creator", creator)
        .attr("creation", CREATION)
        .attr("s_t", CREATION)
        .attr("size", roster.len().to_string())
        .children(roster)
        .build()
}
