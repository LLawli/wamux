//! Unit tests for `domain::groups` (projections only; no client).

use std::str::FromStr;

use wacore::types::message::AddressingMode;
use whatsapp_rust::features::ParticipantType;
use whatsapp_rust::{GroupMetadata, Jid};

use super::*;

#[test]
fn metadata_json_round_trips_key_fields() {
    let member = Jid::from_str("5511999999999@s.whatsapp.net").unwrap();
    let md = GroupMetadata {
        id: Jid::from_str("120363001234567890@g.us").unwrap(),
        subject: Some("Test Group".to_string()),
        description: Some("a description".to_string()),
        participants: vec![GroupParticipant {
            jid: member.clone(),
            phone_number: None,
            lid: None,
            username: None,
            participant_type: ParticipantType::Member,
            details: None,
        }],
        ..GroupMetadata::default()
    };
    let bytes = metadata_json(&md);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["id"], "120363001234567890@g.us");
    assert_eq!(value["subject"], "Test Group");
    assert_eq!(value["description"], "a description");
    assert_eq!(value["participants"][0]["jid"], member.to_string());
}

// REGRESSION (issue #1): in a LID-addressed group every participant jid is
// a `@lid` and `phone_number` is the only PN the roster carries. Flattening
// a participant to its jid string dropped that, and the admin role with it.
#[test]
fn participant_keeps_its_phone_number_and_role() {
    let lid = Jid::from_str("169815004184633@lid").unwrap();
    let pn = Jid::from_str("5511999000111@s.whatsapp.net").unwrap();
    let md = GroupMetadata {
        id: Jid::from_str("120363001234567890@g.us").unwrap(),
        addressing_mode: AddressingMode::Lid,
        participants: vec![GroupParticipant {
            jid: lid.clone(),
            phone_number: Some(pn.clone()),
            lid: Some(lid.clone()),
            username: Some("fulano".into()),
            participant_type: ParticipantType::SuperAdmin,
            details: None,
        }],
        ..GroupMetadata::default()
    };
    let value: serde_json::Value = serde_json::from_slice(&metadata_json(&md)).unwrap();
    assert_eq!(value["addressing_mode"], "lid");
    let participant = &value["participants"][0];
    assert_eq!(participant["jid"], lid.to_string());
    assert_eq!(participant["phone_number"], pn.to_string());
    assert_eq!(participant["lid"], lid.to_string());
    assert_eq!(participant["username"], "fulano");
    assert_eq!(participant["type"], "superadmin");
}

// A PN-addressed group sends no `phone_number`: `jid` already is the phone,
// and the core must leave the absence visible rather than copying the jid
// into it (that would be the edge guessing, done in the core).
#[test]
fn absent_phone_number_stays_null() {
    let md = GroupMetadata {
        id: Jid::from_str("120363001234567890@g.us").unwrap(),
        participants: vec![GroupParticipant {
            jid: Jid::from_str("5511999000111@s.whatsapp.net").unwrap(),
            phone_number: None,
            lid: None,
            username: None,
            participant_type: ParticipantType::Member,
            details: None,
        }],
        ..GroupMetadata::default()
    };
    let value: serde_json::Value = serde_json::from_slice(&metadata_json(&md)).unwrap();
    assert!(value["participants"][0]["phone_number"].is_null());
    assert!(value["participants"][0]["lid"].is_null());
    assert!(value["participants"][0]["username"].is_null());
    assert_eq!(value["participants"][0]["type"], "member");
    assert_eq!(value["addressing_mode"], "pn");
}

// None description must serialize as JSON null (not be omitted), so the
// edge can distinguish "no description" without schema guessing.
#[test]
fn metadata_json_none_description_is_null() {
    let md = GroupMetadata {
        id: Jid::from_str("120363009876543210@g.us").unwrap(),
        subject: Some("No Desc".to_string()),
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

// main made `GroupMetadata.subject` an `Option` (#30, upstream #1513): the
// protocol may omit it. 0.7.0 parsed an absent subject as "", and the wire
// contract (`GroupSummary.subject`, the metadata JSON) carried that string. A
// `null` appearing in the JSON would be a contract change the edge never asked
// for, so absent keeps projecting as "".
#[test]
fn absent_subject_projects_as_empty_string_like_0_7_0() {
    let md = GroupMetadata {
        id: Jid::from_str("120363009876543210@g.us").unwrap(),
        subject: None,
        ..GroupMetadata::default()
    };
    let value: serde_json::Value = serde_json::from_slice(&metadata_json(&md)).unwrap();
    assert_eq!(value["subject"], "");
}

fn group(id: &str, subject: Option<&str>, members: usize) -> GroupMetadata {
    GroupMetadata {
        id: Jid::from_str(id).unwrap(),
        subject: subject.map(str::to_string),
        participants: (0..members)
            .map(|i| GroupParticipant {
                jid: Jid::from_str(&format!("55119990001{i:02}@s.whatsapp.net")).unwrap(),
                phone_number: None,
                lid: None,
                username: None,
                participant_type: ParticipantType::Member,
                details: None,
            })
            .collect(),
        ..GroupMetadata::default()
    }
}

// ListGroups keeps its 0.7.0 shape on main (#30): upstream's new
// `list_participating` returns overviews with no participants and no
// description, so the core issues the full `GroupParticipatingIq` and projects
// full metadata exactly as before. This pins the projection: sorted by subject,
// the participant count, and the whole metadata JSON with the roster in it.
#[test]
fn group_summaries_project_full_metadata_sorted_by_subject() {
    let summaries = group_summaries(vec![
        group("120363000000000002@g.us", Some("Zeta"), 3),
        group("120363000000000001@g.us", Some("Alfa"), 1),
    ]);
    let subjects: Vec<&str> = summaries.iter().map(|s| s.subject.as_str()).collect();
    assert_eq!(subjects, ["Alfa", "Zeta"]);

    let zeta = &summaries[1];
    assert_eq!(zeta.jid, "120363000000000002@g.us");
    assert_eq!(zeta.participants, 3);
    let value: serde_json::Value = serde_json::from_slice(&zeta.metadata).unwrap();
    assert_eq!(value["participants"].as_array().unwrap().len(), 3);
    assert_eq!(value["subject"], "Zeta");
}

#[test]
fn a_group_without_a_subject_lists_with_an_empty_one() {
    let summaries = group_summaries(vec![group("120363000000000003@g.us", None, 0)]);
    assert_eq!(summaries[0].subject, "");
}
