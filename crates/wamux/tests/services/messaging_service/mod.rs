//! MessagingService through the socket (#71): all 24 RPCs over a real Unix
//! socket and tonic client, with a companion account logged in against
//! `MockWaServer`.
//!
//! Every send is opened by a `MockPeer` (a parked Signal client the mock
//! serves the bundle of) and compared by value: DMs, group and status sends,
//! the copy to the account's own phone. Chat actions are read back from the
//! app-state patch with the seeded key. The DM sends are also held to the
//! skeleton of the same sends captured live on 2026-10-02 (`captured`). No
//! status is ever posted live. Honors `WAMUX_TEST_ENGINE`, so `scripts/ci.sh`
//! runs it on both engines; `scripts/check-service-coverage.py` fails CI if an
//! RPC of the service stops being called here.

mod captured;
mod channel_polls;
mod chat_actions;
mod error_messages;
mod every_rpc;
mod harness;
mod hermetic;
mod history;
mod jid_fields;
mod media;
mod private_reply;
mod receipts;
mod refusals;
mod replay;
mod rich;
mod sends;
mod spool;
mod status;
mod status_audience;
mod statuses;
mod typed_inputs;
