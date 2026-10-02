//! The mock's read loop once a client is logged in: decrypt each frame, keep
//! what a test asserts on, answer IQs. Split out of `mock_wa_server.rs` (#71)
//! to keep that file under the size limit.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::net::TcpStream;
use wacore::framing::FrameDecoder;
use wacore_binary::encoder::EncodeNode;
use wacore_binary::marshal::unmarshal_ref;
use wacore_noise::NoiseCipher;

use super::iq_table::{self, SharedIqAnswers};
use super::mock_wa_server::{SharedClientIqs, SharedClientMessages};
use super::peer_answers::{PeerWorld, offline_over};
use super::wire_frames::{next_frame, send_node};

/// The per-connection counters handed to each spawned `serve_connection`.
pub(super) struct ConnCounters {
    pub(super) handshakes: Arc<AtomicUsize>,
    pub(super) post_login_frames: Arc<AtomicUsize>,
    pub(super) parsed_nodes: Arc<AtomicUsize>,
    pub(super) keepalive_pings: Arc<AtomicUsize>,
    pub(super) iq_answers: SharedIqAnswers,
    pub(super) client_messages: SharedClientMessages,
    pub(super) client_iqs: SharedClientIqs,
    pub(super) world: Arc<PeerWorld>,
}

/// One logged-in connection: the socket, the framing state and the transport
/// ciphers with their counters, as the handshake left them.
pub(super) struct Session {
    pub(super) ws: tokio_websockets::WebSocketStream<TcpStream>,
    pub(super) decoder: FrameDecoder,
    pub(super) first_frame: bool,
    pub(super) recv_cipher: NoiseCipher,
    pub(super) send_cipher: NoiseCipher,
    pub(super) send_ctr: u32,
    pub(super) recv_ctr: u32,
}

/// Read the client's encrypted post-login frames until the connection ends.
/// Decrypting even one proves the transport works both ways and the client
/// logged in. We reply a minimal `<iq type=result>` to any `<iq>` so
/// keepalive/login IQs don't immediately fail.
pub(super) async fn read_client_frames(mut session: Session, counters: ConnCounters) {
    loop {
        let frame = match next_frame(
            &mut session.ws,
            &mut session.decoder,
            &mut session.first_frame,
        )
        .await
        {
            Ok(frame) => frame,
            Err(_) => break,
        };
        let mut buf = frame.to_vec();
        if session
            .recv_cipher
            .decrypt_in_place_with_counter(session.recv_ctr, &mut buf)
            .is_err()
        {
            tracing::warn!("post-login decrypt failed (counter desync?)");
            break;
        }
        session.recv_ctr += 1;
        counters.post_login_frames.fetch_add(1, Ordering::SeqCst);

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
        let Ok(node) = unmarshal_ref(&payload) else {
            continue;
        };
        counters.parsed_nodes.fetch_add(1, Ordering::SeqCst);
        route_client_node(&mut session, &counters, &node).await;
    }
}

/// Count, keep and answer one client stanza.
async fn route_client_node(
    session: &mut Session,
    counters: &ConnCounters,
    node: &wacore_binary::NodeRef<'_>,
) {
    let tag = node.tag();
    let xmlns_dbg = node.get_attr("xmlns").map(|v| v.to_string());
    let type_dbg = node.get_attr("type").map(|v| v.to_string());
    tracing::debug!(%tag, ?xmlns_dbg, ?type_dbg, "post-login node from client");
    // Keepalive pings are `<iq xmlns="w:p" type="get">` (wacore KeepaliveSpec).
    // Counting them proves the connection survived long enough for the
    // client's 15-30 s keepalive loop to fire.
    if tag == "iq" && xmlns_dbg.as_deref() == Some("w:p") {
        counters.keepalive_pings.fetch_add(1, Ordering::SeqCst);
    }
    if tag == "message"
        && let Ok(mut messages) = counters.client_messages.lock()
    {
        messages.push(node.to_owned());
    }
    if tag == "iq" {
        answer_client_iq(session, counters, node).await;
    }
    if PeerWorld::records(tag) {
        counters.world.record(node.to_owned());
    }
}

async fn answer_client_iq(
    session: &mut Session,
    counters: &ConnCounters,
    node: &wacore_binary::NodeRef<'_>,
) {
    if let Ok(mut iqs) = counters.client_iqs.lock() {
        iqs.push(node.to_owned());
    }
    let cipher = &session.send_cipher;
    if let Some(reply) = iq_table::iq_reply_for(node, &counters.iq_answers) {
        let _ = send_node(&mut session.ws, cipher, &mut session.send_ctr, &reply).await;
    }
    if counters.world.ends_offline_after(&node.to_owned()) {
        let _ = send_node(
            &mut session.ws,
            cipher,
            &mut session.send_ctr,
            &offline_over(),
        )
        .await;
    }
}
