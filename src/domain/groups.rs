//! Group management helpers over `client.groups()`.

use std::sync::Arc;

use wacore::iq::contacts::SetProfilePictureSpec;
use wacore::iq::groups::{
    GroupCreateOptions, GroupDescription, GroupParticipantOptions, GroupSubject,
    ParticipantChangeResponse,
};
use whatsapp_rust::Client;
use whatsapp_rust::features::{GroupParticipant, MembershipRequest, PreviousDescription};

use crate::domain::jid_parse::{parse_jid, parse_jids};
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

fn invalid<E: std::fmt::Display>(e: E) -> WamuxError {
    WamuxError::InvalidArgument(e.to_string())
}

pub async fn create_group(
    client: &Client,
    subject: &str,
    participants: &[String],
) -> Result<pb::GroupJidResponse, WamuxError> {
    let parts = parse_jids(participants)?;
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
        group_jid: result.metadata.id.to_string(),
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
        "subject": md.subject,
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
        jid: c.jid.to_string(),
        // Absent server attrs relay as the proto3 default (empty string), the
        // same rule the rest of the crate uses for "not set".
        status: c.status.unwrap_or_default(),
        error: c.error.unwrap_or_default(),
        phone_number: c.phone_number.map(|j| j.to_string()).unwrap_or_default(),
        username: c.username.unwrap_or_default(),
        add_request: c.add_request.map(|a| pb::AddRequestInfo {
            code: a.code,
            expiration: a.expiration,
        }),
    }
}

pub async fn add_participants(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .add_participants(&jid, &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn remove_participants(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .remove_participants(&jid, &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn promote(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .promote_participants(&jid, &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn demote(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .demote_participants(&jid, &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn set_subject(client: &Client, group: &str, subject: &str) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    let subject = GroupSubject::new(subject).map_err(invalid)?;
    client
        .groups()
        .set_subject(&jid, subject)
        .await
        .map_err(client_err)
}

pub async fn set_description(
    client: &Client,
    group: &str,
    description: &str,
) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    let description = GroupDescription::new(description).map_err(invalid)?;
    client
        .groups()
        // 0.7 types the third argument as `PreviousDescription`, the server's
        // optimistic-concurrency token. `Resolve` (the lib default) reads the
        // current description id first; it is the only variant that is correct
        // when the caller holds no metadata, and the RPC gives the edge no way
        // to pass one. `Absent` would 409 on any group that already has a
        // description, so it is not the safe literal translation of the old
        // `None` it looks like.
        .set_description(&jid, Some(description), PreviousDescription::Resolve)
        .await
        .map_err(client_err)
}

pub async fn get_metadata(
    client: &Client,
    group: &str,
) -> Result<pb::GroupMetadataResponse, WamuxError> {
    let jid = parse_jid(group)?;
    let metadata = client
        .groups()
        .get_metadata(&jid)
        .await
        .map_err(client_err)?;
    Ok(pb::GroupMetadataResponse {
        metadata: metadata_json(&metadata),
    })
}

pub async fn invite_link(
    client: &Client,
    group: &str,
    reset: bool,
) -> Result<pb::InviteLinkResponse, WamuxError> {
    let jid = parse_jid(group)?;
    let link = client
        .groups()
        .get_invite_link(&jid, reset)
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
        group_jid,
        metadata: Vec::new(),
    })
}

pub async fn leave(client: &Client, group: &str) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    client.groups().leave(&jid).await.map_err(client_err)
}

/// List the groups the account participates in (summary + projected metadata).
/// `get_participating` returns a non-Send future, so it runs isolated.
pub async fn list_participating(client: Arc<Client>) -> Result<Vec<pb::GroupSummary>, WamuxError> {
    let groups = crate::domain::isolate::run_isolated(move || async move {
        client.groups().get_participating().await
    })
    .await?;
    let mut summaries: Vec<pb::GroupSummary> = groups
        .values()
        .map(|m| pb::GroupSummary {
            jid: m.id.to_string(),
            subject: m.subject.clone(),
            participants: m.participants.len() as u32,
            metadata: metadata_json(m),
        })
        .collect();
    summaries.sort_by(|a, b| a.subject.cmp(&b.subject));
    Ok(summaries)
}

/// ListGroups' projection: one summary per group, sorted by subject, each
/// carrying the full `metadata_json` (roster included). An absent subject is
/// "", as 0.7.0 parsed it. Pure, so the contract is testable without a client.
pub fn group_summaries(groups: Vec<whatsapp_rust::GroupMetadata>) -> Vec<pb::GroupSummary> {
    let _ = groups;
    todo!("#30: the projection list_participating does today, over Option subjects")
}

pub async fn set_announce(client: &Client, group: &str, announce: bool) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    client
        .groups()
        .set_announce(&jid, announce)
        .await
        .map_err(client_err)
}

pub async fn set_locked(client: &Client, group: &str, locked: bool) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    client
        .groups()
        .set_locked(&jid, locked)
        .await
        .map_err(client_err)
}

/// Group-level disappearing-messages timer (0 disables). Distinct from the
/// per-message ephemeral flag the edge sets on SendText/SendMedia.
pub async fn set_ephemeral(
    client: &Client,
    group: &str,
    expiration_seconds: u32,
) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    client
        .groups()
        .set_ephemeral(&jid, expiration_seconds)
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
pub async fn set_photo(client: &Client, group: &str, image: Vec<u8>) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    let spec = if image.is_empty() {
        SetProfilePictureSpec::remove_group(&jid)
    } else {
        SetProfilePictureSpec::set_group(&jid, image)
    };
    client.execute(spec).await.map_err(client_err)?;
    Ok(())
}

/// MembershipRequest derives Serialize, so relay the Vec verbatim as JSON
/// bytes (mirrors metadata_json: the edge owns any shaping/filtering).
pub async fn membership_requests(
    client: &Client,
    group: &str,
) -> Result<pb::MembershipRequestsResponse, WamuxError> {
    let jid = parse_jid(group)?;
    let requests = client
        .groups()
        .get_membership_requests(&jid)
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
    group: &str,
    participants: &[String],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .approve_membership_requests(&jid, &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

pub async fn reject_membership(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<pb::ParticipantsResponse, WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .reject_membership_requests(&jid, &parts)
        .await
        .map(participant_changes)
        .map_err(client_err)
}

#[cfg(test)]
mod tests;
