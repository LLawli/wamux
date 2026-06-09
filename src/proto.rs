//! Generated gRPC types (tonic-build output for package `wamux.v1`).

pub mod v1 {
    // Generated oneof enums (EventEnvelope, SendMediaChunk) have naturally
    // unequal variant sizes; we can't restructure generated code.
    #![allow(clippy::large_enum_variant)]
    tonic::include_proto!("wamux.v1");
}

/// Encoded file descriptor set for gRPC server reflection (dev tooling).
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/wamux_descriptor.bin"));
