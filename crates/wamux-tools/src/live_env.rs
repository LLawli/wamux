//! The environment contract every wamux-tools binary reads (#64).
//!
//! Everything is validated before a socket or a database is opened, so a
//! missing variable fails fast and names itself. Each reader takes a lookup
//! function instead of calling `std::env::var` directly: tests cannot set
//! process variables (`set_var` is `unsafe` in edition 2024 and the workspace
//! forbids `unsafe`), and binaries pass `&process_env`.

use std::time::Duration;

pub const SOCKET_PATH_VAR: &str = "WAMUX_SOCKET_PATH";
pub const ACCOUNT_REF_VAR: &str = "WAMUX_REF";
pub const PEER_REF_VAR: &str = "WAMUX_PEER_REF";
pub const LIVE_DEST_VAR: &str = "WAMUX_LIVE_DEST";
pub const DELIVERY_SECS_VAR: &str = "WAMUX_DELIVERY_SECS";
pub const DESTRUCTIVE_VAR: &str = "WAMUX_E2E_DESTRUCTIVE";
pub const INBOUND_VAR: &str = "WAMUX_E2E_INBOUND";
pub const DEFAULT_DELIVERY_SECS: u64 = 30;

/// How a binary reads one variable. `None` and an empty value both mean unset.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LiveEnvError {
    #[error("set {var}: {hint}")]
    Missing {
        var: &'static str,
        hint: &'static str,
    },
    #[error(
        "{var}={value:?} is not a JID: expected <digits>@s.whatsapp.net, <id>@lid or <id>@g.us"
    )]
    NotAJid { var: &'static str, value: String },
    #[error(
        "{var}={value:?} uses the legacy @c.us server, which encrypts for nobody (issue #4); \
         spell it <digits>@s.whatsapp.net"
    )]
    LegacyServer { var: &'static str, value: String },
    #[error("{var}={value:?} is not a whole number of seconds")]
    NotSeconds { var: &'static str, value: String },
    #[error("{var}={value:?} is not the explicit confirmation; set {var}=yes to run this binary")]
    NotConfirmed { var: &'static str, value: String },
    #[error("{LIVE_DEST_VAR} ({dest}) is this account's own number ({own}); pick another chat")]
    OwnNumber { own: String, dest: String },
}

/// The real process environment, with an empty value read as unset.
pub fn process_env(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|value| !value.is_empty())
}

/// `WAMUX_SOCKET_PATH`, else `$HOME/.local/state/wamux/wamux.sock`: the path
/// the production unit binds, so a bare run talks to the daemon that is up.
pub fn socket_path_from(lookup: EnvLookup) -> Result<String, LiveEnvError> {
    if let Some(path) = lookup(SOCKET_PATH_VAR) {
        return Ok(path);
    }
    let home = lookup("HOME").ok_or(LiveEnvError::Missing {
        var: SOCKET_PATH_VAR,
        hint: "the daemon's socket path (HOME is unset, so there is no default to fall back on)",
    })?;
    Ok(format!("{home}/.local/state/wamux/wamux.sock"))
}

/// A variable that has no default, with the hint shown when it is missing.
pub fn required_from(
    lookup: EnvLookup,
    var: &'static str,
    hint: &'static str,
) -> Result<String, LiveEnvError> {
    lookup(var).ok_or(LiveEnvError::Missing { var, hint })
}

/// `WAMUX_REF`: the external_ref of the account the binary acts as. No
/// default: a binary that guessed would act as whichever account was paired
/// under the guessed name.
pub fn account_ref_from(lookup: EnvLookup) -> Result<String, LiveEnvError> {
    required_from(
        lookup,
        ACCOUNT_REF_VAR,
        "the external_ref of the account to act as",
    )
}

/// `WAMUX_LIVE_DEST`: the one chat a writing binary may send to. Required,
/// and spelled with a modern server (`@s.whatsapp.net`, `@lid`, `@g.us`).
pub fn live_dest_from(lookup: EnvLookup) -> Result<String, LiveEnvError> {
    let value = required_from(
        lookup,
        LIVE_DEST_VAR,
        "the chat to send to, as <digits>@s.whatsapp.net",
    )?;
    let not_a_jid = || LiveEnvError::NotAJid {
        var: LIVE_DEST_VAR,
        value: value.clone(),
    };
    let (user, server) = value.split_once('@').ok_or_else(not_a_jid)?;
    if user.is_empty() {
        return Err(not_a_jid());
    }
    match server {
        // Issue #4: the legacy server parses as a non-phone JID and ships
        // a stanza with nothing for the recipient's handset.
        "c.us" => Err(LiveEnvError::LegacyServer {
            var: LIVE_DEST_VAR,
            value,
        }),
        "s.whatsapp.net" | "lid" | "g.us" => Ok(value),
        _ => Err(not_a_jid()),
    }
}

/// `WAMUX_DELIVERY_SECS`: how long a send waits for its `delivered` receipt.
pub fn delivery_window_from(lookup: EnvLookup) -> Result<Duration, LiveEnvError> {
    let Some(raw) = lookup(DELIVERY_SECS_VAR) else {
        return Ok(Duration::from_secs(DEFAULT_DELIVERY_SECS));
    };
    raw.trim()
        .parse::<u64>()
        .map(Duration::from_secs)
        .map_err(|_| LiveEnvError::NotSeconds {
            var: DELIVERY_SECS_VAR,
            value: raw,
        })
}

/// `WAMUX_E2E_DESTRUCTIVE`: exactly `yes`, nothing else (not `1`, not `true`),
/// because the binary changes the account's profile and creates groups.
pub fn destructive_ack_from(lookup: EnvLookup) -> Result<(), LiveEnvError> {
    let value = required_from(
        lookup,
        DESTRUCTIVE_VAR,
        "set to yes to confirm this binary changes the profile and creates groups",
    )?;
    if value != "yes" {
        return Err(LiveEnvError::NotConfirmed {
            var: DESTRUCTIVE_VAR,
            value,
        });
    }
    Ok(())
}

/// `WAMUX_E2E_INBOUND=1`: opt in to the window where a human sends this
/// account a message, so a run with nobody at the phone does not wait or fail.
pub fn inbound_requested_from(lookup: EnvLookup) -> bool {
    lookup(INBOUND_VAR).as_deref() == Some("1")
}

/// The user part of a JID, without the server or the device suffix:
/// `5561...:16@s.whatsapp.net` -> `5561...`.
pub fn user_of(jid: &str) -> &str {
    let before_server = jid.split('@').next().unwrap_or(jid);
    before_server.split(':').next().unwrap_or(before_server)
}

/// Drop the device suffix: `5561...:16@s.whatsapp.net` -> `5561...@s.whatsapp.net`.
/// A recipient is a user, not a device: the server answers 400 bad-request to
/// a group whose participant names one.
pub fn bare_jid(jid: &str) -> String {
    match jid.split_once('@') {
        Some((user, server)) => format!("{}@{server}", user_of(user)),
        None => user_of(jid).to_string(),
    }
}

/// Whether two JIDs name the same user, ignoring device and server.
pub fn same_user(one: &str, other: &str) -> bool {
    user_of(one) == user_of(other)
}

/// The text of an optional wire `Jid`, empty when unset (#122): the binaries
/// compare and print jids as text, and an unset field reads as "no jid".
pub fn jid_text_of(jid: &Option<wamux::proto::v1::Jid>) -> &str {
    jid.as_ref().map_or("", |j| j.value.as_str())
}

/// Refuse a destination that is the connected account's own number.
pub fn refuse_own_number(own: &str, dest: &str) -> Result<(), LiveEnvError> {
    if same_user(own, dest) {
        return Err(LiveEnvError::OwnNumber {
            own: own.to_string(),
            dest: dest.to_string(),
        });
    }
    Ok(())
}
