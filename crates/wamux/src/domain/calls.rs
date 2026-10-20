//! Call control that needs no media stack (#200), over `client.voip()`.

use wamux_types::Jid;
use whatsapp_rust::Client;

use crate::error::{WamuxError, client_err};

/// Reject the call `call_id` ringing this account. Fire and forget: the library
/// sends one `<call><reject/></call>` stanza and waits for no answer, so `Ok`
/// means the stanza was sent, not that the caller's side stopped ringing.
///
/// `peer` (the event's `from`) and `call_creator` stay separate because the
/// stanza addresses one and names the other, and they differ when a companion
/// device is involved. An empty `call_id` is refused here so nothing is sent.
pub async fn reject_call(
    client: &Client,
    call_id: &str,
    peer: &Jid,
    call_creator: &Jid,
) -> Result<(), WamuxError> {
    if call_id.is_empty() {
        return Err(WamuxError::InvalidArgument("missing call_id".into()));
    }
    client
        .voip()
        .reject_call(call_id, peer.as_lib(), call_creator.as_lib())
        .await
        .map_err(client_err)
}
