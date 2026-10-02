//! Mock WhatsApp WSS server: speaks the server side of the Noise XX handshake
//! so a real `whatsapp-rust` Client connects, then (M2) pushes data stanzas.
//!
//! The server is the mirror of the client's XX recipe (`wacore-noise`
//! `XxHandshakeState`): same MixHash order, same DH mixes. We reuse the crate's
//! own primitives (`NoiseHandshake`, `build_cert_chain_bytes`, `framing`) so the
//! crypto stays bug-for-bug compatible with the client.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::iq_table::{IqAnswer, SharedIqAnswers};
use super::peer_answers::PeerWorld;
use super::post_login::{ConnCounters, Session, read_client_frames};
use super::wire_frames::{next_frame, send_frame, send_node};
use anyhow::{Context, anyhow};
use tokio::net::{TcpListener, TcpStream};
use tokio_websockets::ServerBuilder;
use wacore::framing::FrameDecoder;
use wacore::store::Device;
use wacore_binary::builder::NodeBuilder;
use wacore_binary::consts::{NOISE_PATTERN_XX, WA_CONN_HEADER};
use wacore_noise::NoiseHandshake;
use wacore_noise::test_util::build_cert_chain_bytes;
use whatsapp_rust::buffa;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp::{self as wa, HandshakeMessage};

/// JID the mock pushes its post-login `<receipt>` from. Stable so tests can
/// assert the surfaced `ReceiptEvent` carries exactly this chat + id.
pub const PUSHED_RECEIPT_FROM: &str = "5511888888888@s.whatsapp.net";
/// Message id on the pushed `<receipt>`.
pub const PUSHED_RECEIPT_ID: &str = "WAMUX-STRESS-RCPT-1";

/// A running mock server. Drop to stop accepting (the accept task is aborted).
pub struct MockWaServer {
    pub addr: SocketAddr,
    handshakes: Arc<AtomicUsize>,
    post_login_frames: Arc<AtomicUsize>,
    parsed_nodes: Arc<AtomicUsize>,
    keepalive_pings: Arc<AtomicUsize>,
    iq_answers: SharedIqAnswers,
    client_messages: SharedClientMessages,
    client_iqs: SharedClientIqs,
    world: Arc<PeerWorld>,
    accept_task: tokio::task::JoinHandle<()>,
}

/// Every `<message>` a client sent after login, in arrival order: what a test
/// asserts a send put on the wire (#26, the channel poll vote).
pub(super) type SharedClientMessages = Arc<std::sync::Mutex<Vec<wacore_binary::Node>>>;

/// Every `<iq>` a client sent after login, in arrival order: what a write
/// (a group subject, a leave) put on the wire (#68).
pub(super) type SharedClientIqs = Arc<std::sync::Mutex<Vec<wacore_binary::Node>>>;

impl Drop for MockWaServer {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

impl MockWaServer {
    /// Bind `127.0.0.1:0` and start accepting. Returns once the listener is up.
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with(PeerWorld::default()).await
    }

    /// `start` over a given peer world: what `start_as` (#71) builds on.
    pub(super) async fn start_with(world: PeerWorld) -> anyhow::Result<Self> {
        let world = Arc::new(world);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind mock server")?;
        let addr = listener.local_addr().context("local_addr")?;
        let handshakes = Arc::new(AtomicUsize::new(0));
        let post_login_frames = Arc::new(AtomicUsize::new(0));
        let parsed_nodes = Arc::new(AtomicUsize::new(0));
        let keepalive_pings = Arc::new(AtomicUsize::new(0));
        let iq_answers: SharedIqAnswers = Arc::default();
        let client_messages: SharedClientMessages = Arc::default();
        let client_iqs: SharedClientIqs = Arc::default();

        let answers = iq_answers.clone();
        let messages = client_messages.clone();
        let iqs = client_iqs.clone();
        let shared_world = world.clone();
        let hs = handshakes.clone();
        let plf = post_login_frames.clone();
        let pn = parsed_nodes.clone();
        let kap = keepalive_pings.clone();
        let accept_task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _peer)) => {
                        let counters = ConnCounters {
                            handshakes: hs.clone(),
                            post_login_frames: plf.clone(),
                            parsed_nodes: pn.clone(),
                            keepalive_pings: kap.clone(),
                            iq_answers: answers.clone(),
                            client_messages: messages.clone(),
                            client_iqs: iqs.clone(),
                            world: shared_world.clone(),
                        };
                        tokio::spawn(async move {
                            if let Err(e) = serve_connection(stream, counters).await {
                                tracing::debug!(error = %e, "mock connection ended");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "mock accept failed");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            addr,
            handshakes,
            post_login_frames,
            parsed_nodes,
            keepalive_pings,
            iq_answers,
            client_messages,
            client_iqs,
            world,
            accept_task,
        })
    }

    /// Answer every later MEX query with this `{"data":...}` JSON, as the
    /// server would. Lets a test feed the library's own response parsing a
    /// server answer of its choosing. A shortcut over `answer_iq` (#68).
    pub fn answer_mex_with(&self, json: &str) {
        let body = NodeBuilder::new("result")
            .bytes(json.as_bytes().to_vec())
            .build();
        self.answer_iq("w:mex", "*", "*", body);
    }

    /// Answer every later `<iq xmlns="newsletter">` with `<iq type=result>`
    /// wrapping this node, as the server would. Lets a test feed the library's
    /// channel-history parser the page of its choosing (#40). A shortcut over
    /// `answer_iq` (#68).
    pub fn answer_newsletter_iq_with(&self, body: wacore_binary::Node) {
        self.answer_iq("newsletter", "*", "*", body);
    }

    /// Answer every later `<iq>` whose `xmlns`, `type` and first child tag
    /// match with `<iq type=result>` wrapping `body` (#68). `"*"` in
    /// `iq_type` or `child_tag` matches any value; an IQ without a child has
    /// tag `""`. The newest matching answer wins; an IQ nothing matches still
    /// gets a bare result, as before.
    pub fn answer_iq(
        &self,
        xmlns: &str,
        iq_type: &str,
        child_tag: &str,
        body: wacore_binary::Node,
    ) {
        self.push_rule(xmlns, iq_type, child_tag, IqAnswer::Result(Box::new(body)));
    }

    /// Like `answer_iq`, but the server refuses: `<iq type=error>` carrying
    /// `<error code=.. text=..>`, the shape the library turns into
    /// `IqError::ServerError` (#68).
    pub fn answer_iq_error(
        &self,
        xmlns: &str,
        iq_type: &str,
        child_tag: &str,
        code: u16,
        text: &str,
    ) {
        let text = text.to_string();
        self.push_rule(xmlns, iq_type, child_tag, IqAnswer::Refusal { code, text });
    }

    /// The peer world `serve_peer` and friends fill (#71).
    pub(super) fn world(&self) -> &Arc<PeerWorld> {
        &self.world
    }

    pub(super) fn push_rule(&self, xmlns: &str, iq_type: &str, child_tag: &str, answer: IqAnswer) {
        // Poisoning needs a panic while the lock is held; nothing here panics.
        if let Ok(mut answers) = self.iq_answers.lock() {
            answers.push(xmlns, iq_type, child_tag, answer);
        }
    }

    /// Every `<iq>` clients have sent after login, oldest first: what a write
    /// put on the wire (#68).
    pub fn client_iqs(&self) -> Vec<wacore_binary::Node> {
        // Poisoning needs a panic while the lock is held; nothing here panics.
        self.client_iqs
            .lock()
            .map(|iqs| iqs.clone())
            .unwrap_or_default()
    }

    /// The `<message>` stanzas clients have sent so far, oldest first.
    pub fn client_messages(&self) -> Vec<wacore_binary::Node> {
        // Poisoning needs a panic while the lock is held; nothing here panics.
        self.client_messages
            .lock()
            .map(|messages| messages.clone())
            .unwrap_or_default()
    }

    /// `ws://` URL a client transport should target.
    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    /// Count of handshakes that fully completed (ClientFinish decrypted).
    pub fn handshakes_completed(&self) -> usize {
        self.handshakes.load(Ordering::SeqCst)
    }

    /// Count of post-handshake encrypted frames decrypted from clients (proves
    /// the client accepted `<success>`, logged in, and the transport ciphers
    /// work in both directions).
    pub fn post_login_frames(&self) -> usize {
        self.post_login_frames.load(Ordering::SeqCst)
    }

    /// Count of post-login frames the mock actually **parsed** into a node (via
    /// `unpack` + `unmarshal_ref`). Distinct from `post_login_frames` (decrypt
    /// only): a non-zero value proves the wire envelope is decoded end-to-end, so
    /// the "decrypts but never parses" regression (the flag-byte bug) can't hide
    /// behind a green frame count. See B1/B4 in docs/BACKLOG.md.
    pub fn parsed_nodes(&self) -> usize {
        self.parsed_nodes.load(Ordering::SeqCst)
    }

    /// Count of client-initiated keepalive pings seen (`<iq xmlns="w:p">`). A
    /// non-zero value proves the connection stayed up long enough for the
    /// client's 15-30 s keepalive loop to fire and that the mock answered it.
    pub fn keepalive_pings(&self) -> usize {
        self.keepalive_pings.load(Ordering::SeqCst)
    }
}

/// Unix seconds (server time) for the `<success t=...>` attribute.
fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One client connection: WS upgrade, the XX responder handshake, then the
/// post-handshake `<success>` + read loop over the encrypted transport.
async fn serve_connection(stream: TcpStream, counters: ConnCounters) -> anyhow::Result<()> {
    let counter = counters.handshakes.clone();
    let world = counters.world.clone();
    let (_req, mut ws) = ServerBuilder::new()
        .accept(stream)
        .await
        .context("ws upgrade")?;

    let mut decoder = FrameDecoder::new();
    let mut first_frame = true;

    // <- ClientHello (e)
    let hello_bytes = next_frame(&mut ws, &mut decoder, &mut first_frame).await?;
    let hello =
        HandshakeMessage::decode_from_slice(&hello_bytes[..]).context("decode ClientHello")?;
    let client_ephemeral = hello
        .client_hello
        .into_option()
        .and_then(|h| h.ephemeral)
        .ok_or_else(|| anyhow!("ClientHello missing ephemeral"))?;

    // Mirror of the client's XX recipe. MixHash order MUST match the client:
    // client_e, server_e, then the encrypt/decrypt transcript.
    let mut noise = NoiseHandshake::new(NOISE_PATTERN_XX, &WA_CONN_HEADER)
        .map_err(|e| anyhow!("noise init: {e}"))?;
    noise.authenticate(&client_ephemeral);

    // Reuse the crate's own keypair generation (avoids a `rand` version clash
    // with wacore-libsignal). A mock doesn't need per-connection key secrecy.
    let server_ephemeral = Device::new().noise_key;
    let server_static = Device::new().noise_key;
    let server_ephemeral_pub: Vec<u8> = server_ephemeral.public_key.public_key_bytes().to_vec();
    let server_static_pub: Vec<u8> = server_static.public_key.public_key_bytes().to_vec();
    let server_static_arr: [u8; 32] = server_static_pub
        .as_slice()
        .try_into()
        .context("server static not 32 bytes")?;

    // -> e, ee, s, es  (ServerHello: ephemeral, enc(static), enc(cert))
    noise.authenticate(&server_ephemeral_pub);
    noise
        .mix_shared_secret(server_ephemeral.private_key.serialize(), &client_ephemeral)
        .map_err(|e| anyhow!("ee: {e}"))?;
    let enc_static = noise
        .encrypt(&server_static_pub)
        .map_err(|e| anyhow!("enc static: {e}"))?;
    noise
        .mix_shared_secret(server_static.private_key.serialize(), &client_ephemeral)
        .map_err(|e| anyhow!("es: {e}"))?;
    let cert = build_cert_chain_bytes(&server_static_arr);
    let enc_cert = noise.encrypt(&cert).map_err(|e| anyhow!("enc cert: {e}"))?;

    let server_hello = HandshakeMessage {
        server_hello: buffa::MessageField::some(wa::handshake_message::ServerHello {
            ephemeral: Some(server_ephemeral_pub),
            r#static: Some(enc_static),
            payload: Some(enc_cert),
            ..Default::default()
        }),
        ..Default::default()
    };
    send_frame(&mut ws, &server_hello.encode_to_vec()).await?;

    // <- s, se  (ClientFinish: enc(client static), enc(payload))
    let finish_bytes = next_frame(&mut ws, &mut decoder, &mut first_frame).await?;
    let finish =
        HandshakeMessage::decode_from_slice(&finish_bytes[..]).context("decode ClientFinish")?;
    let client_finish = finish
        .client_finish
        .into_option()
        .ok_or_else(|| anyhow!("missing ClientFinish"))?;
    let enc_client_static = client_finish
        .r#static
        .ok_or_else(|| anyhow!("ClientFinish missing static"))?;
    let enc_payload = client_finish
        .payload
        .ok_or_else(|| anyhow!("ClientFinish missing payload"))?;

    let client_static = noise
        .decrypt(&enc_client_static)
        .map_err(|e| anyhow!("dec client static: {e}"))?;
    noise
        .mix_shared_secret(server_ephemeral.private_key.serialize(), &client_static)
        .map_err(|e| anyhow!("se: {e}"))?;
    let client_payload = noise
        .decrypt(&enc_payload)
        .map_err(|e| anyhow!("dec payload: {e}"))?;

    // Handshake done: derive the transport ciphers. The split is from the
    // initiator's view (write = client->server, read = server->client), so the
    // server SWAPS: it sends with `read` and receives with `write`.
    let (recv_cipher, send_cipher) = noise.finish().map_err(|e| anyhow!("split: {e}"))?;
    counter.fetch_add(1, Ordering::SeqCst);
    tracing::info!(
        payload_len = client_payload.len(),
        "mock handshake complete (XX), client payload decrypted"
    );

    // -> <success>: the client treats this as login success and flips
    // is_logged_in, then sends its post-login IQs over the encrypted transport.
    let mut send_ctr: u32 = 0;
    let recv_ctr: u32 = 0;
    // #71: teach the client who the served peers are before it reports being
    // logged in; see `PeerWorld::contact_notifications`.
    for notification in world.contact_notifications() {
        send_node(&mut ws, &send_cipher, &mut send_ctr, &notification).await?;
    }
    let mut success = NodeBuilder::new("success").attr("t", now_secs().to_string());
    if let Some(lid) = world.success_lid() {
        success = success.attr("lid", lid);
    }
    send_node(&mut ws, &send_cipher, &mut send_ctr, &success.build()).await?;

    // -> push one <receipt>: a server-originated stanza that needs no Signal
    // session, so a logged-in client decodes it and dispatches Event::Receipt
    // straight into wamux's pipeline. Proves "pushed stanza -> event" (M2b).
    let receipt = NodeBuilder::new("receipt")
        .attr("from", PUSHED_RECEIPT_FROM)
        .attr("id", PUSHED_RECEIPT_ID)
        .attr("type", "delivery")
        .attr("t", now_secs().to_string())
        .build();
    send_node(&mut ws, &send_cipher, &mut send_ctr, &receipt).await?;

    let session = Session {
        ws,
        decoder,
        first_frame,
        recv_cipher,
        send_cipher,
        send_ctr,
        recv_ctr,
    };
    read_client_frames(session, counters).await;
    Ok(())
}
