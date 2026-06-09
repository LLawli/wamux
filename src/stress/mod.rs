//! Stress-test harness: a mock WhatsApp WSS endpoint so many real
//! `whatsapp-rust` Clients can connect to a local server and exercise the full
//! transport + Noise + event pipeline without touching the real WhatsApp.
//!
//! Feasible because the client's cert verification is intentionally lenient
//! (`wacore-noise` keeps `WA_CERT_PUB_KEY` unused so an e2e mock can stand in).
//! This is test infrastructure, gated behind the `stress` feature.

pub mod mock_wa_server;

pub use mock_wa_server::MockWaServer;
