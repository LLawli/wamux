//! Group management helpers over `client.groups()`.

use std::sync::Arc;

use wacore::iq::contacts::SetProfilePictureSpec;
use wacore::iq::groups::{
    GroupCreateOptions, GroupDescription, GroupParticipantOptions, GroupSubject,
};
use whatsapp_rust::Client;
use whatsapp_rust::features::MembershipRequest;

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

/// GroupMetadata isn't Serialize, so project the load-bearing fields into JSON.
fn metadata_json(md: &whatsapp_rust::GroupMetadata) -> Vec<u8> {
    let participants: Vec<String> = md.participants.iter().map(|p| p.jid.to_string()).collect();
    serde_json::to_vec(&serde_json::json!({
        "id": md.id.to_string(),
        "subject": md.subject,
        "description": md.description,
        "participants": participants,
    }))
    .unwrap_or_default()
}

pub async fn add_participants(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .add_participants(&jid, &parts)
        .await
        .map_err(client_err)?;
    Ok(())
}

pub async fn remove_participants(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .remove_participants(&jid, &parts)
        .await
        .map_err(client_err)?;
    Ok(())
}

pub async fn promote(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .promote_participants(&jid, &parts)
        .await
        .map_err(client_err)
}

pub async fn demote(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .demote_participants(&jid, &parts)
        .await
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
        .set_description(&jid, Some(description), None)
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

/// The per-participant change responses are advisory; the core drops them and
/// returns Empty (the edge re-reads state via GetMembershipRequests if needed).
pub async fn approve_membership(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .approve_membership_requests(&jid, &parts)
        .await
        .map_err(client_err)?;
    Ok(())
}

pub async fn reject_membership(
    client: &Client,
    group: &str,
    participants: &[String],
) -> Result<(), WamuxError> {
    let jid = parse_jid(group)?;
    let parts = parse_jids(participants)?;
    client
        .groups()
        .reject_membership_requests(&jid, &parts)
        .await
        .map_err(client_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use whatsapp_rust::features::{GroupParticipant, ParticipantType};
    use whatsapp_rust::{GroupMetadata, Jid};

    use super::*;

    #[test]
    fn metadata_json_round_trips_key_fields() {
        let member = Jid::from_str("5511999999999@s.whatsapp.net").unwrap();
        let md = GroupMetadata {
            id: Jid::from_str("120363001234567890@g.us").unwrap(),
            subject: "Test Group".to_string(),
            description: Some("a description".to_string()),
            participants: vec![GroupParticipant {
                jid: member.clone(),
                phone_number: None,
                participant_type: ParticipantType::Member,
            }],
            ..GroupMetadata::default()
        };
        let bytes = metadata_json(&md);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["id"], "120363001234567890@g.us");
        assert_eq!(value["subject"], "Test Group");
        assert_eq!(value["description"], "a description");
        assert_eq!(value["participants"][0], member.to_string());
    }

    // None description must serialize as JSON null (not be omitted), so the
    // edge can distinguish "no description" without schema guessing.
    #[test]
    fn metadata_json_none_description_is_null() {
        let md = GroupMetadata {
            id: Jid::from_str("120363009876543210@g.us").unwrap(),
            subject: "No Desc".to_string(),
            ..GroupMetadata::default()
        };
        let value: serde_json::Value = serde_json::from_slice(&metadata_json(&md)).unwrap();
        assert!(value["description"].is_null());
        assert_eq!(value["participants"].as_array().unwrap().len(), 0);
    }

    // We relay MembershipRequest's own Serialize verbatim (relay-pure). Jid
    // serializes to a structured object ({user, server, ...}), not a string, so
    // assert on the component the edge keys off (user) rather than a flat jid string.
    #[test]
    fn requests_json_projects_jid_and_time() {
        let reqs = vec![
            MembershipRequest {
                jid: Jid::from_str("5511999999999@s.whatsapp.net").unwrap(),
                request_time: Some(1718500000),
            },
            MembershipRequest {
                jid: Jid::from_str("5511888888888@s.whatsapp.net").unwrap(),
                request_time: None,
            },
        ];
        let value: serde_json::Value = serde_json::from_slice(&requests_json(&reqs)).unwrap();
        let arr = value.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["jid"]["user"], "5511999999999");
        assert_eq!(arr[0]["jid"]["server"], "s.whatsapp.net");
        assert_eq!(arr[0]["request_time"], 1718500000);
        // request_time is skip_serializing_if = None, so it's absent (not null).
        assert!(arr[1].get("request_time").is_none());
    }
}
