//! Group management helpers over `client.groups()`.

use std::sync::Arc;

use wacore::iq::contacts::SetProfilePictureSpec;
use wacore::iq::groups::{
    GroupCreateOptions, GroupDescription, GroupParticipantOptions, GroupParticipatingIq,
    GroupSubject, ParticipantChangeResponse,
};
use wamux_types::{Jid, relay_jid};
use whatsapp_rust::Client;
use whatsapp_rust::features::{GroupParticipant, MembershipRequest, PreviousDescription};

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
        .map_err(client_err)?;
    Ok(pb::GroupJidResponse {
        group: relay_jid(result.metadata.id.to_string()),
        metadata: metadata_json(&result.metadata),
    })
}

/// GroupMetadata isn't Serialize (unlike MembershipRequest, which we relay
/// verbatim), so the JSON is hand-built -- and hand-picking is exactly what
/// dropped each participant's identity here. A LID-addressed group hands back
/// every member as a `@lid` with `phone_number` alongside, so flattening a
/// participant to its jid string threw away the answer to "who is this @lid"
/// for the whole roster, plus who is admin (issue #1).
fn metadata_json(md: &whatsapp_rust::GroupMetadata) -> Vec<u8> {
    let participants: Vec<serde_json::Value> =
        md.participants.iter().map(participant_json).collect();
    serde_json::to_vec(&serde_json::json!({
        "id": md.id.to_string(),
        // main made `subject` an `Option<String>` (upstream #1513, #30): the
        // protocol may omit it. 0.7.0 parsed an absent subject as "", and the
        // wire contract already promises that string, so absence keeps
        // projecting as "" rather than a `null` the edge never asked for.
        "subject": md.subject.clone().unwrap_or_default(),
        "description": md.description,
        // Which namespace the roster is addressed in, so the edge knows whether
        // `jid` is a `@lid` before it tries to name anyone.
        "addressing_mode": md.addressing_mode.as_str(),
        "participants": participants,
    }))
    .unwrap_or_default()
}

/// One participant, whole: the jid the group addresses them by, the phone jid
/// and the lid the server sent alongside it (either can be absent, depending on
/// the group's addressing mode), the Meta username when the roster carries one,
/// and the admin role.
///
/// `lid` and `username` only exist from whatsapp-rust 0.7 on. Both stay null
/// rather than being guessed: the group roster is one of the few places the
/// server volunteers a username at all (a received message never carries one),
/// so relaying it here is the difference between the edge knowing an identity
/// and having to go ask for it.
fn participant_json(p: &GroupParticipant) -> serde_json::Value {
    serde_json::json!({
        "jid": p.jid.to_string(),
        "phone_number": p.phone_number.as_ref().map(|j| j.to_string()),
        "lid": p.lid.as_ref().map(|j| j.to_string()),
        "username": p.username.as_ref().map(|u| u.to_string()),
        "type": p.participant_type.as_str(),
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
        jid: relay_jid(c.jid.to_string()),
        // Absent server attrs relay as the proto3 default (empty string), the
        // same rule the rest of the crate uses for "not set".
        status: c.status.unwrap_or_default(),
        error: c.error.unwrap_or_default(),
        phone_number: c
            .phone_number
            .and_then(|phone| relay_jid(phone.to_string())),
        username: c.username.unwrap_or_default(),
        add_request: c.add_request.map(|a| pb::AddRequestInfo {
            code: a.code,
            expiration: a.expiration,
        }),
    }
}

pub async fn add_participants(
    client: &Client,
    group: &Jid,
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
    group: &Jid,
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
    group: &Jid,
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
    group: &Jid,
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

pub async fn set_subject(client: &Client, group: &Jid, subject: &str) -> Result<(), WamuxError> {
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
    group: &Jid,
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
    group: &Jid,
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
        metadata: metadata_json(&metadata),
    })
}

pub async fn invite_link(
    client: &Client,
    group: &Jid,
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
        .map_err(client_err)?;
    let group_jid = match result {
        whatsapp_rust::JoinGroupResult::Joined(jid)
        | whatsapp_rust::JoinGroupResult::PendingApproval(jid) => jid.to_string(),
    };
    Ok(pb::GroupJidResponse {
        group: relay_jid(group_jid),
        metadata: Vec::new(),
    })
}

pub async fn leave(client: &Client, group: &Jid) -> Result<(), WamuxError> {
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
/// carrying the full `metadata_json` (roster included). An absent subject is
/// "", as 0.7.0 parsed it. Pure, so the contract is testable without a client.
pub fn group_summaries(groups: Vec<whatsapp_rust::GroupMetadata>) -> Vec<pb::GroupSummary> {
    let mut summaries: Vec<pb::GroupSummary> = groups
        .iter()
        .map(|m| pb::GroupSummary {
            jid: relay_jid(m.id.to_string()),
            subject: m.subject.clone().unwrap_or_default(),
            participants: m.participants.len() as u32,
            metadata: metadata_json(m),
        })
        .collect();
    summaries.sort_by(|a, b| a.subject.cmp(&b.subject));
    summaries
}

pub async fn set_announce(client: &Client, group: &Jid, announce: bool) -> Result<(), WamuxError> {
    client
        .groups()
        .set_announce(group.as_lib(), announce)
        .await
        .map_err(client_err)
}

pub async fn set_locked(client: &Client, group: &Jid, locked: bool) -> Result<(), WamuxError> {
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
    group: &Jid,
    expiration_seconds: u32,
) -> Result<(), WamuxError> {
    client
        .groups()
        .set_ephemeral(group.as_lib(), expiration_seconds)
        .await
        .map_err(client_err)
}

/// Fetch an invite's group metadata without joining. Projected with the same
/// `metadata_json` helper as GetGroupMetadata.
pub async fn preview_invite(
    client: &Client,
    code: &str,
) -> Result<pb::GroupMetadataResponse, WamuxError> {
    let metadata = client
        .groups()
        .get_invite_info(code)
        .await
        .map_err(client_err)?;
    Ok(pb::GroupMetadataResponse {
        metadata: metadata_json(&metadata),
    })
}

/// Set or remove the group photo. An empty `image` means remove: `set_group`
/// asserts (panics) on empty bytes, so empty MUST route to `remove_group`.
pub async fn set_photo(client: &Client, group: &Jid, image: Vec<u8>) -> Result<(), WamuxError> {
    let spec = if image.is_empty() {
        SetProfilePictureSpec::remove_group(group.as_lib())
    } else {
        SetProfilePictureSpec::set_group(group.as_lib(), image)
    };
    client.execute(spec).await.map_err(client_err)?;
    Ok(())
}

/// MembershipRequest derives Serialize, so relay the Vec verbatim as JSON
/// bytes (mirrors metadata_json: the edge owns any shaping/filtering).
pub async fn membership_requests(
    client: &Client,
    group: &Jid,
) -> Result<pb::MembershipRequestsResponse, WamuxError> {
    let requests = client
        .groups()
        .get_membership_requests(group.as_lib())
        .await
        .map_err(client_err)?;
    Ok(pb::MembershipRequestsResponse {
        requests: requests_json(&requests),
    })
}

/// Project pending membership requests to a JSON array of {jid, request_time}.
fn requests_json(requests: &[MembershipRequest]) -> Vec<u8> {
    serde_json::to_vec(requests).unwrap_or_default()
}

pub async fn approve_membership(
    client: &Client,
    group: &Jid,
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
    group: &Jid,
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
