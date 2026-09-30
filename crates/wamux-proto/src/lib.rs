//! Generated gRPC/protobuf types for the wamux socket contract. The bottom of the
//! crate graph: it depends on no wamux crate, and the daemon and the tools
//! depend on it.

/// Package `wamux.v1`: the services and messages a socket client sees.
pub mod v1 {
    // Generated oneof enums (EventEnvelope, SendMediaChunk) have naturally
    // unequal variant sizes; we can't restructure generated code.
    #![allow(clippy::large_enum_variant)]
    tonic::include_proto!("wamux.v1");
}

/// Package `wamux.store`: the on-disk blob format (#31). Messages only, and not
/// part of the socket's contract, so it is absent from `FILE_DESCRIPTOR_SET`.
pub mod store {
    tonic::include_proto!("wamux.store");
}

/// Encoded file descriptor set for gRPC server reflection (dev tooling). Only
/// `wamux.v1`: reflection must not expose the store blob format.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/wamux_descriptor.bin"));
