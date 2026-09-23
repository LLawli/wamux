//! The two channel-metadata MEX requests (#30). #1372 was a `400` from a
//! persisted query sent with a declared variable missing; main's `MexRequest`
//! now carries the declared set, so the omission is checkable without a socket.

use std::str::FromStr;

use wacore::iq::mex_operations::{fetch_all_newsletters_metadata, fetch_newsletter};

use super::*;

const CHANNEL: &str = "120363144038483540@newsletter";

#[test]
fn the_list_request_declares_and_carries_every_variable() {
    let request = list_subscribed_request();
    assert_eq!(request.doc.name, fetch_all_newsletters_metadata::NAME);
    assert_eq!(request.doc.id, fetch_all_newsletters_metadata::DOC_ID);
    assert_eq!(
        request.declared_variables,
        fetch_all_newsletters_metadata::VARIABLE_KEYS
    );
    assert_eq!(
        request.missing_variables().unwrap(),
        Vec::<&str>::new(),
        "a declared variable left out is the 400 of #1372"
    );
}

#[test]
fn the_metadata_request_declares_and_carries_every_variable() {
    let request = get_metadata_request(&Jid::from_str(CHANNEL).unwrap());
    assert_eq!(request.doc.name, fetch_newsletter::NAME);
    assert_eq!(request.doc.id, fetch_newsletter::DOC_ID);
    assert_eq!(request.declared_variables, fetch_newsletter::VARIABLE_KEYS);
    assert_eq!(request.missing_variables().unwrap(), Vec::<&str>::new());
}

// The values measured to work against the live accounts (2026-09) must not
// drift while the request moves onto the generated constants.
#[test]
fn the_metadata_request_keeps_the_values_measured_live() {
    let request = get_metadata_request(&Jid::from_str(CHANNEL).unwrap());
    let vars = &request.variables;
    assert_eq!(vars["input"]["key"], CHANNEL);
    assert_eq!(vars["input"]["type"], "JID");
    assert_eq!(vars["input"]["view_role"], "GUEST");
    assert_eq!(vars["fetch_viewer_metadata"], true);
    assert_eq!(vars["fetch_full_image"], true);
    assert_eq!(vars["fetch_creation_time"], true);
}

// The doc ids production verified. If the generated module ever rotates one,
// this fails and says so, instead of the server answering 400 in production.
#[test]
fn the_generated_doc_ids_are_the_ones_verified_live() {
    assert_eq!(fetch_all_newsletters_metadata::DOC_ID, "25399611239711790");
    assert_eq!(fetch_newsletter::DOC_ID, "27456920720571478");
}
