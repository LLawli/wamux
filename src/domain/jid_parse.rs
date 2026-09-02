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

/// Parse a JID that is allowed to be absent, where proto3's empty string IS
/// absence (the same rule `require_jid` and `wire_defaults` follow).
///
/// Issue #20: `MarkReadRequest.sender` is genuinely optional -- a DM's author
/// is the chat itself -- and an edge saying so with `Jid { value: "" }` rather
/// than by omitting the field must not get `InvalidArgument("empty jid")`.
pub fn parse_optional_jid(value: &str) -> Result<Option<Jid>, WamuxError> {
    if value.is_empty() {
        return Ok(None);
    }
    parse_jid(value).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::Server;

    #[test]
    fn parses_phone_number_jid_and_round_trips() {
        let jid = parse_jid("5511999999999@s.whatsapp.net").unwrap();
        assert_eq!(jid.to_string(), "5511999999999@s.whatsapp.net");
        assert_eq!(jid.user, "5511999999999");
        assert_eq!(jid.server, Server::Pn);
    }

    // REGRESSION (core purity, Sprint 1 worked example in CLAUDE.md): the edge
    // sends `@c.us` to bypass the library's PN->LID upgrade, so the core must
    // parse it as the legacy server and relay it verbatim -- never rewrite it
    // to `@s.whatsapp.net` or anything else.
    #[test]
    fn legacy_c_us_jid_survives_verbatim() {
        let jid = parse_jid("5511999999999@c.us").unwrap();
        assert_eq!(jid.server, Server::Legacy);
        assert_eq!(jid.to_string(), "5511999999999@c.us");
    }

    #[test]
    fn empty_jid_is_invalid_argument_mentioning_empty() {
        let err = parse_jid("").unwrap_err();
        match err {
            WamuxError::InvalidArgument(msg) => assert!(msg.contains("empty"), "got: {msg}"),
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    // `Jid::from_str` treats an `@`-less string as a bare server name, so
    // "not a jid" fails as "unknown server". The error must carry the
    // offending value so the edge can see what it sent.
    #[test]
    fn garbage_jid_error_carries_offending_value() {
        let err = parse_jid("not a jid").unwrap_err();
        match err {
            WamuxError::InvalidArgument(msg) => assert!(msg.contains("not a jid"), "got: {msg}"),
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    #[test]
    fn batch_of_valid_jids_parses_in_order() {
        let values = vec![
            "5511999999999@s.whatsapp.net".to_string(),
            "120363001234567890@g.us".to_string(),
        ];
        let jids = parse_jids(&values).unwrap();
        assert_eq!(jids.len(), 2);
        assert_eq!(jids[0].to_string(), "5511999999999@s.whatsapp.net");
        assert_eq!(jids[1].to_string(), "120363001234567890@g.us");
    }

    // Proto3 has no presence for a string: an edge that means "no sender" can
    // say it either way, and both must mean the same thing.
    #[test]
    fn an_empty_optional_jid_is_absence_not_an_error() {
        assert!(parse_optional_jid("").unwrap().is_none());
    }

    #[test]
    fn a_present_optional_jid_parses_like_any_other() {
        let jid = parse_optional_jid("5511999999999@s.whatsapp.net")
            .unwrap()
            .expect("a non-empty value must parse to Some");
        assert_eq!(jid.user, "5511999999999");
    }

    // Absence is the empty string; garbage is still garbage.
    #[test]
    fn an_invalid_optional_jid_still_fails() {
        assert!(matches!(
            parse_optional_jid("not a jid"),
            Err(WamuxError::InvalidArgument(_))
        ));
    }

    #[test]
    fn one_invalid_jid_fails_the_whole_batch() {
        let values = vec!["5511999999999@s.whatsapp.net".to_string(), String::new()];
        assert!(matches!(
            parse_jids(&values),
            Err(WamuxError::InvalidArgument(_))
        ));
    }
}
