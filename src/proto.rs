//! Generated gRPC types (tonic-build output for package `wamux.v1`).

pub mod v1 {
    // Generated oneof enums (EventEnvelope, SendMediaChunk) have naturally
    // unequal variant sizes; we can't restructure generated code.
    #![allow(clippy::large_enum_variant)]
    // tonic's generated client/server return `Result<_, tonic::Status>`, and
    // `Status` is ~176 bytes. The clippy of the nightly aligned with
    // whatsapp-rust main (#30, nightly-2026-06-16) flags every one of those
    // signatures; they are tonic's shape, not ours to box.
    #![allow(clippy::result_large_err)]
    tonic::include_proto!("wamux.v1");
}

/// Encoded file descriptor set for gRPC server reflection (dev tooling).
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/wamux_descriptor.bin"));
