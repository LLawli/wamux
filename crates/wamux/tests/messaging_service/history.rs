//! FetchMessageHistory asks the account's own phone for older messages: a
//! peer message (`category="peer"`) to device 0, carrying a
//! HISTORY_SYNC_ON_DEMAND request. The phone answers later with a history sync
//! the edge correlates by the session id the RPC returned.

use wamux::proto::v1 as pb;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::common::mock_wire::{attr, sent_message};
use crate::harness::{fixture, jid, sender};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_message_history_asks_the_phone_on_demand() {
    let mut f = fixture("fetch_message_history_asks_the_phone_on_demand").await;
    let chat = f.peer.pn().to_string();
    let request = pb::FetchMessageHistoryRequest {
        account: f.a(),
        chat: jid(&chat),
        oldest_msg_id: "3EB0OLDEST".into(),
        oldest_msg_from_me: true,
        oldest_msg_timestamp_ms: 1_790_000_000_000,
        count: 50,
    };
    let answer = f
        .messages
        .fetch_message_history(request)
        .await
        .expect("fetch history")
        .into_inner();
    // The session id is the request stanza's own id (whatsapp-rust pdo.rs).
    let stanza = sent_message(&f.mock, &answer.session_id).await;
    assert_eq!(attr(&stanza, "category").as_deref(), Some("peer"));
    let opened = f
        .phone
        .open_dm(&stanza, &sender())
        .await
        .expect("phone opens");
    let protocol = opened
        .protocol_message
        .as_option()
        .expect("protocolMessage");
    use wa::message::protocol_message::Type;
    assert_eq!(
        protocol.r#type,
        Some(Type::PEER_DATA_OPERATION_REQUEST_MESSAGE)
    );
    let pdo = protocol
        .peer_data_operation_request_message
        .as_option()
        .expect("peer data operation");
    assert_eq!(
        pdo.peer_data_operation_request_type,
        Some(wa::message::PeerDataOperationRequestType::HISTORY_SYNC_ON_DEMAND)
    );
    let on_demand = pdo
        .history_sync_on_demand_request
        .as_option()
        .expect("on-demand request");
    assert_eq!(on_demand.chat_jid.as_deref(), Some(chat.as_str()));
    assert_eq!(on_demand.oldest_msg_id.as_deref(), Some("3EB0OLDEST"));
    assert_eq!(on_demand.oldest_msg_from_me, Some(true));
    assert_eq!(on_demand.oldest_msg_timestamp_ms, Some(1_790_000_000_000));
    assert_eq!(on_demand.on_demand_msg_count, Some(50));
    f.cleanup().await;
}
