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

use anyhow::{Context, anyhow};
use bytes::{Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_websockets::{Message, ServerBuilder};
use wacore::framing::{FrameDecoder, encode_frame};
use wacore::store::Device;
use wacore_binary::builder::NodeBuilder;
use wacore_binary::consts::{NOISE_PATTERN_XX, WA_CONN_HEADER};
use wacore_binary::encoder::EncodeNode;
use wacore_binary::marshal::{marshal, unmarshal_ref};
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
    accept_task: tokio::task::JoinHandle<()>,
}

impl Drop for MockWaServer {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

impl MockWaServer {
    /// Bind `127.0.0.1:0` and start accepting. Returns once the listener is up.
    pub async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind mock server")?;
        let addr = listener.local_addr().context("local_addr")?;
        let handshakes = Arc::new(AtomicUsize::new(0));
        let post_login_frames = Arc::new(AtomicUsize::new(0));
        let parsed_nodes = Arc::new(AtomicUsize::new(0));
        let keepalive_pings = Arc::new(AtomicUsize::new(0));

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
            accept_task,
        })
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

/// The per-connection counters handed to each spawned `serve_connection`.
struct ConnCounters {
    handshakes: Arc<AtomicUsize>,
    post_login_frames: Arc<AtomicUsize>,
    parsed_nodes: Arc<AtomicUsize>,
    keepalive_pings: Arc<AtomicUsize>,
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
    let ConnCounters {
        handshakes: counter,
        post_login_frames: post_login,
        parsed_nodes,
        keepalive_pings,
    } = counters;
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
    let mut recv_ctr: u32 = 0;
    let success = NodeBuilder::new("success")
        .attr("t", now_secs().to_string())
        .build();
    let success_plain = marshal(&success).map_err(|e| anyhow!("marshal success: {e}"))?;
    let success_ct = send_cipher
        .encrypt_with_counter(send_ctr, &success_plain)
        .map_err(|e| anyhow!("encrypt success: {e}"))?;
    send_ctr += 1;
    send_frame(&mut ws, &success_ct).await?;

    // -> push one <receipt>: a server-originated stanza that needs no Signal
    // session, so a logged-in client decodes it and dispatches Event::Receipt
    // straight into wamux's pipeline. Proves "pushed stanza -> event" (M2b).
    let receipt = NodeBuilder::new("receipt")
        .attr("from", PUSHED_RECEIPT_FROM)
        .attr("id", PUSHED_RECEIPT_ID)
        .attr("type", "delivery")
        .attr("t", now_secs().to_string())
        .build();
    let receipt_plain = marshal(&receipt).map_err(|e| anyhow!("marshal receipt: {e}"))?;
    let receipt_ct = send_cipher
        .encrypt_with_counter(send_ctr, &receipt_plain)
        .map_err(|e| anyhow!("encrypt receipt: {e}"))?;
    send_ctr += 1;
    send_frame(&mut ws, &receipt_ct).await?;

    // Read the client's encrypted post-login frames. Decrypting even one proves
    // the transport works both ways and the client logged in. We reply a minimal
    // <iq type=result> to any <iq> so keepalive/login IQs don't immediately fail.
    loop {
        let frame = match next_frame(&mut ws, &mut decoder, &mut first_frame).await {
            Ok(f) => f,
            Err(_) => break,
        };
        let mut buf = frame.to_vec();
        if recv_cipher
            .decrypt_in_place_with_counter(recv_ctr, &mut buf)
            .is_err()
        {
            tracing::warn!("post-login decrypt failed (counter desync?)");
            break;
        }
        recv_ctr += 1;
        post_login.fetch_add(1, Ordering::SeqCst);

        // The decrypted payload is `[flag_byte][node]` (flag & 2 => zlib), the
        // same envelope `wacore_binary::Encoder` writes on send. Without this
        // unpack the leading flag byte derails `unmarshal_ref`, so the client's
        // post-login IQs (e.g. usync) go unanswered, the waiter stays pending,
        // and the keepalive loop skips its ping forever (#stress-m2b).
        let payload = match wacore_binary::util::unpack(&buf) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "post-login unpack failed");
                continue;
            }
        };

        if let Ok(node) = unmarshal_ref(&payload) {
            parsed_nodes.fetch_add(1, Ordering::SeqCst);
            let tag = node.tag();
            let xmlns_dbg = node.get_attr("xmlns").map(|v| v.to_string());
            let type_dbg = node.get_attr("type").map(|v| v.to_string());
            tracing::debug!(%tag, ?xmlns_dbg, ?type_dbg, "post-login node from client");
            // Keepalive pings are `<iq xmlns="w:p" type="get">` (wacore
            // KeepaliveSpec). Counting them proves the connection survived long
            // enough for the client's 15-30 s keepalive loop to fire.
            if tag == "iq"
                && node.get_attr("xmlns").map(|v| v.to_string()).as_deref() == Some("w:p")
            {
                keepalive_pings.fetch_add(1, Ordering::SeqCst);
            }
            if tag == "iq"
                && let Some(id) = node.get_attr("id").map(|v| v.to_string())
            {
                let reply = NodeBuilder::new("iq")
                    .attr("type", "result")
                    .attr("id", id)
                    .build();
                if let Ok(plain) = marshal(&reply)
                    && let Ok(ct) = send_cipher.encrypt_with_counter(send_ctr, &plain)
                {
                    send_ctr += 1;
                    let _ = send_frame(&mut ws, &ct).await;
                }
            }
        }
    }
    Ok(())
}

/// Pull the next WhatsApp frame, stripping the 4-byte `WA_CONN_HEADER` that
/// prefixes only the very first client frame.
async fn next_frame<S>(
    ws: &mut tokio_websockets::WebSocketStream<S>,
    decoder: &mut FrameDecoder,
    first_frame: &mut bool,
) -> anyhow::Result<BytesMut>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        if let Some(frame) = decoder.decode_frame() {
            return Ok(frame);
        }
        let msg = ws
            .next()
            .await
            .ok_or_else(|| anyhow!("connection closed before frame"))?
            .context("ws read")?;
        if !msg.is_binary() {
            continue;
        }
        let payload = msg.into_payload();
        let bytes: &[u8] = payload.as_ref();
        if *first_frame {
            // First frame carries the WA connection header before the length.
            *first_frame = false;
            decoder.feed(&bytes[WA_CONN_HEADER.len()..]);
        } else {
            decoder.feed(bytes);
        }
    }
}

/// Frame a payload (3-byte length prefix) and send it as one WS binary message.
async fn send_frame<S>(
    ws: &mut tokio_websockets::WebSocketStream<S>,
    payload: &[u8],
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let framed = encode_frame(payload, None).map_err(|e| anyhow!("encode frame: {e}"))?;
    ws.send(Message::binary(Bytes::from(framed)))
        .await
        .context("ws send")?;
    Ok(())
}
