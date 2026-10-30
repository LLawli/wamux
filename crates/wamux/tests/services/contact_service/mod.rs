//! ContactService through the socket (#197): the blocklist and SetAbout RPCs
//! over a real Unix socket and tonic client, with the account logged in against
//! `MockWaServer`, plus a call to every one of the 15 RPCs.
//!
//! Honors `WAMUX_TEST_ENGINE`, so `scripts/ci.sh` runs it on both engines.
//! `scripts/check-service-coverage.py` fails CI if an RPC of the service stops
//! being called here.

mod about;
mod blocking;
mod blocklist;
mod harness;
mod picture;
mod statuses;
