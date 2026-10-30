//! GetProfilePicture refusals (#205).

use tonic::Code;
use wacore_binary::builder::NodeBuilder;

use crate::contact_service::harness::{CONTACT_PN, fixture};

// A rate limit the server puts inside `<picture>` is the edge's to back off
// from, not "this contact has no picture": the contact may well have one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_profile_picture_rate_limited_is_resource_exhausted() {
    let mut f = fixture("get_profile_picture_rate_limited_is_resource_exhausted").await;
    let refusal = NodeBuilder::new("picture")
        .children([NodeBuilder::new("error")
            .attr("code", "429")
            .attr("text", "rate-overlimit")
            .attr("type", "wait")
            .build()])
        .build();
    f.mock
        .answer_iq("w:profile:picture", "get", "picture", refusal);
    let request = f.jid_request(CONTACT_PN);
    let status = f
        .contacts
        .get_profile_picture(request)
        .await
        .expect_err("a rate limit is an error, not an empty picture");
    assert_eq!(status.code(), Code::ResourceExhausted, "{status:?}");
    f.cleanup().await;
}
