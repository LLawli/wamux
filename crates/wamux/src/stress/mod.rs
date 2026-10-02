//! Stress-test harness: a mock WhatsApp WSS endpoint so many real
//! `whatsapp-rust` Clients can connect to a local server and exercise the full
//! transport + Noise + event pipeline without touching the real WhatsApp.
//!
//! Feasible because the client's cert verification is intentionally lenient
//! (`wacore-noise` keeps `WA_CERT_PUB_KEY` unused so an e2e mock can stand in).
//! This is test infrastructure, gated behind the `stress` feature.

mod iq_table;
pub mod loopback_http;
pub mod mock_cdn;
pub mod mock_wa_server;
mod wire_frames;

pub use mock_cdn::MockCdn;
pub use mock_wa_server::MockWaServer;
