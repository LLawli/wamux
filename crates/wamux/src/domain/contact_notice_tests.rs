//! #151 (part 4 of #141): what the server says about a contact, routed through
//! `map_event`, so each test also proves the event no longer falls into the
//! RawEvent catch-all. The picture and disappearing-mode values are the ones
//! the trabalho account produced on 2026-10-09 at 12:29Z to 12:30Z, read off
//! the production subscription (jids anonymized). The others are hand-built in
//! the shapes the library's handlers fill (`handlers/notification/` at
//! 6f07e3a): no live capture carried them.

use std::str::FromStr;

use wacore::stanza::business::BusinessSubscription;
use wacore::types::events::{
    BusinessStatusUpdate, BusinessUpdateType, ContactNumberChanged, ContactSyncRequested,
    ContactUpdated, DisappearingModeChanged, Event, PictureUpdate, UserAboutUpdate,
};
use whatsapp_rust::Jid;

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::contact_notice::Notice;
use crate::proto::v1::event_envelope::Event as PbEvent;

const CONTACT: &str = "100000000000002@lid";
const GROUP: &str = "120363000000000151@g.us";
const ADMIN: &str = "100000000000001@lid";

/// The instant `seconds` after the epoch, as the library's builders take it.
/// A macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at {
    ($seconds:expr) => {
        wacore::time::from_secs($seconds).expect("test instant")
    };
}

fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid")
}

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

/// The one ContactNotice the event maps to; anything else is a failure.
fn notice_of(event: Event) -> Notice {
    match map_event(&event).as_slice() {
        [PbEvent::ContactNotice(pb::ContactNotice { notice: Some(n) })] => n.clone(),
        other => panic!("expected one ContactNotice, got {other:?}"),
    }
}

/// The capture: the trabalho account set a new picture, seen from pessoal.
#[test]
fn picture_relays_the_captured_change() {
    let event = Event::PictureUpdate(
        PictureUpdate::builder()
            .jid(lib(CONTACT))
            .timestamp(at!(1_791_548_999))
            .removed(false)
            .picture_id("1402149940".to_string())
            .build(),
    );
    let expected = Notice::Picture(pb::PictureChange {
        jid: wire(CONTACT),
        author: None,
        timestamp: 1_791_548_999_000,
        removed: false,
        picture_id: Some("1402149940".to_string()),
    });
    assert_eq!(notice_of(event), expected);
}

/// A group's picture removed by an admin: the author crosses, no id.
#[test]
fn picture_removed_from_a_group_carries_the_author() {
    let event = Event::PictureUpdate(
        PictureUpdate::builder()
            .jid(lib(GROUP))
            .author(lib(ADMIN))
            .timestamp(at!(1_791_549_000))
            .removed(true)
            .build(),
    );
    let expected = Notice::Picture(pb::PictureChange {
        jid: wire(GROUP),
        author: wire(ADMIN),
        timestamp: 1_791_549_000_000,
        removed: true,
        picture_id: None,
    });
    assert_eq!(notice_of(event), expected);
}

#[test]
fn about_relays_the_text_as_about() {
    let event = Event::UserAboutUpdate(
        UserAboutUpdate::builder()
            .jid(lib(CONTACT))
            .status("Disponível".to_string())
            .timestamp(at!(1_791_549_100))
            .build(),
    );
    let expected = Notice::About(pb::AboutChange {
        jid: wire(CONTACT),
        about: "Disponível".to_string(),
        timestamp: 1_791_549_100_000,
    });
    assert_eq!(notice_of(event), expected);
}

#[test]
fn profile_relays_the_contact() {
    let event = Event::ContactUpdated(
        ContactUpdated::builder()
            .jid(lib(CONTACT))
            .timestamp(at!(1_791_549_200))
            .build(),
    );
    let expected = Notice::Profile(pb::ContactProfileChange {
        jid: wire(CONTACT),
        timestamp: 1_791_549_200_000,
    });
    assert_eq!(notice_of(event), expected);
}

#[test]
fn number_change_relays_both_numbers_and_lids() {
    let event = Event::ContactNumberChanged(
        ContactNumberChanged::builder()
            .old_jid(lib("5511900000001@s.whatsapp.net"))
            .new_jid(lib("5511900000009@s.whatsapp.net"))
            .old_lid(lib("100000000000001@lid"))
            .new_lid(lib("100000000000009@lid"))
            .timestamp(at!(1_791_549_300))
            .build(),
    );
    let expected = Notice::Number(pb::NumberChange {
        old_pn: wire("5511900000001@s.whatsapp.net"),
        new_pn: wire("5511900000009@s.whatsapp.net"),
        old_lid: wire("100000000000001@lid"),
        new_lid: wire("100000000000009@lid"),
        timestamp: 1_791_549_300_000,
    });
    assert_eq!(notice_of(event), expected);
}

#[test]
fn number_change_without_lids_leaves_them_unset() {
    let event = Event::ContactNumberChanged(
        ContactNumberChanged::builder()
            .old_jid(lib("5511900000001@s.whatsapp.net"))
            .new_jid(lib("5511900000009@s.whatsapp.net"))
            .timestamp(at!(1_791_549_300))
            .build(),
    );
    match notice_of(event) {
        Notice::Number(change) => assert_eq!((change.old_lid, change.new_lid), (None, None)),
        other => panic!("expected number, got {other:?}"),
    }
}

#[test]
fn sync_requested_relays_after_when_present() {
    let with_after = Event::ContactSyncRequested(
        ContactSyncRequested::builder()
            .after(at!(1_791_000_000))
            .timestamp(at!(1_791_549_400))
            .build(),
    );
    let expected = Notice::SyncRequested(pb::ContactSyncRequest {
        after: Some(1_791_000_000_000),
        timestamp: 1_791_549_400_000,
    });
    assert_eq!(notice_of(with_after), expected);
    let without = Event::ContactSyncRequested(
        ContactSyncRequested::builder()
            .timestamp(at!(1_791_549_400))
            .build(),
    );
    match notice_of(without) {
        Notice::SyncRequested(request) => assert_eq!(request.after, None),
        other => panic!("expected sync_requested, got {other:?}"),
    }
}

fn disappearing_event(duration: u32, setting_secs: i64) -> Event {
    Event::DisappearingModeChanged(
        DisappearingModeChanged::builder()
            .from(lib(CONTACT))
            .duration(duration)
            .setting_timestamp(at!(setting_secs))
            .build(),
    )
}

/// The capture: 24 h turned on, then off six seconds later.
#[test]
fn disappearing_mode_relays_the_captured_timer() {
    let on = Notice::DisappearingMode(pb::DisappearingModeChange {
        jid: wire(CONTACT),
        duration_seconds: 86_400,
        setting_timestamp: 1_791_549_052_000,
    });
    assert_eq!(notice_of(disappearing_event(86_400, 1_791_549_052)), on);
    let off = Notice::DisappearingMode(pb::DisappearingModeChange {
        jid: wire(CONTACT),
        duration_seconds: 0,
        setting_timestamp: 1_791_549_058_000,
    });
    assert_eq!(notice_of(disappearing_event(0, 1_791_549_058)), off);
}

fn business_type_of(update_type: BusinessUpdateType) -> i32 {
    let event = Event::BusinessStatusUpdate(
        BusinessStatusUpdate::builder()
            .jid(lib(CONTACT))
            .update_type(update_type)
            .timestamp(at!(1_791_549_500))
            .product_ids(Vec::new())
            .collection_ids(Vec::new())
            .subscriptions(Vec::new())
            .build(),
    );
    match notice_of(event) {
        Notice::Business(change) => change.update_type,
        other => panic!("expected business, got {other:?}"),
    }
}

/// Every library value to its own case; the library's own Unknown is UNKNOWN.
#[test]
fn business_maps_every_update_type() {
    use pb::BusinessUpdateType as T;
    let cases = [
        (BusinessUpdateType::RemovedAsBusiness, T::RemovedAsBusiness),
        (
            BusinessUpdateType::VerifiedNameChanged,
            T::VerifiedNameChanged,
        ),
        (BusinessUpdateType::ProfileUpdated, T::ProfileUpdated),
        (BusinessUpdateType::ProductsUpdated, T::ProductsUpdated),
        (
            BusinessUpdateType::CollectionsUpdated,
            T::CollectionsUpdated,
        ),
        (
            BusinessUpdateType::SubscriptionsUpdated,
            T::SubscriptionsUpdated,
        ),
        (BusinessUpdateType::Unknown, T::Unknown),
    ];
    for (library, wire) in cases {
        assert_eq!(business_type_of(library), wire as i32, "{library:?}");
    }
}

/// Every field, with the subscription times in seconds turned into ms (the
/// library's own test has `expiration_date` 1800000000).
#[test]
fn business_relays_every_field() {
    let subscription = BusinessSubscription {
        id: "sub-1".to_string(),
        status: "active".to_string(),
        expiration_date: Some(1_800_000_000),
        creation_time: Some(1_790_000_000),
    };
    let event = Event::BusinessStatusUpdate(
        BusinessStatusUpdate::builder()
            .jid(lib(CONTACT))
            .update_type(BusinessUpdateType::SubscriptionsUpdated)
            .timestamp(at!(1_791_549_500))
            .target_jid(lib(ADMIN))
            .hash("h4sh".to_string())
            .verified_name("Loja".to_string())
            .product_ids(vec!["p1".to_string()])
            .collection_ids(vec!["c1".to_string(), "c2".to_string()])
            .subscriptions(vec![subscription])
            .build(),
    );
    let expected = Notice::Business(pb::BusinessStatusChange {
        jid: wire(CONTACT),
        update_type: pb::BusinessUpdateType::SubscriptionsUpdated as i32,
        timestamp: 1_791_549_500_000,
        target: wire(ADMIN),
        hash: Some("h4sh".to_string()),
        verified_name: Some("Loja".to_string()),
        product_ids: vec!["p1".to_string()],
        collection_ids: vec!["c1".to_string(), "c2".to_string()],
        subscriptions: vec![pb::BusinessSubscriptionInfo {
            id: "sub-1".to_string(),
            status: "active".to_string(),
            expiration_date: Some(1_800_000_000_000),
            creation_time: Some(1_790_000_000_000),
        }],
    });
    assert_eq!(notice_of(event), expected);
}

/// A nonsense subscription time saturates instead of overflowing.
#[test]
fn business_subscription_times_saturate() {
    let subscription = BusinessSubscription {
        id: "sub-1".to_string(),
        status: "active".to_string(),
        expiration_date: Some(i64::MAX),
        creation_time: None,
    };
    let event = Event::BusinessStatusUpdate(
        BusinessStatusUpdate::builder()
            .jid(lib(CONTACT))
            .update_type(BusinessUpdateType::SubscriptionsUpdated)
            .timestamp(at!(1_791_549_500))
            .product_ids(Vec::new())
            .collection_ids(Vec::new())
            .subscriptions(vec![subscription])
            .build(),
    );
    match notice_of(event) {
        Notice::Business(change) => {
            assert_eq!(change.subscriptions[0].expiration_date, Some(i64::MAX));
            assert_eq!(change.subscriptions[0].creation_time, None);
        }
        other => panic!("expected business, got {other:?}"),
    }
}
