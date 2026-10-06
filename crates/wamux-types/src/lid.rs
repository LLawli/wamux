//! A LID<->phone lookup query (#1, #116).

use wamux_proto::v1 as pb;

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

/// `InvalidArgument("missing jid")` for an empty value (#121),
/// `("invalid jid '<v>': ..")` for a malformed one. `query` keeps the value
/// as it came.
impl TryFrom<pb::Jid> for LidPnQuery {
    type Error = WamuxError;

    fn try_from(query: pb::Jid) -> Result<Self, WamuxError> {
        let jid: Jid = Jid::try_from(query.clone())?;
        Ok(Self {
            query: query.value,
            jid,
        })
    }
}
