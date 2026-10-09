//! Group management helpers over `client.groups()`.

use std::sync::Arc;

use wacore::iq::contacts::SetProfilePictureSpec;
use wacore::iq::groups::{
    GroupCreateOptions, GroupDescription, GroupParticipantOptions, GroupParticipatingIq,
    GroupSubject, ParticipantChangeResponse,
};
use wamux_types::{GroupJid, Jid, relay_jid, relay_lib_jid, relay_optional_lib_jid};
use whatsapp_rust::Client;
use whatsapp_rust::features::{GroupError, PreviousDescription};

use crate::domain::group_metadata::{group_metadata_of, membership_request_of};
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// The library's own jids, which is what `client.groups()` takes. Cloning
/// is cheap and the conversion is the one place the newtype is unwrapped.
fn lib_jids(participants: &[Jid]) -> Vec<whatsapp_rust::Jid> {
    participants
        .iter()
        .map(|jid| jid.as_lib().clone())
        .collect()
}

/// A request the library refused before sending anything (#96) comes back as
/// `GroupError::InvalidRequest`; each caller says which status that cause
/// deserves (a bad invite code is the caller's fault, an unmapped LID is a
/// precondition). Carries the inner message, not the "invalid group request:"
/// Display. Everything else keeps the usual mapping.
fn local_refusal(err: GroupError, as_error: fn(String) -> WamuxError) -> WamuxError {
    match err {
        GroupError::InvalidRequest(message) => as_error(message),
        other => client_err(other),
    }
}

pub async fn create_group(
    client: &Client,
    subject: &str,
    participants: &[Jid],
) -> Result<pb::GroupJidResponse, WamuxError> {
    let parts: Vec<whatsapp_rust::Jid> = lib_jids(participants);
    let mut options = GroupCreateOptions::new(subject);
    options.participants = parts
        .into_iter()
        .map(GroupParticipantOptions::new)
        .collect();
    let result = client
        .groups()
        .create_group(options)
        .await
        .map_err(|err| local_refusal(err, WamuxError::FailedPrecondition))?;
    Ok(pb::GroupJidResponse {
        group: relay_lib_jid(&result.metadata.id),
        metadata: Some(group_metadata_of(&result.metadata)),
        join_outcome: pb::JoinOutcome::Unspecified as i32,
    })
}

/// Relay one server verdict per participant, verbatim. The server answers each
/// participant separately, so a change request can partially succeed; deciding
/// which failure is retryable, invitable (the 403 `add_request` token), or
/// fatal is edge policy, so the core hands the whole list back instead of
/// collapsing it into one ok/failed.
fn participant_changes(changes: Vec<ParticipantChangeResponse>) -> pb::ParticipantsResponse {
    pb::ParticipantsResponse {
        results: changes.into_iter().map(participant_change).collect(),
    }
}

fn participant_change(c: ParticipantChangeResponse) -> pb::ParticipantChange {
    pb::ParticipantChange {
        jid: relay_lib_jid(&c.jid),
        // Absent server attrs relay as the proto3 default (empty string), the
        // same rule the rest of the crate uses for "not set".
        status: c.status.unwrap_or_default(),
        error: c.error.unwrap_or_default(),
        phone_number: relay_optional_lib_jid(c.phone_number.as_ref()),
        username: c.username.unwrap_or_default(),
        add_request: c.add_request.map(|a| pb::AddRequestInfo {
            code: a.code,
            expiration: a.expiration,
        }),
    }
}

pub async fn add_participants(
    client: &Client,
    group: &GroupJid,
    participants: &[Jid],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let parts: Vec<whatsapp_rust::Jid> = lib_jids(participants);
    client
        .groups()
        .add_participants(group.as_lib(), &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn remove_participants(
    client: &Client,
    group: &GroupJid,
    participants: &[Jid],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let parts: Vec<whatsapp_rust::Jid> = lib_jids(participants);
    client
        .groups()
        .remove_participants(group.as_lib(), &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn promote(
    client: &Client,
    group: &GroupJid,
    participants: &[Jid],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let parts: Vec<whatsapp_rust::Jid> = lib_jids(participants);
    client
        .groups()
        .promote_participants(group.as_lib(), &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn demote(
    client: &Client,
    group: &GroupJid,
    participants: &[Jid],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let parts: Vec<whatsapp_rust::Jid> = lib_jids(participants);
    client
        .groups()
        .demote_participants(group.as_lib(), &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn set_subject(
    client: &Client,
    group: &GroupJid,
    subject: &str,
) -> Result<(), WamuxError> {
    let subject =
        GroupSubject::new(subject).map_err(|e| WamuxError::InvalidArgument(e.to_string()))?;
    client
        .groups()
        .set_subject(group.as_lib(), subject)
        .await
        .map_err(client_err)
}

pub async fn set_description(
    client: &Client,
    group: &GroupJid,
    description: &str,
) -> Result<(), WamuxError> {
    let description = GroupDescription::new(description)
        .map_err(|e| WamuxError::InvalidArgument(e.to_string()))?;
    client
        .groups()
        // 0.7 types the third argument as `PreviousDescription`, the server's
        // optimistic-concurrency token. `Resolve` (the lib default) reads the
        // current description id first; it is the only variant that is correct
        // when the caller holds no metadata, and the RPC gives the edge no way
        // to pass one. `Absent` would 409 on any group that already has a
        // description, so it is not the safe literal translation of the old
        // `None` it looks like.
        .set_description(
            group.as_lib(),
            Some(description),
            PreviousDescription::Resolve,
        )
        .await
        .map_err(client_err)
}

pub async fn get_metadata(
    client: &Client,
    group: &GroupJid,
) -> Result<pb::GroupMetadataResponse, WamuxError> {
    // main split 0.7.0's `get_metadata` into `fetch_metadata` (protocol data
    // only) plus an opt-in `resolve_participant_addresses` backfill (#30). The
    // roster is one of the few places the server volunteers a username/PN pair
    // at all (issue #1), so GetGroupMetadata keeps asking for both.
    let mut metadata = client
        .groups()
        .fetch_metadata(group.as_lib())
        .await
        .map_err(client_err)?;
    client
        .groups()
        .resolve_participant_addresses(&mut metadata)
        .await;
    Ok(pb::GroupMetadataResponse {
        metadata: Some(group_metadata_of(&metadata)),
    })
}

pub async fn invite_link(
    client: &Client,
    group: &GroupJid,
    reset: bool,
) -> Result<pb::InviteLinkResponse, WamuxError> {
    let link = client
        .groups()
        .get_invite_link(group.as_lib(), reset)
        .await
        .map_err(client_err)?;
    Ok(pb::InviteLinkResponse { link })
}

pub async fn join_with_invite(
    client: &Client,
    code: &str,
) -> Result<pb::GroupJidResponse, WamuxError> {
    let result = client
        .groups()
        .join_with_invite_code(code)
        .await
        .map_err(|err| local_refusal(err, WamuxError::InvalidArgument))?;
    let (group_jid, outcome) = match result {
        whatsapp_rust::JoinGroupResult::Joined(jid) => (jid, pb::JoinOutcome::Joined),
        whatsapp_rust::JoinGroupResult::PendingApproval(jid) => {
            (jid, pb::JoinOutcome::PendingApproval)
        }
    };
    Ok(pb::GroupJidResponse {
        group: relay_jid(group_jid.to_string()),
        metadata: None,
        join_outcome: outcome as i32,
    })
}

pub async fn leave(client: &Client, group: &GroupJid) -> Result<(), WamuxError> {
    client
        .groups()
        .leave(group.as_lib())
        .await
        .map_err(client_err)
}

/// List the groups the account participates in (summary + projected metadata).
///
/// main renamed `get_participating` to `list_participating`, but the new one
/// answers slim `GroupOverview`s with no participants and no description
/// (#30): using it here would degrade ListGroups. So this issues the full
/// `GroupParticipatingIq` itself, converts each group with `GroupMetadata::
/// from`, and backfills phone numbers the same way `get_metadata` does (issue
/// #1) -- 0.7.0's `get_participating` did that backfill internally; main only
/// does it when asked, and dropping it would lose identities the edge relies
/// on. The future this builds is still non-Send, so it runs isolated.
pub async fn list_participating(client: Arc<Client>) -> Result<Vec<pb::GroupSummary>, WamuxError> {
    let groups = crate::domain::isolate::run_isolated(move || async move {
        let response = client.execute(GroupParticipatingIq::new()).await?;
        let mut metadatas: Vec<whatsapp_rust::GroupMetadata> = response
            .groups
            .into_iter()
            .map(whatsapp_rust::GroupMetadata::from)
            .collect();
        for metadata in &mut metadatas {
            client
                .groups()
                .resolve_participant_addresses(metadata)
                .await;
        }
        Ok::<_, anyhow::Error>(metadatas)
    })
    .await?;
    Ok(group_summaries(groups))
}

/// ListGroups' projection: one summary per group, sorted by subject, each
/// carrying the full typed metadata (roster included). An absent subject is
/// "", as 0.7.0 parsed it. Pure, so the contract is testable without a client.
pub fn group_summaries(groups: Vec<whatsapp_rust::GroupMetadata>) -> Vec<pb::GroupSummary> {
    let mut summaries: Vec<pb::GroupSummary> = groups
        .iter()
        .map(|m| pb::GroupSummary {
            jid: relay_lib_jid(&m.id),
            subject: m.subject.clone().unwrap_or_default(),
            participants: m.participants.len() as u32,
            metadata: Some(group_metadata_of(m)),
        })
        .collect();
    summaries.sort_by(|a, b| a.subject.cmp(&b.subject));
    summaries
}

pub async fn set_announce(
    client: &Client,
    group: &GroupJid,
    announce: bool,
) -> Result<(), WamuxError> {
    client
        .groups()
        .set_announce(group.as_lib(), announce)
        .await
        .map_err(client_err)
}

pub async fn set_locked(client: &Client, group: &GroupJid, locked: bool) -> Result<(), WamuxError> {
    client
        .groups()
        .set_locked(group.as_lib(), locked)
        .await
        .map_err(client_err)
}

/// Group-level disappearing-messages timer (0 disables). Distinct from the
/// per-message ephemeral flag the edge sets on SendText/SendMedia.
pub async fn set_ephemeral(
    client: &Client,
    group: &GroupJid,
    expiration_seconds: u32,
) -> Result<(), WamuxError> {
    client
        .groups()
        .set_ephemeral(group.as_lib(), expiration_seconds)
        .await
        .map_err(client_err)
}

/// Fetch an invite's group metadata without joining. Projected the same way
/// as GetGroupMetadata.
pub async fn preview_invite(
    client: &Client,
    code: &str,
) -> Result<pb::GroupMetadataResponse, WamuxError> {
    let metadata = client
        .groups()
        .get_invite_info(code)
        .await
        .map_err(|err| local_refusal(err, WamuxError::InvalidArgument))?;
    Ok(pb::GroupMetadataResponse {
        metadata: Some(group_metadata_of(&metadata)),
    })
}

/// Set or remove the group photo. An empty `image` means remove: `set_group`
/// asserts (panics) on empty bytes, so empty MUST route to `remove_group`.
pub async fn set_photo(
    client: &Client,
    group: &GroupJid,
    image: Vec<u8>,
) -> Result<(), WamuxError> {
    let spec = if image.is_empty() {
        SetProfilePictureSpec::remove_group(group.as_lib())
    } else {
        SetProfilePictureSpec::set_group(group.as_lib(), image)
    };
    client.execute(spec).await.map_err(client_err)?;
    Ok(())
}

/// Pending requests, one typed entry each (#132).
pub async fn membership_requests(
    client: &Client,
    group: &GroupJid,
) -> Result<pb::MembershipRequestsResponse, WamuxError> {
    let requests = client
        .groups()
        .get_membership_requests(group.as_lib())
        .await
        .map_err(client_err)?;
    Ok(pb::MembershipRequestsResponse {
        requests: requests.iter().map(membership_request_of).collect(),
    })
}

pub async fn approve_membership(
    client: &Client,
    group: &GroupJid,
    participants: &[Jid],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let parts: Vec<whatsapp_rust::Jid> = lib_jids(participants);
    client
        .groups()
        .approve_membership_requests(group.as_lib(), &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn reject_membership(
    client: &Client,
    group: &GroupJid,
    participants: &[Jid],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let parts: Vec<whatsapp_rust::Jid> = lib_jids(participants);
    client
        .groups()
        .reject_membership_requests(group.as_lib(), &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

#[cfg(test)]
mod tests;
