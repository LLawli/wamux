//! A poll published to a channel (#205).
//!
//! The library builds a poll only as an end-to-end creation: it keeps the
//! creator identity and the secret so this account can decrypt the votes, and
//! it refuses a channel, which has no end-to-end session to keep them for. The
//! server still takes a poll a channel owner publishes (measured live on
//! 2026-10-21: published, stored, listed in the channel history as a POLL
//! creation). So for a channel the core builds the same `wa::Message` the
//! library builds and publishes it through the library's plain channel send,
//! and the edge gets the same `SendPollResult` it gets for a chat.
//!
//! Mirrors the library's private `build_poll_creation_message` (`features/
//! polls.rs`): the same refusals with the same messages, v3 for a single
//! choice and v1 otherwise, TEXT content, and the 32-byte secret in
//! `messageContextInfo`. Upstream issue asks for a public API to replace this.

use std::collections::HashSet;

use wamux_types::{Jid, NewPoll};
use whatsapp_rust::buffa::MessageField;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::waproto::whatsapp::message::PollCreationMessage;
use whatsapp_rust::waproto::whatsapp::message::poll_creation_message::Option as PollOption;
use whatsapp_rust::{Client, SendResult};

use crate::error::{WamuxError, client_err};

/// The length of a poll's `messageSecret`, as WA Web generates it.
pub const POLL_SECRET_LEN: usize = 32;

/// WA Web caps a poll at 12 options and the library refuses fewer than 2.
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 12;

/// The poll message the library would build for `poll`, carrying `secret`.
/// A poll the library would refuse is `InvalidArgument` with the library's
/// own message, so a chat and a channel refuse the same poll alike.
pub fn newsletter_poll_message(
    poll: &NewPoll,
    secret: &[u8; POLL_SECRET_LEN],
) -> Result<wa::Message, WamuxError> {
    let created = MessageField::some(poll_creation(poll)?);
    let mut message = wa::Message::default();
    // WA Web: v3 for a single choice, v1 for several.
    if poll.selectable_count == 1 {
        message.poll_creation_message_v3 = created;
    } else {
        message.poll_creation_message = created;
    }
    let mut context = wa::MessageContextInfo::default();
    context.message_secret = Some(secret.to_vec());
    message.message_context_info = MessageField::some(context);
    Ok(message)
}

/// Publish `poll` to the channel `to` with a fresh secret, and answer with
/// the send and that secret, like `send_rich::send_poll` does for a chat.
pub async fn send_newsletter_poll(
    client: &Client,
    to: Jid,
    poll: &NewPoll,
) -> Result<(SendResult, Vec<u8>), WamuxError> {
    let secret = fresh_secret()?;
    let message = newsletter_poll_message(poll, &secret)?;
    let result = client
        .send_message(to.into_lib(), message)
        .await
        .map_err(client_err)?;
    Ok((result, secret.to_vec()))
}

/// 32 bytes from the OS random source. The secret decrypts every vote, so it
/// must not be predictable.
fn fresh_secret() -> Result<[u8; POLL_SECRET_LEN], WamuxError> {
    let mut secret = [0u8; POLL_SECRET_LEN];
    getrandom::fill(&mut secret)
        .map_err(|e| WamuxError::Client(format!("no randomness for a poll secret: {e}")))?;
    Ok(secret)
}

fn poll_creation(poll: &NewPoll) -> Result<PollCreationMessage, WamuxError> {
    check_poll(poll)?;
    let mut created = PollCreationMessage::default();
    created.name = Some(poll.name.clone());
    created.options = poll
        .options
        .iter()
        .map(String::as_str)
        .map(poll_option)
        .collect();
    created.selectable_options_count = Some(poll.selectable_count);
    // WA Web always sets the content type (TEXT for a normal poll).
    created.poll_content_type = Some(wa::message::PollContentType::TEXT);
    Ok(created)
}

fn poll_option(name: &str) -> PollOption {
    let mut option = PollOption::default();
    option.option_name = Some(name.to_string());
    option
}

/// The library's refusals, in its order and with its words.
fn check_poll(poll: &NewPoll) -> Result<(), WamuxError> {
    let count = poll.options.len();
    if count < MIN_OPTIONS {
        return Err(invalid_poll("poll must have at least 2 options".into()));
    }
    if count > MAX_OPTIONS {
        return Err(invalid_poll(
            "polls can have a maximum of 12 options".into(),
        ));
    }
    if poll.selectable_count < 1 || poll.selectable_count as usize > count {
        return Err(invalid_poll(format!(
            "selectable_count must be between 1 and {count} (got {})",
            poll.selectable_count
        )));
    }
    // Duplicate names hash alike, which would make votes indistinguishable.
    let mut seen = HashSet::new();
    match poll.options.iter().find(|name| !seen.insert(*name)) {
        Some(name) => Err(invalid_poll(format!("duplicate option name: {name}"))),
        None => Ok(()),
    }
}

fn invalid_poll(reason: String) -> WamuxError {
    WamuxError::InvalidArgument(format!("invalid poll: {reason}"))
}

#[cfg(test)]
#[path = "newsletter_poll_tests.rs"]
mod tests;
