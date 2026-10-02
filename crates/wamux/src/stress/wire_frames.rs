//! Frame IO for the mock's WS connection (split from `mock_wa_server.rs` to
//! keep that file under the 500-line limit, #68).

use anyhow::{Context, anyhow};
use bytes::{Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use tokio_websockets::Message;
use wacore::framing::{FrameDecoder, encode_frame};
use wacore_binary::consts::WA_CONN_HEADER;

/// Pull the next WhatsApp frame, stripping the 4-byte `WA_CONN_HEADER` that
/// prefixes only the very first client frame.
pub(super) async fn next_frame<S>(
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
pub(super) async fn send_frame<S>(
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

/// Marshal `node`, seal it with the next send counter and put it on the wire.
pub(super) async fn send_node<S>(
    ws: &mut tokio_websockets::WebSocketStream<S>,
    cipher: &wacore_noise::NoiseCipher,
    counter: &mut u32,
    node: &wacore_binary::Node,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let plain = wacore_binary::marshal::marshal(node).map_err(|e| anyhow!("marshal: {e}"))?;
    let sealed = cipher
        .encrypt_with_counter(*counter, &plain)
        .map_err(|e| anyhow!("encrypt: {e}"))?;
    *counter += 1;
    send_frame(ws, &sealed).await
}
