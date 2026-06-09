//! Parse/validate JID strings coming off the wire into typed `Jid`s.

use std::str::FromStr;

use whatsapp_rust::Jid;

use crate::error::WamuxError;

/// Parse a JID, mapping failures to a clean `InvalidArgument`.
pub fn parse_jid(value: &str) -> Result<Jid, WamuxError> {
    if value.is_empty() {
        return Err(WamuxError::InvalidArgument("empty jid".to_string()));
    }
    Jid::from_str(value)
        .map_err(|e| WamuxError::InvalidArgument(format!("invalid jid '{value}': {e}")))
}

/// Parse a batch of JID strings.
pub fn parse_jids(values: &[String]) -> Result<Vec<Jid>, WamuxError> {
    values.iter().map(|v| parse_jid(v)).collect()
}
