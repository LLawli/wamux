//! `WamuxError` -> `tonic::Status` (#114): the code and the message of each
//! variant, as the daemon answered them before the move.

use tonic::Code;

use crate::WamuxError;

fn status(err: WamuxError) -> tonic::Status {
    tonic::Status::from(err)
}

fn assert_status(err: WamuxError, code: Code, message: &str) {
    let status = status(err);
    assert_eq!(status.code(), code);
    assert_eq!(status.message(), message);
}

#[test]
fn client_facing_errors_keep_their_code_and_message() {
    assert_status(
        WamuxError::AccountNotFound("pessoal".into()),
        Code::NotFound,
        "account pessoal not found",
    );
    assert_status(
        WamuxError::NotConnected,
        Code::FailedPrecondition,
        "account is not connected",
    );
    assert_status(
        WamuxError::InvalidArgument("missing jid".into()),
        Code::InvalidArgument,
        "missing jid",
    );
    assert_status(
        WamuxError::ResourceExhausted("media exceeds size limit".into()),
        Code::ResourceExhausted,
        "media exceeds size limit",
    );
}

#[test]
fn internal_errors_hide_their_cause() {
    let store = wacore::store::error::StoreError::Validation("secret detail".into());
    assert_status(WamuxError::Store(store), Code::Internal, "internal error");
    assert_status(
        WamuxError::Other(anyhow::anyhow!("secret detail")),
        Code::Internal,
        "internal error",
    );
}

#[test]
fn a_client_error_is_unavailable_and_generic() {
    assert_status(
        WamuxError::Client("socket closed: secret detail".into()),
        Code::Unavailable,
        "whatsapp operation failed",
    );
}

#[test]
fn a_server_rejection_maps_its_code_and_relays_it() {
    let table = [
        (400, Code::InvalidArgument),
        (401, Code::Unauthenticated),
        (403, Code::PermissionDenied),
        (404, Code::NotFound),
        (405, Code::PermissionDenied),
        (429, Code::ResourceExhausted),
        (500, Code::Unavailable),
        (409, Code::Unavailable),
    ];
    for (code, grpc) in table {
        let status = status(WamuxError::WaServer {
            code,
            text: "not-allowed".into(),
        });
        assert_eq!(status.code(), grpc, "wa code {code}");
        assert_eq!(
            status.message(),
            format!("whatsapp server rejected the request: code={code}, text='not-allowed'")
        );
        assert_eq!(
            status.metadata().get("wa-code").unwrap(),
            code.to_string().as_str()
        );
        assert_eq!(status.metadata().get("wa-text").unwrap(), "not-allowed");
    }
}

/// tonic carries ASCII metadata only: a non-ASCII text is left out of the
/// trailer, and the prose still has it.
#[test]
fn a_non_ascii_server_text_skips_the_trailer_but_keeps_the_code() {
    let status = status(WamuxError::WaServer {
        code: 403,
        text: "não".into(),
    });
    assert_eq!(status.metadata().get("wa-code").unwrap(), "403");
    assert!(status.metadata().get("wa-text").is_none());
    assert!(status.message().contains("não"));
}
