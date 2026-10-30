//! Service integration tests: one binary for every suite that drives a gRPC
//! service over the socket against the `MockWaServer`, so they link once instead
//! of once each (#235). They need the mock, hence the `stress` feature.
//!
//! A new service suite goes here as a `mod`. A suite that owns process-global
//! state (the allocator, a peak-RSS measurement, a global subscriber) must stay
//! its own binary: in a shared one it would see the other suites' state.
//! `media_spool_rss` and `stress_message_load` are two.
#![cfg(feature = "stress")]

// Only a subset of the shared helpers is used per suite.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;

mod call_service;
mod contact_service;
mod group_service;
mod logout_events;
mod media_service;
mod messaging_service;
mod newsletter_service;
mod stress_newsletter_history;
mod stress_newsletter_parse;
mod stress_newsletter_poll_vote;
