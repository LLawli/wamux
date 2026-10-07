//! Unit tests for `domain::groups` (projections only; no client). The
//! metadata projection itself is tested in `domain/group_metadata_tests.rs`
//! (#132); these pin what `groups.rs` builds around it.

use std::str::FromStr;

use whatsapp_rust::features::{GroupParticipant, ParticipantType};
use whatsapp_rust::{GroupMetadata, Jid};

use super::*;
use crate::domain::group_metadata::group_metadata_of;

/// A jid as the core relays it out (#121): the value verbatim.
fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
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
// the participant count, and the whole typed metadata with the roster in it
// (#132: it was JSON).
#[test]
fn group_summaries_project_full_metadata_sorted_by_subject() {
    let zeta_group = group("120363000000000002@g.us", Some("Zeta"), 3);
    let summaries = group_summaries(vec![
        zeta_group.clone(),
        group("120363000000000001@g.us", Some("Alfa"), 1),
    ]);
    let subjects: Vec<&str> = summaries.iter().map(|s| s.subject.as_str()).collect();
    assert_eq!(subjects, ["Alfa", "Zeta"]);

    let zeta = &summaries[1];
    assert_eq!(zeta.jid, wire("120363000000000002@g.us"));
    assert_eq!(zeta.participants, 3);
    assert_eq!(zeta.metadata, Some(group_metadata_of(&zeta_group)));
}

// `GroupSummary.subject` stays a plain string (#132 left it as it was): an
// absent subject lists as "", while the typed metadata beside it keeps the
// absence (unset `subject`).
#[test]
fn a_group_without_a_subject_lists_with_an_empty_one() {
    let summaries = group_summaries(vec![group("120363000000000003@g.us", None, 0)]);
    assert_eq!(summaries[0].subject, "");
    let metadata = summaries[0]
        .metadata
        .clone()
        .expect("metadata is always set");
    assert_eq!(metadata.subject, None);
}

/// The struct is `#[non_exhaustive]`, so it is parsed from the `<participant>`
/// node the server answers with, as the library does.
fn change(phone_number: Option<&str>) -> ParticipantChangeResponse {
    use wacore::protocol::ProtocolNode;
    let mut node = whatsapp_rust::NodeBuilder::new("participant")
        .attr("jid", "100000000000002@lid")
        .attr("type", "200");
    if let Some(pn) = phone_number {
        node = node.attr("phone_number", pn);
    }
    ParticipantChangeResponse::try_from_node(&node.build()).expect("a well-formed participant")
}

// #121: the participant and the phone jid next to it are `Jid`s, relayed as
// the server named them.
#[test]
fn a_participant_change_relays_its_jids() {
    let out = participant_change(change(Some("5511900000002@s.whatsapp.net")));
    assert_eq!(out.jid, wire("100000000000002@lid"));
    assert_eq!(out.phone_number, wire("5511900000002@s.whatsapp.net"));
    assert_eq!(out.status, "200");
}

// A participant the server sent no phone for has an unset `phone_number`,
// never `Jid { value: "" }`.
#[test]
fn a_participant_change_without_a_phone_leaves_it_unset() {
    let out = participant_change(change(None));
    assert_eq!(out.phone_number, None);
}
