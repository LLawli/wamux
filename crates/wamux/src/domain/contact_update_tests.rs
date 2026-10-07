//! #133: the typed projection of a library `ContactUpdate`, asserted as the
//! whole message. It used to cross as the library's JSON in `raw`.

use std::str::FromStr;

use wacore::types::events::ContactUpdate;
use whatsapp_rust::Jid;
use whatsapp_rust::waproto::whatsapp::sync_action_value::ContactAction;

use super::*;

const CONTACT: &str = "100000000000002@lid";
const MUTATION_SECS: i64 = 1_717_932_000;

/// The instant `seconds` after the epoch, as the library's builders take it.
/// A macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at {
    ($seconds:expr) => {
        wacore::time::from_secs($seconds).expect("test instant")
    };
}

/// Every field of the sync action set. The generated struct is only built by
/// assignment here, which is what clippy's lint below would rewrite.
#[allow(clippy::field_reassign_with_default)]
fn every_action_field() -> ContactAction {
    let mut action = ContactAction::default();
    action.full_name = Some("Fulano de Tal".to_string());
    action.first_name = Some("Fulano".to_string());
    action.lid_jid = Some(CONTACT.to_string());
    action.save_on_primary_addressbook = Some(true);
    action.pn_jid = Some("5511900000002@s.whatsapp.net".to_string());
    action.username = Some("fulano".to_string());
    action
}

#[test]
fn contact_update_maps_every_field() {
    let update = ContactUpdate::builder()
        .jid(Jid::from_str(CONTACT).expect("test jid"))
        .timestamp(at!(MUTATION_SECS))
        .action_timestamp(at!(MUTATION_SECS - 5))
        .action(Box::new(every_action_field()))
        .from_full_sync(true)
        .build();
    let expected = pb::ContactUpdate {
        jid: Some(pb::Jid {
            value: CONTACT.to_string(),
        }),
        timestamp: MUTATION_SECS * 1000,
        action_timestamp: Some((MUTATION_SECS - 5) * 1000),
        from_full_sync: true,
        action: Some(pb::ContactAction {
            full_name: Some("Fulano de Tal".to_string()),
            first_name: Some("Fulano".to_string()),
            lid: Some(pb::Jid {
                value: CONTACT.to_string(),
            }),
            save_on_primary_addressbook: Some(true),
            pn: Some(pb::Jid {
                value: "5511900000002@s.whatsapp.net".to_string(),
            }),
            username: Some("fulano".to_string()),
        }),
    };
    assert_eq!(contact_update_of(&update), expected);
}

/// Upstream #1562: with no timestamp in the mutation, `timestamp` is a
/// fallback and only an unset `action_timestamp` says so. An empty action
/// crosses as an empty message, every field unset.
#[test]
fn contact_update_absent_fields_stay_unset() {
    let update = ContactUpdate::builder()
        .jid(Jid::from_str(CONTACT).expect("test jid"))
        .timestamp(at!(0))
        .action(Box::default())
        .from_full_sync(false)
        .build();
    let expected = pb::ContactUpdate {
        jid: Some(pb::Jid {
            value: CONTACT.to_string(),
        }),
        timestamp: 0,
        action_timestamp: None,
        from_full_sync: false,
        action: Some(pb::ContactAction::default()),
    };
    assert_eq!(contact_update_of(&update), expected);
}

/// An empty `lidJid` / `pnJid` is no jid: unset, never `Jid { value: "" }`.
#[test]
#[allow(clippy::field_reassign_with_default)] // see every_action_field
fn an_empty_jid_string_in_the_action_stays_unset() {
    let mut action = ContactAction::default();
    action.lid_jid = Some(String::new());
    action.pn_jid = Some(String::new());
    let update = ContactUpdate::builder()
        .jid(Jid::from_str(CONTACT).expect("test jid"))
        .timestamp(at!(MUTATION_SECS))
        .action(Box::new(action))
        .from_full_sync(false)
        .build();
    let mapped = contact_update_of(&update).action.expect("an action");
    assert_eq!((mapped.lid, mapped.pn), (None, None));
}
