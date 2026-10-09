//! #101: the library refuses a request it can never serve with a typed error
//! that has no server code. `client_err` read it as an opaque client failure
//! and the edge saw Unavailable ("the core is down"), so it retried a request
//! that could never succeed. The caller's mistake is InvalidArgument.

use whatsapp_rust::features::{AppStateError, PollError};

use super::client_err;

fn status_for(err: impl Into<anyhow::Error>) -> tonic::Status {
    tonic::Status::from(client_err(err))
}

#[test]
fn an_invalid_poll_is_invalid_argument_with_the_library_reason() {
    let status = status_for(PollError::InvalidPoll(
        "poll must have at least 2 options".into(),
    ));
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(
        status.message().contains("at least 2 options"),
        "{status:?}"
    );
}

#[test]
fn an_invalid_app_state_request_is_invalid_argument_even_under_context() {
    let err = anyhow::Error::new(AppStateError::InvalidRequest(
        "mute_end_timestamp_ms is in the past (1000 <= 2000)".into(),
    ))
    .context("mute chat");
    let status = status_for(err);
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("in the past"), "{status:?}");
}

// The two refusals above are about the request. A lost connection is not, and
// must keep telling the edge the core could not serve it right now.
#[test]
fn a_transport_failure_stays_unavailable() {
    let status = status_for(AppStateError::NotConnected);
    assert_eq!(status.code(), tonic::Code::Unavailable);
    let poll = status_for(PollError::NotLoggedIn);
    assert_eq!(poll.code(), tonic::Code::Unavailable);
}
