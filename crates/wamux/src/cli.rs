//! The daemon's command line (#165). `wamux` with no arguments serves, as it
//! always has; `wamux store decrypt --yes` is the one operator command, and the
//! #166 rotation joins it. No parsing crate: two words and a flag.

/// What the process was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Run the daemon.
    Serve,
    /// Turn an encrypted store back into plaintext. `yes` is the explicit
    /// confirmation: it writes every account's keys in the clear.
    StoreDecrypt { yes: bool },
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
        other => Err(UsageError(format!(
            "unrecognized arguments {other:?}\n{USAGE}"
        ))),
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
