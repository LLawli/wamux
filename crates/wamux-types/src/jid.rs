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
}

impl NewsletterJid {
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
