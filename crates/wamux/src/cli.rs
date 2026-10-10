//! The daemon's command line (#165). `wamux` with no arguments serves, as it
//! always has; `wamux store decrypt --yes` (#165) and `wamux store rotate-key
//! --new-key-file <path>` (#166) are the operator commands. No parsing crate: a
//! few words and a flag.

use std::path::PathBuf;

/// What the process was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Run the daemon.
    Serve,
    /// Turn an encrypted store back into plaintext. `yes` is the explicit
    /// confirmation: it writes every account's keys in the clear.
    StoreDecrypt { yes: bool },
    /// Re-seal an encrypted store under the key in `new_key_file` (#166). The
    /// old key is the configured `store_key_file`.
    StoreRotateKey { new_key_file: PathBuf },
    /// Print the usage and exit.
    Help,
}

/// The arguments could not be read as a command; the message says why and
/// includes the usage.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct UsageError(pub String);

const USAGE: &str = "usage:
  wamux                       run the daemon (config: wamux.toml and WAMUX_* variables)
  wamux store decrypt --yes   turn an encrypted store back into plaintext (needs
                              store_key_file; stop the daemon first)
  wamux store rotate-key --new-key-file <path>
                              re-seal an encrypted store under the key in <path>
                              (the old key is store_key_file; stop the daemon first)
  wamux --help                print this text";

/// The usage text.
pub fn usage() -> &'static str {
    USAGE
}

/// Read the arguments after the program name.
pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] => Ok(Command::Serve),
        ["--help" | "-h" | "help"] => Ok(Command::Help),
        ["store", "decrypt"] => Ok(Command::StoreDecrypt { yes: false }),
        ["store", "decrypt", "--yes"] => Ok(Command::StoreDecrypt { yes: true }),
        ["store", "rotate-key", "--new-key-file", path] => Ok(Command::StoreRotateKey {
            new_key_file: PathBuf::from(path),
        }),
        other => Err(UsageError(format!(
            "unrecognized arguments {other:?}\n{USAGE}"
        ))),
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
