//! Channel (`@newsletter`) requests (#26, #116): a history page, a poll vote,
//! this account's own add-ons.

use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::jid::NewsletterJid;

/// WA Web's own ceiling (`REPEATED_CHILD(<vote>, 0, 1000)` in
/// `WASmaxOutMessagePublishNewsletterPollVoteMixin`), which the library also
/// enforces with a private constant. Checked here so an oversized vote answers
/// InvalidArgument: the library's `InvalidRequest` would reach the caller
/// through `client_err` as Unavailable, which reads as "the core is down".
pub const MAX_NEWSLETTER_VOTE_OPTIONS: usize = 1000;

/// A page of a channel's history, newest first.
///
/// The jid is a `NewsletterJid` (#99): measured live (2026-10-09), the server
/// sent nothing back for a group or phone jid, so the call waited out the
/// library's 75 s IQ timeout and came back as Unavailable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewsletterHistoryQuery {
    pub jid: NewsletterJid,
    /// Never 0: a page size of 0 asks the server for nothing, and the empty
    /// answer would read as "this channel has no history".
    pub count: u32,
    /// `None` is the newest page; proto3's 0 is absence, not "before row zero".
    pub before: Option<u64>,
}

/// A vote in a channel poll. The list is the whole selection: it replaces the
/// previous vote, and an empty one removes it. Each option is named by
/// `sha256(option_name)`, at most `MAX_NEWSLETTER_VOTE_OPTIONS`, none repeated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewsletterPollVote {
    pub jid: NewsletterJid,
    pub server_id: u64,
    pub option_hashes: Vec<[u8; 32]>,
}

/// This account's own reactions and poll votes on a channel's recent messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewsletterAddOnsQuery {
    pub jid: NewsletterJid,
    /// Never 0, for the same reason as `NewsletterHistoryQuery::count`.
    pub limit: u32,
}

/// The jid (`NewsletterJid::from_required_wire`, #99), then `"count must be at least 1, got 0"`.
impl TryFrom<pb::GetNewsletterMessagesRequest> for NewsletterHistoryQuery {
    type Error = WamuxError;

    fn try_from(request: pb::GetNewsletterMessagesRequest) -> Result<Self, WamuxError> {
        let jid: NewsletterJid = NewsletterJid::from_required_wire(request.jid)?;
        require_at_least_one("count", request.count)?;
        // 0 is proto3's "absent": start at the newest rather than before row zero.
        let before: Option<u64> = (request.before != 0).then_some(request.before);
        Ok(Self {
            jid,
            count: request.count,
            before,
        })
    }
}

/// The jid (`NewsletterJid::from_required_wire`), then `"server_id must name
/// the poll, got 0"`, then the hashes: `"a poll vote names at most 1000 options, got <n>"`,
/// `"option_hashes[<i>] must be 32 bytes (sha256 of the option name), got
/// <n>"`, `"option_hashes[<i>] repeats an earlier option"`.
impl TryFrom<pb::SendNewsletterPollVoteRequest> for NewsletterPollVote {
    type Error = WamuxError;

    fn try_from(request: pb::SendNewsletterPollVoteRequest) -> Result<Self, WamuxError> {
        let jid: NewsletterJid = NewsletterJid::from_required_wire(request.jid)?;
        if request.server_id == 0 {
            return Err(WamuxError::InvalidArgument(
                "server_id must name the poll, got 0".to_string(),
            ));
        }
        let option_hashes: Vec<[u8; 32]> = option_hashes(&request.option_hashes)?;
        Ok(Self {
            jid,
            server_id: request.server_id,
            option_hashes,
        })
    }
}

/// The jid (`NewsletterJid::from_required_wire`), then `"limit must be at least 1, got 0"`.
impl TryFrom<pb::GetMyNewsletterAddOnsRequest> for NewsletterAddOnsQuery {
    type Error = WamuxError;

    fn try_from(request: pb::GetMyNewsletterAddOnsRequest) -> Result<Self, WamuxError> {
        let jid: NewsletterJid = NewsletterJid::from_required_wire(request.jid)?;
        require_at_least_one("limit", request.limit)?;
        Ok(Self {
            jid,
            limit: request.limit,
        })
    }
}

/// A page size (`count`, `limit`) is the caller's to choose, and 0 asks the
/// server for nothing. Refused so it answers InvalidArgument instead of an
/// empty list, which reads as "this channel has no history".
fn require_at_least_one(field: &str, value: u32) -> Result<(), WamuxError> {
    if value == 0 {
        return Err(WamuxError::InvalidArgument(format!(
            "{field} must be at least 1, got 0"
        )));
    }
    Ok(())
}

/// The wire's `repeated bytes` as the library's fixed-size hashes: each exactly
/// 32 bytes, at most `MAX_NEWSLETTER_VOTE_OPTIONS`, none repeated.
fn option_hashes(raw: &[Vec<u8>]) -> Result<Vec<[u8; 32]>, WamuxError> {
    if raw.len() > MAX_NEWSLETTER_VOTE_OPTIONS {
        return Err(WamuxError::InvalidArgument(format!(
            "a poll vote names at most {MAX_NEWSLETTER_VOTE_OPTIONS} options, got {}",
            raw.len()
        )));
    }
    let mut hashes: Vec<[u8; 32]> = Vec::with_capacity(raw.len());
    for (index, bytes) in raw.iter().enumerate() {
        let hash: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
            WamuxError::InvalidArgument(format!(
                "option_hashes[{index}] must be 32 bytes (sha256 of the option name), got {}",
                bytes.len()
            ))
        })?;
        if hashes.contains(&hash) {
            return Err(WamuxError::InvalidArgument(format!(
                "option_hashes[{index}] repeats an earlier option"
            )));
        }
        hashes.push(hash);
    }
    Ok(hashes)
}
