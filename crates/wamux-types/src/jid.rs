//! JIDs, parsed once at the boundary (#63, #114).
//!
//! `Jid` wraps the library's parsed `Jid` (decided 2026-10-05): the library's
//! parse is the one normalization the core applies, and it is not the core's
//! own. `@c.us` comes out `@s.whatsapp.net` because the library parses it as a
//! phone user (the fix for #4); the core rewrites nothing itself.

use std::str::FromStr;

use wacore_binary::Server;
use wamux_proto::v1 as pb;

use crate::error::WamuxError;

/// Any JID the library accepts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Jid(wacore_binary::Jid);

/// A JID on the group server (`@g.us`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GroupJid(Jid);

/// A JID on the newsletter server (`@newsletter`), a channel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NewsletterJid(Jid);

impl Jid {
    /// `InvalidArgument("empty jid")` for an empty string,
    /// `InvalidArgument("invalid jid '<v>': <reason>")` for one the library refuses.
    pub fn parse(value: &str) -> Result<Self, WamuxError> {
        if value.is_empty() {
            return Err(WamuxError::InvalidArgument("empty jid".to_string()));
        }
        wacore_binary::Jid::from_str(value)
            .map(Self)
            .map_err(|e| WamuxError::InvalidArgument(format!("invalid jid '{value}': {e}")))
    }

    /// A JID that may be absent, where proto3's empty string IS absence: a DM's
    /// `participant`, MarkRead's `sender` (#20, #115). A non-empty value that
    /// does not parse is still `InvalidArgument`.
    pub fn parse_optional(value: &str) -> Result<Option<Self>, WamuxError> {
        if value.is_empty() {
            return Ok(None);
        }
        Self::parse(value).map(Some)
    }

    /// The library's type, for calls into whatsapp-rust.
    pub fn as_lib(&self) -> &wacore_binary::Jid {
        &self.0
    }

    pub fn into_lib(self) -> wacore_binary::Jid {
        self.0
    }
}

impl From<wacore_binary::Jid> for Jid {
    fn from(jid: wacore_binary::Jid) -> Self {
        Self(jid)
    }
}

impl std::fmt::Display for Jid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A required `pb::Jid`: an empty value is `InvalidArgument("missing jid")`,
/// the message the services answer today.
impl TryFrom<pb::Jid> for Jid {
    type Error = WamuxError;

    fn try_from(jid: pb::Jid) -> Result<Self, WamuxError> {
        Self::parse(&jid.value).map_err(|err| match err {
            WamuxError::InvalidArgument(message) if message == "empty jid" => {
                WamuxError::InvalidArgument("missing jid".to_string())
            }
            other => other,
        })
    }
}

impl Jid {
    /// A required jid field of a request (#120): unset, or set with an empty
    /// value, is `InvalidArgument("missing jid")`; a malformed one names the
    /// value (`"invalid jid '<v>': .."`).
    pub fn from_required_wire(jid: Option<pb::Jid>) -> Result<Self, WamuxError> {
        Self::try_from(jid.unwrap_or_default())
    }

    /// A repeated jid field of a request (#121), in order: each one is
    /// required, and the first unset, empty or malformed one fails the lot.
    pub fn from_required_wire_list(jids: Vec<pb::Jid>) -> Result<Vec<Self>, WamuxError> {
        jids.into_iter()
            .map(|jid| Self::from_required_wire(Some(jid)))
            .collect()
    }

    /// An optional jid field of a request (#120), such as a DM's
    /// `participant`: unset, or set with an empty value, is absence. A
    /// malformed one is still `InvalidArgument`.
    pub fn from_optional_wire(jid: Option<pb::Jid>) -> Result<Option<Self>, WamuxError> {
        match jid {
            Some(jid) if !jid.value.is_empty() => Self::parse(&jid.value).map(Some),
            _ => Ok(None),
        }
    }
}

/// A jid the core relays out, in a response or an event (#120): the value
/// verbatim, never parsed, and an empty one is an unset field rather than
/// `Jid { value: "" }`.
pub fn relay_jid(value: impl Into<String>) -> Option<pb::Jid> {
    let value: String = value.into();
    // proto3 has no "empty message": an empty value is an unset field (#120).
    (!value.is_empty()).then_some(pb::Jid { value })
}

/// A library jid the core relays out (#142): its `Display`, verbatim, under the
/// same rule as `relay_jid`. The one conversion every mapping uses, so the rule
/// lives in one place instead of a copy per module.
pub fn relay_lib_jid(jid: &wacore_binary::Jid) -> Option<pb::Jid> {
    relay_jid(jid.to_string())
}

/// An optional library jid: absent is an unset field, never an empty value (#122).
pub fn relay_optional_lib_jid(jid: Option<&wacore_binary::Jid>) -> Option<pb::Jid> {
    jid.and_then(relay_lib_jid)
}

impl From<Jid> for pb::Jid {
    fn from(jid: Jid) -> Self {
        Self {
            value: jid.to_string(),
        }
    }
}

impl GroupJid {
    /// `InvalidArgument("'<v>' is not a group: expected a jid ending in @g.us")`
    /// for a valid JID on another server.
    pub fn parse(value: &str) -> Result<Self, WamuxError> {
        let jid = Jid::parse(value)?;
        if jid.0.server != Server::Group {
            return Err(WamuxError::InvalidArgument(format!(
                "'{value}' is not a group: expected a jid ending in @g.us"
            )));
        }
        Ok(Self(jid))
    }

    pub fn as_jid(&self) -> &Jid {
        &self.0
    }

    /// The library's jid, which is what `client.groups()` takes.
    pub fn as_lib(&self) -> &wacore_binary::Jid {
        self.0.as_lib()
    }

    /// A required group field of a request (#96): unset, or set with an empty
    /// value, is `InvalidArgument("missing jid")`; a jid on another server is
    /// refused by `parse`, before anything reaches the account.
    pub fn from_required_wire(jid: Option<pb::Jid>) -> Result<Self, WamuxError> {
        let value: String = jid.unwrap_or_default().value;
        if value.is_empty() {
            return Err(WamuxError::InvalidArgument("missing jid".to_string()));
        }
        // Parsed from the text as sent, so the refusal names what the caller wrote.
        Self::parse(&value)
    }
}

impl NewsletterJid {
    /// A required channel field of a request (#121): unset, or set with an
    /// empty value, is `InvalidArgument("missing jid")`; a jid on another
    /// server is refused as `parse` refuses it.
    pub fn from_required_wire(jid: Option<pb::Jid>) -> Result<Self, WamuxError> {
        let value: String = jid.unwrap_or_default().value;
        if value.is_empty() {
            return Err(WamuxError::InvalidArgument("missing jid".to_string()));
        }
        // Parsed from the text as sent, so the refusal names what the caller wrote.
        Self::parse(&value)
    }

    /// `InvalidArgument("'<v>' is not a channel: expected a jid ending in
    /// @newsletter")` for a valid JID on another server, as
    /// `require_newsletter_jid` answers today.
    pub fn parse(value: &str) -> Result<Self, WamuxError> {
        let jid = Jid::parse(value)?;
        if jid.0.server != Server::Newsletter {
            return Err(WamuxError::InvalidArgument(format!(
                "'{value}' is not a channel: expected a jid ending in @newsletter"
            )));
        }
        Ok(Self(jid))
    }

    pub fn as_jid(&self) -> &Jid {
        &self.0
    }
}
