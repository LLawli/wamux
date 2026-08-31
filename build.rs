use std::path::PathBuf;

// Regenerate Rust from proto/ on every build (the .proto files are the source of
// truth). protoc is vendored via protoc-bin-vendored so the host needs no protoc.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    // SAFETY: single-threaded build script; set before tonic_build spawns protoc.
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let protos = [
        "proto/common.proto",
        "proto/account.proto",
        "proto/events.proto",
        "proto/messaging.proto",
        "proto/media.proto",
        "proto/groups.proto",
        "proto/contacts.proto",
        "proto/newsletters.proto",
        "proto/admin.proto",
    ];

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("wamux_descriptor.bin"))
        .compile_protos(&protos, &["proto"])?;

    println!("cargo:rerun-if-changed=proto");
    Ok(())
}
