//! A WhatsApp peer the socket suites can send to and read back (#71).
//!
//! Before this, every encrypted send through the mock failed: the client asks
//! the server for the recipient's devices and prekey bundle, the mock had none
//! to give, and no `<message>` ever left. A `MockPeer` is the other end: a
//! `whatsapp_rust::Client` over an in-memory store that never connects, so it
//! has a real Signal identity and signed prekey. `MockWaServer::serve_peer`
//! hands its bundle to the client under test, and the test opens what the
//! client sent with the peer's own session, as the recipient's phone would.
//! No crypto is written here: bundle, decrypt and unpadding are the library's.

use std::sync::Arc;

use anyhow::{Context, anyhow};
use wacore::iq::prekeys::PreKeyBundleUserNode;
use wacore::libsignal::protocol::{IdentityKey, PreKeyBundle};
use wacore::message_processing::EncType;
use wacore::protocol::ProtocolNode;
use wacore::store::in_memory::InMemoryBackend;
use wacore_binary::Node;
use whatsapp_rust::bot::Bot;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid, TokioRuntime};
use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
use whatsapp_rust_ureq_http_client::UreqHttpClient;

use super::node_read::{bytes_of, children_named, jid_attr, str_attr};

/// One peer user, device 0 only. Cheap to clone: clones share the session
/// state, so a `pkmsg` opened through one clone sets up the `msg` after it.
#[derive(Clone)]
pub struct MockPeer {
    pn: Jid,
    lid: Jid,
    client: Arc<Client>,
}

impl MockPeer {
    /// A peer whose phone number user is `pn_user` and LID user `lid_user`
    /// (bare user parts, e.g. `"5511900000002"`, `"100000000000002"`), with a
    /// fresh identity and signed prekey on device 0.
    pub async fn new(pn_user: &str, lid_user: &str) -> anyhow::Result<Self> {
        // `build` only assembles the client; `run` is what connects, and it
        // is never called, so the peer touches no socket (#71). `Device::new`
        // inside the persistence manager is where the identity comes from.
        let bot = Bot::builder()
            .with_backend(InMemoryBackend::new())
            .with_transport_factory(TokioWebSocketTransportFactory::new())
            .with_http_client(UreqHttpClient::new())
            .with_runtime(TokioRuntime)
            .build()
            .await
            .map_err(|e| anyhow!("build the peer client: {e}"))?;
        Ok(Self {
            pn: Jid::pn(pn_user),
            lid: Jid::lid(lid_user),
            client: bot.client(),
        })
    }

    /// `<pn_user>@s.whatsapp.net`.
    pub fn pn(&self) -> Jid {
        self.pn.clone()
    }

    /// `<lid_user>@lid`.
    pub fn lid(&self) -> Jid {
        self.lid.clone()
    }

    /// The `<user>` child of a prekey fetch answer (`<iq xmlns="encrypt"
    /// type="get">`) for this peer's device 0, under `jid`, the address the
    /// client asked for (PN or LID).
    pub fn prekey_user(&self, jid: &Jid) -> anyhow::Result<Node> {
        // The bundle is the device's real published material (the recipe of
        // the library's own `establish_acknowledged_session`, bench_support.rs),
        // so what the client encrypts to it is decryptable here. No one-time
        // prekey: `PreKeyBundle` allows `None`.
        let device = self.client.persistence_manager().get_device_snapshot();
        let bundle = PreKeyBundle::new(
            device.registration_id,
            0u32.into(),
            None,
            device.signed_pre_key_id.into(),
            device.signed_pre_key.public_key,
            device.signed_pre_key_signature,
            IdentityKey::new(device.identity_key.public_key),
        )
        .context("build the peer prekey bundle")?;
        let node = PreKeyBundleUserNode::from_bundle(jid.clone(), &bundle, None)
            .context("render the peer prekey bundle")?;
        Ok(node.into_node())
    }

    /// What this peer's device 0 received in `message`, a `<message>` the
    /// client sent: the `<enc>` under the `<to>` that names this peer (by PN or
    /// LID), or the message's own `<enc>` when it carries no `<participants>`.
    /// `sender` is the address the client sent from (its own device jid).
    /// Returns the unpadded `wa.Message`.
    pub async fn open_dm(&self, message: &Node, sender: &Jid) -> anyhow::Result<wa::Message> {
        let enc = self
            .own_enc(message)
            .with_context(|| format!("no <enc> for {} in the message", self.pn))?;
        self.open_pairwise(enc, sender).await
    }

    /// A group or status `<message>`: open this peer's `<to>` as `open_dm`
    /// does, process the sender key distribution it carries for the group the
    /// message is addressed to, then decrypt the message's `skmsg` from
    /// `sender`. Returns the unpadded `wa.Message` of the skmsg. A later
    /// message to the same group may carry no `<to>` for this peer (the key
    /// was already distributed): then only the skmsg is decrypted, with the
    /// sender key an earlier call stored.
    pub async fn open_group(&self, message: &Node, sender: &Jid) -> anyhow::Result<wa::Message> {
        let group = jid_attr(message, "to").context("group message without a `to` jid")?;
        if let Some(enc) = self.own_enc(message) {
            self.take_sender_key(enc, &group, sender).await?;
        }
        let skmsg = children_named(message, "enc")
            .find(|enc| str_attr(enc, "type").as_deref() == Some("skmsg"))
            .context("group message without a skmsg <enc>")?;
        let padded = self
            .client
            .signal()
            .decrypt_group_message(&group, sender, bytes_of(skmsg)?)
            .await
            .context("decrypt the skmsg")?;
        decode_padded(&padded, skmsg)
    }

    /// The `<enc>` under the `<to>` naming this peer's device 0, if any.
    fn own_enc<'a>(&self, message: &'a Node) -> Option<&'a Node> {
        let Some(participants) = message.get_optional_child("participants") else {
            return message.get_optional_child("enc");
        };
        children_named(participants, "to")
            .find(|to| self.names_device_zero(to))
            .and_then(|to| to.get_optional_child("enc"))
    }

    fn names_device_zero(&self, to: &Node) -> bool {
        jid_attr(to, "jid").is_some_and(|jid| {
            jid.device == 0 && (jid.user == self.pn.user || jid.user == self.lid.user)
        })
    }

    async fn open_pairwise(&self, enc: &Node, sender: &Jid) -> anyhow::Result<wa::Message> {
        let kind = enc_type_of(enc)?;
        let padded = self
            .client
            .signal()
            .decrypt_message(sender, kind, bytes_of(enc)?)
            .await
            .with_context(|| format!("decrypt a pairwise <enc> from {sender}"))?;
        decode_padded(&padded, enc)
    }

    /// Open the pairwise `<enc>` that carries the sender key distribution and
    /// store the key under `group`, so the skmsg that follows can be read.
    async fn take_sender_key(&self, enc: &Node, group: &Jid, sender: &Jid) -> anyhow::Result<()> {
        let carrier = self.open_pairwise(enc, sender).await?;
        let skdm = carrier
            .sender_key_distribution_message
            .into_option()
            .and_then(|skdm| skdm.axolotl_sender_key_distribution_message)
            .context("the pairwise message carries no sender key distribution")?;
        self.client
            .signal()
            .process_sender_key_distribution(group, sender, &skdm)
            .await
            .context("store the sender key")
    }
}

fn enc_type_of(enc: &Node) -> anyhow::Result<EncType> {
    let wire = str_attr(enc, "type").context("<enc> without a type")?;
    EncType::from_wire(&wire).ok_or_else(|| anyhow!("unknown <enc type={wire}>"))
}

/// Unpad with the `<enc v>` the sender wrote and decode the `wa.Message`.
fn decode_padded(padded: &[u8], enc: &Node) -> anyhow::Result<wa::Message> {
    let version: u8 = str_attr(enc, "v")
        .map(|v| v.parse())
        .transpose()
        .context("<enc v> is not a number")?
        .unwrap_or(2);
    wacore::messages::decode_plaintext(padded, version).context("unpad and decode the message")
}
