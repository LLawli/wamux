//! NewsletterService through the socket (#70): all 6 RPCs over a real Unix
//! socket and tonic client, with the account logged in against `MockWaServer`.
//!
//! The five reads are answered with what WhatsApp's server sent on 2026-10-02
//! (`captured`, anonymized); the poll vote is held to the stanza WA Web sent
//! live on 2026-09-25 (#26), since voting on a real channel is a public write.
//! Honors `WAMUX_TEST_ENGINE`, so `scripts/ci.sh` runs it on both engines.
//! `scripts/check-service-coverage.py` fails CI if an RPC of the service stops
//! being called here.
#![cfg(feature = "stress")]

// Only a subset of the shared helpers is used by this binary.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;

mod captured;
mod harness;
mod reads;
mod statuses;
mod writes;
