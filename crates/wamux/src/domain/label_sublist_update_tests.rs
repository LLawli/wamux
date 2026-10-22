//! #215: a label sublist entry, routed through `map_event`, so each test also
//! proves the event does not fall into the RawEvent catch-all. Hand built from
//! the library's type: a sublist belongs to a business account, which the live
//! accounts are not.

use std::str::FromStr;

use wacore::types::events::{Event, LabelSublistChange, LabelSublistUpdate};
use whatsapp_rust::Jid;

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;
use crate::proto::v1::label_sublist_update::Change;

const CHAT_PN: &str = "5511900000215@s.whatsapp.net";
const CHAT_LID: &str = "100000000000215@lid";

/// The instant `ms` after the epoch, as the library's builders take it.
macro_rules! at_ms {
    ($ms:expr) => {
        wacore::time::from_millis($ms).expect("test instant")
    };
}

fn sublist_event(
    chat: &str,
    change: LabelSublistChange,
    action_timestamp: Option<i64>,
    from_full_sync: bool,
) -> Event {
    Event::LabelSublistUpdate(
        LabelSublistUpdate::builder()
            .predefined_id(-3)
            .chat_jid(Jid::from_str(chat).expect("test jid"))
            .change(change)
            .maybe_action_timestamp(action_timestamp.map(|ms| at_ms!(ms)))
            .from_full_sync(from_full_sync)
            .build(),
    )
}

fn sublist_of(event: Event) -> pb::LabelSublistUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::LabelSublist(update)] => update.clone(),
        other => panic!("expected one LabelSublistUpdate, got {other:?}"),
    }
}

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

#[test]
fn an_upsert_carries_its_key_and_the_new_sublist_id() {
    let update = sublist_of(sublist_event(
        CHAT_PN,
        LabelSublistChange::Upsert { sub_list_id: 7 },
        Some(1_792_000_000_789),
        false,
    ));
    assert_eq!(update.predefined_id, -3, "a signed id crosses as sent");
    assert_eq!(update.chat, wire(CHAT_PN));
    assert_eq!(update.action_timestamp, Some(1_792_000_000_789));
    assert!(!update.from_full_sync);
    assert_eq!(update.change, Some(Change::UpsertSubListId(7)));
}

#[test]
fn an_upsert_to_sublist_zero_is_a_value_not_a_removal() {
    let update = sublist_of(sublist_event(
        CHAT_PN,
        LabelSublistChange::Upsert { sub_list_id: 0 },
        None,
        false,
    ));
    assert_eq!(update.change, Some(Change::UpsertSubListId(0)));
}

#[test]
fn a_removal_is_its_own_case() {
    let update = sublist_of(sublist_event(
        CHAT_LID,
        LabelSublistChange::Remove,
        None,
        true,
    ));
    assert_eq!(update.change, Some(Change::Remove(pb::Empty {})));
    assert!(update.from_full_sync);
    assert_eq!(update.chat, wire(CHAT_LID), "a LID chat is not rewritten");
}

#[test]
fn an_absent_action_time_is_unset_and_zero_is_a_time() {
    let absent = sublist_of(sublist_event(
        CHAT_PN,
        LabelSublistChange::Remove,
        None,
        false,
    ));
    assert_eq!(absent.action_timestamp, None);
    let epoch = sublist_of(sublist_event(
        CHAT_PN,
        LabelSublistChange::Remove,
        Some(0),
        false,
    ));
    assert_eq!(epoch.action_timestamp, Some(0));
}
