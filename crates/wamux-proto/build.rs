use std::path::PathBuf;

// Regenerate Rust from proto/ on every build (the .proto files are the source of
// truth). protoc is vendored via protoc-bin-vendored so the host needs no protoc.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let protos = [
        "proto/common.proto",
        "proto/account.proto",
        "proto/events.proto",
        "proto/messaging.proto",
        "proto/media.proto",
        "proto/group_metadata.proto",
        "proto/group_update.proto",
        "proto/app_state.proto",
        "proto/groups.proto",
        "proto/contacts.proto",
        "proto/newsletter_tallies.proto",
        "proto/newsletters.proto",
        "proto/admin.proto",
    ];

    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("wamux_descriptor.bin"))
        .compile_with_config(protoc_config()?, &protos, &["proto"])?;

    // The on-disk blob format (#31). Messages only: no service, and no entry in
    // the reflection descriptor above, because it is not part of the socket's
    // contract. btree_map makes map encoding deterministic, which the
    // cross-engine byte-parity test depends on.
    tonic_prost_build::configure()
        .build_client(false)
        .build_server(false)
        .btree_map(".")
        .compile_with_config(protoc_config()?, &["proto/store/blobs.proto"], &["proto"])?;

    println!("cargo:rerun-if-changed=proto");
    Ok(())
}

// Points prost at the vendored protoc through its config, so the build script
// needs neither a host protoc nor a mutated process environment (no `set_var`).
fn protoc_config() -> Result<prost_build::Config, Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    Ok(config)
}
