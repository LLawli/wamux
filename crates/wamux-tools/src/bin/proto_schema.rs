//! `proto_schema` (#280): prints the wamux.v1 contract as the JSON schema the
//! client snippet page reads. `scripts/ci.sh` diffs it against
//! `site/schema.json`; regenerate with
//! `cargo run -p wamux-tools --bin proto_schema > site/schema.json`.

use std::io::Write;
use std::process::ExitCode;

use wamux_tools::proto_schema::schema_json;

fn main() -> ExitCode {
    let text: String = match schema_json(wamux::proto::FILE_DESCRIPTOR_SET) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("proto_schema: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = std::io::stdout().lock().write_all(text.as_bytes()) {
        eprintln!("proto_schema: cannot write the schema to stdout: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
