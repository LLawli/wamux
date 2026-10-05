//! `lid_mapping` (#116): the tests that lived inline, moved so the loop can
//! seal them. Same assertions; a query is the domain's `LidPnQuery`.

use wacore::store::traits::LidPnMappingEntry;
use wamux_types::LidPnQuery;
use whatsapp_rust::lid_pn_cache::{LearningSource, LidPnEntry};

use super::{lid_pn_result, stored_mapping};

fn stored(lid: &str, phone: &str) -> LidPnMappingEntry {
    LidPnMappingEntry {
        lid: lid.to_string(),
        phone_number: phone.to_string(),
        created_at: 1_717_932_000,
        updated_at: 1_717_932_001,
        learning_source: "usync".to_string(),
    }
}

fn query(text: &str) -> LidPnQuery {
    LidPnQuery::try_from(text.to_string()).expect("a valid query")
}

#[test]
fn stored_pair_renders_both_sides_as_full_jids() {
    let mapping = stored_mapping(&stored("169815004184633", "5511999000111"));
    assert_eq!(mapping.lid, "169815004184633@lid");
    assert_eq!(mapping.pn, "5511999000111@s.whatsapp.net");
    assert_eq!(mapping.created_at, 1_717_932_000);
    assert_eq!(mapping.learning_source, "usync");
}

// A half-written row must not become the JID "@lid", which parses and would
// then be relayed onward by an edge as if it named someone.
#[test]
fn missing_user_part_stays_empty_not_a_bare_server() {
    let mapping = stored_mapping(&stored("", "5511999000111"));
    assert!(mapping.lid.is_empty(), "got {:?}", mapping.lid);
    assert_eq!(mapping.pn, "5511999000111@s.whatsapp.net");
}

#[test]
fn a_miss_keeps_the_query_and_carries_no_mapping() {
    let result = lid_pn_result(&query("169815004184633@lid"), None);
    assert_eq!(result.query, "169815004184633@lid");
    assert!(!result.found);
    assert!(result.mapping.is_none());
}

// Decided 2026-10-05: the answer echoes the query as the caller wrote it, so
// a legacy spelling comes back as it went, not as the parse prints it.
#[test]
fn a_miss_echoes_the_query_as_sent() {
    let result = lid_pn_result(&query("5511999000111@c.us"), None);
    assert_eq!(result.query, "5511999000111@c.us");
}

#[test]
fn a_hit_relays_the_library_learning_source_verbatim() {
    let entry = LidPnEntry::with_timestamp(
        "169815004184633".to_string(),
        "5511999000111".to_string(),
        1_717_932_000,
        LearningSource::PeerLidMessage,
    );
    let result = lid_pn_result(&query("169815004184633@lid"), Some(entry));
    assert!(result.found);
    let mapping = result.mapping.expect("a hit carries the pair");
    assert_eq!(mapping.pn, "5511999000111@s.whatsapp.net");
    assert_eq!(mapping.learning_source, "peer_lid_message");
}
