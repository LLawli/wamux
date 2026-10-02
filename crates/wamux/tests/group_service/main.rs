//! GroupService through the socket (#68): all 21 RPCs over a real Unix socket
//! and tonic client, with the account logged in against `MockWaServer`.
//!
//! The five reads are answered with what WhatsApp's server sent on 2026-10-02
//! (`captured`, anonymized); the writes with shapes the whatsapp-rust 6f07e3a
//! parser accepts. Honors `WAMUX_TEST_ENGINE`, so `scripts/ci.sh` runs it on
//! both engines. `scripts/check-service-coverage.py` fails CI if an RPC of the
//! service stops being called here.
#![cfg(feature = "stress")]

// Only a subset of the shared helpers is used by this binary.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;

#[allow(dead_code)]
mod captured;
mod harness;
mod participants;
mod reads;
mod statuses;
mod writes;
