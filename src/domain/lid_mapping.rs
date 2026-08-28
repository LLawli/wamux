//! Reads of the LID<->phone-number mapping the library already keeps.
//!
//! Relay-pure: two ways OUT of the library's own mapping, no policy. The core
//! never invents a pair, never rewrites a JID onto the other namespace, and
//! never decides which of the two a chat "really is" -- an unknown pair answers
//! `found=false` and the edge composes from there (issue #1).

use std::sync::Arc;

use wacore::store::traits::{Backend, LidPnMappingEntry};
use whatsapp_rust::lid_pn_cache::LidPnEntry;
use whatsapp_rust::{Client, Server};

use crate::domain::jid_parse::parse_jids;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// Batched LID<->PN lookup against the live client: in-memory cache first,
/// durable store on miss (the library's `get_lid_pn_entry` is cache-aside).
/// One result per query, in request order, so the caller can zip them back.
pub async fn resolve_lid_pn(
    client: &Client,
    jids: &[String],
) -> Result<Vec<pb::LidPnResult>, WamuxError> {
    let parsed = parse_jids(jids)?;
    let mut results = Vec::with_capacity(parsed.len());
    for (query, jid) in jids.iter().zip(parsed) {
        let entry = client.get_lid_pn_entry(&jid).await.map_err(client_err)?;
        results.push(lid_pn_result(query, entry));
    }
    Ok(results)
}

/// Every pair persisted for this account's device. Storage-side, so it answers
/// for a disconnected account -- and, being the durable side, it does not see a
/// mapping the client has only cached (the library's offline history replay
/// warms the cache and skips the write).
pub async fn list_lid_mappings(
    backend: Arc<dyn Backend>,
) -> Result<Vec<pb::LidPnMapping>, WamuxError> {
    let entries = backend.get_all_lid_mappings().await?;
    Ok(entries.iter().map(stored_mapping).collect())
}

/// A cache/store hit becomes `found=true` + the pair; a miss keeps the query so
/// the caller can tell which of a batch went unanswered.
fn lid_pn_result(query: &str, entry: Option<LidPnEntry>) -> pb::LidPnResult {
    pb::LidPnResult {
        query: query.to_string(),
        found: entry.is_some(),
        mapping: entry.map(|e| {
            lid_pn_mapping(
                &e.lid,
                &e.phone_number,
                e.created_at,
                e.learning_source.as_str(),
            )
        }),
    }
}

fn stored_mapping(entry: &LidPnMappingEntry) -> pb::LidPnMapping {
    lid_pn_mapping(
        &entry.lid,
        &entry.phone_number,
        entry.created_at,
        &entry.learning_source,
    )
}

fn lid_pn_mapping(lid: &str, phone: &str, created_at: i64, source: &str) -> pb::LidPnMapping {
    pb::LidPnMapping {
        lid: side_jid(lid, Server::Lid),
        pn: side_jid(phone, Server::Pn),
        created_at,
        learning_source: source.to_string(),
    }
}

/// Render one side as a full JID. The store keeps bare user parts and each
/// side's namespace is fixed, so this is rendering, not identity guessing. An
/// empty user part stays empty rather than becoming a bare "@lid".
fn side_jid(user: &str, server: Server) -> String {
    if user.is_empty() {
        return String::new();
    }
    format!("{user}@{}", server.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::lid_pn_cache::LearningSource;

    fn stored(lid: &str, phone: &str) -> LidPnMappingEntry {
        LidPnMappingEntry {
            lid: lid.to_string(),
            phone_number: phone.to_string(),
            created_at: 1_717_932_000,
            updated_at: 1_717_932_001,
            learning_source: "usync".to_string(),
        }
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
        let result = lid_pn_result("169815004184633@lid", None);
        assert_eq!(result.query, "169815004184633@lid");
        assert!(!result.found);
        assert!(result.mapping.is_none());
    }

    #[test]
    fn a_hit_relays_the_library_learning_source_verbatim() {
        let entry = LidPnEntry::with_timestamp(
            "169815004184633".to_string(),
            "5511999000111".to_string(),
            1_717_932_000,
            LearningSource::PeerLidMessage,
        );
        let result = lid_pn_result("169815004184633@lid", Some(entry));
        assert!(result.found);
        let mapping = result.mapping.expect("a hit carries the pair");
        assert_eq!(mapping.pn, "5511999000111@s.whatsapp.net");
        assert_eq!(mapping.learning_source, "peer_lid_message");
    }
}
