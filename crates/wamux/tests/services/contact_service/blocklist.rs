//! #197: GetBlocklist relays every entry the server lists, its `t` as unix
//! seconds, unset when the server sent none.

use wacore_binary::builder::NodeBuilder;
use wamux::proto::v1 as pb;

use crate::common::mock_wire::sent_iq;
use crate::contact_service::harness::{BLOCKLIST, CONTACT_LID, CONTACT_PN, fixture, jid};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_blocklist_relays_every_entry() {
    let mut f = fixture("get_blocklist_relays_every_entry").await;
    let list = NodeBuilder::new("list")
        .children([
            NodeBuilder::new("item")
                .attr("jid", CONTACT_LID)
                .attr("t", "1760000000")
                .build(),
            NodeBuilder::new("item").attr("jid", CONTACT_PN).build(),
        ])
        .build();
    f.mock.answer_iq(BLOCKLIST, "get", "", list);
    let entries = f
        .contacts
        .get_blocklist(f.account.clone())
        .await
        .expect("blocklist")
        .into_inner()
        .entries;
    assert_eq!(
        entries,
        vec![
            pb::BlocklistEntry {
                jid: jid(CONTACT_LID),
                timestamp: Some(1_760_000_000),
            },
            pb::BlocklistEntry {
                jid: jid(CONTACT_PN),
                timestamp: None,
            },
        ]
    );
    sent_iq(&f.mock, BLOCKLIST, "get", "").await;
    f.cleanup().await;
}
