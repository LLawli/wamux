//! Poll creation, a vote, and the votes to tally (#13, #115).

use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::jid::Jid;
use crate::message_id::MessageId;

/// WhatsApp generates a 32-byte per-poll secret and the HKDF refuses any other
/// length. Checked at the boundary so a mis-sized secret answers
/// InvalidArgument (the caller's mistake) instead of the Unavailable an
/// upstream client error maps to, which would read as "the core is down".
pub const POLL_SECRET_LEN: usize = 32;

/// This account's vote. Empty `options` retracts it. The chat is the service's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollVoteCast {
    /// The poll's stanza id. It keys the vote's HKDF, so an empty one would
    /// derive a wrong key rather than fail.
    pub poll_id: MessageId,
    pub creator: Jid,
    pub message_secret: [u8; POLL_SECRET_LEN],
    pub options: Vec<String>,
}

/// One vote as the edge received it, still encrypted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedVote {
    pub voter: Jid,
    pub enc_payload: Vec<u8>,
    pub enc_iv: Vec<u8>,
}

/// The votes to tally, in the order they arrived (oldest first, last wins).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollVotesToTally {
    pub poll_id: MessageId,
    pub creator: Jid,
    pub message_secret: [u8; POLL_SECRET_LEN],
    /// Never empty: a vote names its option by the SHA-256 of the option's
    /// name, so with no names every vote would tally against nothing.
    pub options: Vec<String>,
    pub votes: Vec<EncryptedVote>,
}

/// A poll to create. The library validates the options and the count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewPoll {
    pub name: String,
    pub options: Vec<String>,
    pub selectable_count: u32,
}

/// Creator, poll id (`"empty poll_id"`), secret (`"message_secret must be 32
/// bytes, got <n>"`), in the order the domain checked them.
impl TryFrom<pb::SendPollVoteRequest> for PollVoteCast {
    type Error = WamuxError;

    fn try_from(request: pb::SendPollVoteRequest) -> Result<Self, WamuxError> {
        let (creator, poll_id, message_secret) = vote_identity(
            &request.poll_creator_jid,
            request.poll_id,
            &request.message_secret,
        )?;
        Ok(Self {
            poll_id,
            creator,
            message_secret,
            options: request.options,
        })
    }
}

/// Creator, poll id, secret, options (`"no options to tally against"`), then
/// every voter: one that does not parse fails the whole tally.
impl TryFrom<pb::AggregatePollVotesRequest> for PollVotesToTally {
    type Error = WamuxError;

    fn try_from(request: pb::AggregatePollVotesRequest) -> Result<Self, WamuxError> {
        let (creator, poll_id, message_secret) = vote_identity(
            &request.poll_creator_jid,
            request.poll_id,
            &request.message_secret,
        )?;
        if request.options.is_empty() {
            return Err(WamuxError::InvalidArgument(
                "no options to tally against".to_string(),
            ));
        }
        let votes = request
            .votes
            .into_iter()
            .map(EncryptedVote::try_from)
            .collect::<Result<Vec<EncryptedVote>, WamuxError>>()?;
        Ok(Self {
            poll_id,
            creator,
            message_secret,
            options: request.options,
            votes,
        })
    }
}

impl TryFrom<pb::PollVote> for EncryptedVote {
    type Error = WamuxError;

    fn try_from(vote: pb::PollVote) -> Result<Self, WamuxError> {
        Ok(Self {
            voter: Jid::parse(&vote.voter_jid)?,
            enc_payload: vote.enc_payload,
            enc_iv: vote.enc_iv,
        })
    }
}

/// The three checks a vote and a tally share, in the order the domain made
/// them: creator, poll id, secret.
fn vote_identity(
    creator: &str,
    poll_id: String,
    secret: &[u8],
) -> Result<(Jid, MessageId, [u8; POLL_SECRET_LEN]), WamuxError> {
    let creator = Jid::parse(creator)?;
    // The poll id keys the vote's HKDF: an empty one derives a wrong key rather
    // than failing, so it is refused under its own name, not "empty message id".
    let poll_id = MessageId::new(poll_id)
        .map_err(|_| WamuxError::InvalidArgument("empty poll_id".to_string()))?;
    let secret = <[u8; POLL_SECRET_LEN]>::try_from(secret).map_err(|_| {
        WamuxError::InvalidArgument(format!(
            "message_secret must be {POLL_SECRET_LEN} bytes, got {}",
            secret.len()
        ))
    })?;
    Ok((creator, poll_id, secret))
}

impl From<pb::SendPollRequest> for NewPoll {
    fn from(request: pb::SendPollRequest) -> Self {
        Self {
            name: request.name,
            options: request.options,
            selectable_count: request.selectable_count,
        }
    }
}
