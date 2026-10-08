//! #153 (part 2b of #141): this account's own push name, routed through
//! `map_event`, so each test also proves the event no longer falls into the
//! RawEvent catch-all. The rename is the one the user made on the trabalho
//! phone on 2026-10-08 at 19:58Z (read off the production subscription; the
//! names are replaced, the shape is kept: a shorter name).

use wacore::types::events::{Event, SelfPushNameUpdated};

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;

/// The one SelfPushNameUpdate the event maps to; anything else is a failure.
fn self_push_name_of(event: Event) -> pb::SelfPushNameUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::SelfPushName(update)] => update.clone(),
        other => panic!("expected one SelfPushNameUpdate, got {other:?}"),
    }
}

fn rename_event(old_name: &str, new_name: &str) -> Event {
    Event::SelfPushNameUpdated(
        SelfPushNameUpdated::builder()
            .from_server(true)
            .old_name(old_name.to_string())
            .new_name(new_name.to_string())
            .build(),
    )
}

#[test]
fn self_push_name_relays_the_captured_rename() {
    let expected = pb::SelfPushNameUpdate {
        old_name: "Equipe Comercial".to_string(),
        new_name: "Equipe".to_string(),
    };
    assert_eq!(
        self_push_name_of(rename_event("Equipe Comercial", "Equipe")),
        expected
    );
}

/// The first name after pairing: the library had none, so the old one is
/// empty, which crosses as the empty string.
#[test]
fn self_push_name_relays_an_empty_old_name() {
    let update = self_push_name_of(rename_event("", "Equipe"));
    assert_eq!(
        (update.old_name.as_str(), update.new_name.as_str()),
        ("", "Equipe")
    );
}
