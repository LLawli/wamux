//! #116: a channel that does not exist is NotFound with a message naming the
//! channel. Before, `newsletter_err` built an `AccountNotFound`, so the wire
//! read "account no newsletter metadata for <jid> not found".

use tonic::Code;

use crate::captured::{MISSING, MISSING_JSON};
use crate::harness::fixture;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_channel_names_the_channel() {
    let mut f = fixture("a_missing_channel_names_the_channel").await;
    f.mock.answer_mex_with(MISSING_JSON);
    let status = f
        .channels
        .get_newsletter_metadata(f.jid_request(MISSING))
        .await
        .expect_err("no channel behind the jid");
    assert_eq!(status.code(), Code::NotFound, "{status:?}");
    assert_eq!(status.message(), format!("newsletter {MISSING} not found"));
    f.cleanup().await;
}
