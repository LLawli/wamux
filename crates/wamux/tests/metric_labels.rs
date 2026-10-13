//! #80: the request metrics are labelled by method only for the methods the
//! daemon serves. A local client calling made-up paths used to create one
//! Prometheus series per path, never released, and `GetMetrics` grew with
//! them. Over the real socket, read back through `AdminService.GetMetrics`.

use std::collections::BTreeSet;
use std::time::Duration;

use http::uri::PathAndQuery;
use tonic::Code;
use tonic::transport::Channel;
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::admin_service_client::AdminServiceClient;

#[allow(dead_code)]
mod common;

const BOGUS_CALLS: u64 = 1000;
const FAMILIES: [&str; 2] = [
    "wamux_grpc_requests_total",
    "wamux_grpc_request_duration_seconds",
];

async fn metrics(channel: &Channel) -> String {
    AdminServiceClient::new(channel.clone())
        .get_metrics(pb::Empty {})
        .await
        .expect("GetMetrics")
        .into_inner()
        .prometheus
}

/// The value of `name="..."` in a sample line, if it has one.
fn label_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let start = line.find(&format!("{name}=\""))? + name.len() + 2;
    let end = line[start..].find('"')?;
    Some(&line[start..start + end])
}

/// Every `method` label in the two request metric families.
fn method_labels(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|line| FAMILIES.iter().any(|family| line.starts_with(family)))
        .filter_map(|line| label_value(line, "method"))
        .map(str::to_string)
        .collect()
}

/// The request counter's samples matching `keep`, summed.
fn requests_where(text: &str, keep: impl Fn(&str) -> bool) -> u64 {
    text.lines()
        .filter(|line| line.starts_with("wamux_grpc_requests_total{"))
        .filter(|line| keep(line))
        .filter_map(|line| line.rsplit(' ').next()?.parse::<u64>().ok())
        .sum()
}

fn unimplemented_requests(text: &str) -> u64 {
    requests_where(text, |line| {
        label_value(line, "status") == Some("unimplemented")
    })
}

fn unknown_unimplemented(text: &str) -> u64 {
    requests_where(text, |line| {
        label_value(line, "status") == Some("unimplemented")
            && label_value(line, "method") == Some("unknown")
    })
}

/// Call `path`, which the daemon does not serve, with an empty message.
async fn call_made_up_path(channel: &Channel, path: &str) -> tonic::Status {
    let mut grpc = tonic::client::Grpc::new(channel.clone());
    grpc.ready().await.expect("channel ready");
    let path = PathAndQuery::try_from(path).expect("a valid path");
    grpc.unary::<pb::Empty, pb::Empty, _>(
        tonic::Request::new(pb::Empty {}),
        path,
        tonic_prost::ProstCodec::default(),
    )
    .await
    .expect_err("a made-up path is not served")
}

/// Half made-up services, half made-up methods of a real service.
fn bogus_path(index: u64) -> String {
    if index.is_multiple_of(2) {
        format!("/wamux.bogus{index}.Service/Method{index}")
    } else {
        format!("/wamux.v1.AccountService/NoSuchMethod{index}")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_thousand_unknown_paths_add_one_unknown_series() {
    let (channel, _engine) = common::spawn_server().await;
    let before_text = metrics(&channel).await;
    let before = method_labels(&before_text);
    let unimplemented_before = unimplemented_requests(&before_text);
    let unknown_before = unknown_unimplemented(&before_text);

    for index in 0..BOGUS_CALLS {
        let path = bogus_path(index);
        let status = call_made_up_path(&channel, &path).await;
        assert_eq!(status.code(), Code::Unimplemented, "{path}: {status:?}");
    }
    // The sample is recorded when the response body drops, a beat after the
    // client has its answer: wait until all of them are counted, under any label.
    let after_text = common::poll_until(
        "the made-up calls to be counted",
        Duration::from_secs(10),
        || async {
            let text = metrics(&channel).await;
            (unimplemented_requests(&text) >= unimplemented_before + BOGUS_CALLS).then_some(text)
        },
    )
    .await;

    let after = method_labels(&after_text);
    let bogus: Vec<&String> = after
        .iter()
        .filter(|label| label.contains("bogus") || label.contains("NoSuchMethod"))
        .collect();
    assert!(
        bogus.is_empty(),
        "made-up paths became labels: {} of them, e.g. {:?}",
        bogus.len(),
        bogus.first()
    );
    // The only labels allowed to appear: `unknown`, and the real methods this
    // binary's tests call (GetMetrics here, ListAccounts in the other test,
    // which shares the process-wide recorder).
    let allowed = [
        "unknown",
        "wamux.v1.AdminService/GetMetrics",
        "wamux.v1.AccountService/ListAccounts",
    ];
    let added: BTreeSet<&String> = after.difference(&before).collect();
    assert!(
        added.iter().all(|label| allowed.contains(&label.as_str())),
        "new labels: {added:?}"
    );
    assert!(after.contains("unknown"), "{after:?}");
    assert_eq!(
        unknown_unimplemented(&after_text),
        unknown_before + BOGUS_CALLS,
        "every made-up call counted once, under unknown"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_known_method_keeps_its_own_label() {
    let (channel, _engine) = common::spawn_server().await;
    AccountServiceClient::new(channel.clone())
        .list_accounts(pb::ListAccountsRequest {})
        .await
        .expect("ListAccounts");
    common::poll_until(
        "ListAccounts to be labelled as itself",
        Duration::from_secs(10),
        || async {
            let labels = method_labels(&metrics(&channel).await);
            labels
                .contains("wamux.v1.AccountService/ListAccounts")
                .then_some(())
        },
    )
    .await;
}
