//! A LID<->phone lookup query (#1, #116).

use crate::error::WamuxError;
use crate::jid::Jid;

/// One query of a ResolveLidPn batch: the text exactly as the caller sent it,
/// which the answer echoes so the caller can tell which of a batch went
/// unanswered, and the jid it parses to, which is what the library looks up.
/// The echo stays as sent (decided 2026-10-05): a `@c.us` query comes back
/// `@c.us`, not in the spelling the parse would print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LidPnQuery {
    pub query: String,
    pub jid: Jid,
}

/// `InvalidArgument("empty jid")` / `("invalid jid '<v>': ..")`, as
/// `parse_jids` answered.
impl TryFrom<String> for LidPnQuery {
    type Error = WamuxError;

    fn try_from(query: String) -> Result<Self, WamuxError> {
        let jid: Jid = Jid::parse(&query)?;
        Ok(Self { query, jid })
    }
}
