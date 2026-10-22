//! The four events whatsapp-rust eab45737 added reach a subscriber as typed
//! envelopes through the socket, not as a `RawEvent` (#215).
//!
//! Each test publishes the library's event on the account's own publisher (the
//! path the library's handler takes) and reads it back with `SubscribeEvents`
//! over a real socket, replaying the ring so the read does not race the
//! forwarder attaching. The mapping details are pinned by the unit tests next
//! to each mapper; these prove the arm reaches the wire.
//!
//! Runs against whichever engine WAMUX_TEST_ENGINE names (default Postgres,
//! which needs DATABASE_URL and the docker container).

#![allow(clippy::field_reassign_with_default)] // the library's records are built by assignment

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use tonic::transport::Channel;
use wacore::types::events::{
    CallLogHistory, Event, LabelSublistChange, LabelSublistUpdate, ReachoutTimelock,
    ReachoutTimelockUpdate, UnarchiveChatsSettingUpdate,
};
use wamux::proto::v1 as pb;
use wamux::proto::v1::event_envelope::Event as PbEvent;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::state::{AccountRegistry, RegistryTuning};
use whatsapp_rust::Jid;
use whatsapp_rust::waproto::whatsapp::CallLogRecord;
use whatsapp_rust::waproto::whatsapp::sync_action_value::UnarchiveChatsSetting;

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

const CHAT: &str = "100000000000215@lid";
const CALL_ID: &str = "009F90B557B94899EC85D0D3E67B6215";

/// Publish `event` on a fresh account, then subscribe over the socket with a
/// replay and return the one envelope that arrives.
async fn envelope_for(test: &str, event: Event) -> pb::EventEnvelope {
    let engine = common::test_engine().await;
    let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(8)));
    let prefix = common::test_prefix("typed_events", test);
    let _ = common::sweep_orphans(registry.storage(), &prefix).await;
    let handle = registry
        .create_account(Some(&format!("{prefix}{}", uuid::Uuid::new_v4())))
        .await
        .expect("create account");
    let channel: Channel = common::serve_registry(registry.clone()).await;

    handle.publisher.publish_lib(Arc::new(event));
    let request = pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(pb::AccountRef {
            r#ref: Some(pb::account_ref::Ref::Uuid(handle.uuid.to_string())),
        })),
        replay_from_ring: 8,
    };
    let mut stream = EventServiceClient::new(channel)
        .subscribe_events(request)
        .await
        .expect("subscribe")
        .into_inner();
    let envelope = tokio::time::timeout(Duration::from_secs(5), stream.message())
        .await
        .expect("an envelope in time")
        .expect("a healthy stream")
        .expect("an open stream");
    registry.delete(&handle).await.expect("delete account");
    envelope
}

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unarchive_chats_setting_reaches_the_socket_as_an_account_setting() {
    let mut action = UnarchiveChatsSetting::default();
    action.unarchive_chats = Some(false);
    let event = Event::UnarchiveChatsSettingUpdate(
        UnarchiveChatsSettingUpdate::builder()
            .unarchive_chats(false)
            .timestamp(wacore::time::from_millis(1_792_000_000_123).expect("instant"))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    );
    let envelope = envelope_for("unarchive_chats_setting", event).await;
    let Some(PbEvent::AccountSetting(update)) = envelope.event else {
        panic!("expected an AccountSettingUpdate, got {:?}", envelope.event);
    };
    assert_eq!(update.timestamp, 1_792_000_000_123);
    assert_eq!(
        update.setting,
        Some(pb::account_setting_update::Setting::UnarchiveChats(
            pb::UnarchiveChatsSetting {
                unarchive_chats: false
            }
        ))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn label_sublist_update_reaches_the_socket_as_its_own_arm() {
    let event = Event::LabelSublistUpdate(
        LabelSublistUpdate::builder()
            .predefined_id(5)
            .chat_jid(Jid::from_str(CHAT).expect("test jid"))
            .change(LabelSublistChange::Upsert { sub_list_id: 2 })
            .from_full_sync(false)
            .build(),
    );
    let envelope = envelope_for("label_sublist_update", event).await;
    let Some(PbEvent::LabelSublist(update)) = envelope.event else {
        panic!("expected a LabelSublistUpdate, got {:?}", envelope.event);
    };
    assert_eq!(update.predefined_id, 5);
    assert_eq!(update.chat, wire(CHAT));
    assert_eq!(
        update.change,
        Some(pb::label_sublist_update::Change::UpsertSubListId(2))
    );
    assert_eq!(update.action_timestamp, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn call_log_history_reaches_the_socket_as_its_own_arm() {
    let mut record = CallLogRecord::default();
    record.call_id = Some(CALL_ID.to_string());
    record.start_time = Some(1_791_489_445);
    let event = Event::CallLogHistory(
        CallLogHistory::builder()
            .from_me(false)
            .record(Box::new(record))
            .chunk_order(1)
            .build(),
    );
    let envelope = envelope_for("call_log_history", event).await;
    let Some(PbEvent::CallLogHistory(history)) = envelope.event else {
        panic!("expected a CallLogHistory, got {:?}", envelope.event);
    };
    assert_eq!(history.call_id.as_deref(), Some(CALL_ID));
    assert_eq!(history.from_me, Some(false));
    assert_eq!(history.start_time, Some(1_791_489_445_000));
    assert_eq!(history.chunk_order, Some(1));
    assert_eq!(history.sync_type, None);
    assert_eq!(history.call_creator, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reachout_timelock_update_reaches_the_socket_as_its_own_arm() {
    let mut state = ReachoutTimelock::default();
    state.is_active = Some(false);
    let event = Event::ReachoutTimelockUpdate(
        ReachoutTimelockUpdate::builder()
            .state(state)
            .stanza_id("3EB0TIMELOCK".to_string())
            .build(),
    );
    let envelope = envelope_for("reachout_timelock_update", event).await;
    let Some(PbEvent::ReachoutTimelock(update)) = envelope.event else {
        panic!(
            "expected a ReachoutTimelockUpdate, got {:?}",
            envelope.event
        );
    };
    assert_eq!(update.is_active, Some(false));
    assert_eq!(update.stanza_id.as_deref(), Some("3EB0TIMELOCK"));
    assert_eq!(update.enforcement_type, None);
    assert_eq!(update.from, None);
}
