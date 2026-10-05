//! Reads of the LID<->phone-number mapping the library already keeps.
//!
//! Relay-pure: two ways OUT of the library's own mapping, no policy. The core
//! never invents a pair, never rewrites a JID onto the other namespace, and
//! never decides which of the two a chat "really is" -- an unknown pair answers
//! `found=false` and the edge composes from there (issue #1).

use std::sync::Arc;

use wacore::store::traits::{Backend, LidPnMappingEntry};
use wamux_types::LidPnQuery;
use whatsapp_rust::lid_pn_cache::LidPnEntry;
use whatsapp_rust::{Client, Server};

use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// Batched LID<->PN lookup against the live client: in-memory cache first,
/// durable store on miss (the library's `get_lid_pn_entry` is cache-aside).
/// One result per query, in request order, so the caller can zip them back.
pub async fn resolve_lid_pn(
    client: &Client,
    queries: &[LidPnQuery],
) -> Result<Vec<pb::LidPnResult>, WamuxError> {
    let mut results = Vec::with_capacity(queries.len());
    for query in queries {
        let entry = client
            .get_lid_pn_entry(query.jid.as_lib())
            .await
            .map_err(client_err)?;
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
fn lid_pn_result(query: &LidPnQuery, entry: Option<LidPnEntry>) -> pb::LidPnResult {
    pb::LidPnResult {
        // The text as sent (#116), not the parsed jid: a `@c.us` query comes back `@c.us`.
        query: query.query.clone(),
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
#[path = "lid_mapping_tests.rs"]
mod tests;
