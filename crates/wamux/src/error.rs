//! The daemon's side of the error type. `WamuxError` and its mapping to
//! `tonic::Status` live in `wamux-types` (#63, #114); what stays here is
//! building one from a whatsapp-rust error, which needs the library's types.
//! Storage trait impls must return `wacore::store::error::StoreError`, so that
//! mapping lives in `storage::sqlx_error`.

pub use wamux_types::WamuxError;

/// Wrap an error from a whatsapp-rust client call. Walks the cause chain for a
/// server IQ rejection and lifts its code into `WaServer`; anything else keeps
/// the full anyhow cause chain (`{:#}`) as an opaque `Client` error.
pub(crate) fn client_err(err: impl Into<anyhow::Error>) -> WamuxError {
    let err: anyhow::Error = err.into();
    if let Some((code, text)) = err.chain().find_map(iq_server_rejection) {
        return WamuxError::WaServer { code, text };
    }
    match err.chain().find_map(invalid_request_message) {
        Some(reason) => WamuxError::InvalidArgument(reason),
        None => WamuxError::Client(format!("{err:#}")),
    }
}

/// #101: the lib refuses a request it can never serve with typed errors that
/// carry no server code (`PollError::InvalidPoll`, `AppStateError::
/// InvalidRequest`). Read as opaque they became Unavailable and the edge
/// retried a call that could never succeed. The lib's own Display is the
/// reason; `NotConnected` and `NotLoggedIn` are transport, not the request.
fn invalid_request_message(cause: &(dyn std::error::Error + 'static)) -> Option<String> {
    use whatsapp_rust::features::{AppStateError, PollError};
    if let Some(err @ PollError::InvalidPoll(_)) = cause.downcast_ref::<PollError>() {
        return Some(err.to_string());
    }
    if let Some(err @ AppStateError::InvalidRequest(_)) = cause.downcast_ref::<AppStateError>() {
        return Some(err.to_string());
    }
    None
}

/// The lib surfaces server rejections as five types depending on the path:
/// `ServerErrorCode` (its own cross-crate wrapper), the high-level `IqError`,
/// wacore's `IqError`, a MEX query's `ExtensionError`, or `GroupError::
/// DescriptionConflict`. Probe all five.
fn iq_server_rejection(cause: &(dyn std::error::Error + 'static)) -> Option<(u16, String)> {
    use wacore::request::{IqError as WacoreIq, ServerErrorCode};
    use whatsapp_rust::features::GroupError;
    use whatsapp_rust::request::IqError as ClientIq;
    if let Some(e) = cause.downcast_ref::<ServerErrorCode>() {
        return Some((e.code, e.text.clone()));
    }
    // `..`: 0.7 added `error_type` (the XMPP error class) and `backoff` (a
    // server-directed retry delay). Ignored on purpose. Retry timing is edge
    // policy, so the core has no use for them, and surfacing them is a proto
    // change to make deliberately, not one smuggled into the error mapper.
    if let Some(ClientIq::ServerError { code, text, .. }) = cause.downcast_ref::<ClientIq>() {
        return Some((*code, text.clone()));
    }
    if let Some(WacoreIq::ServerError { code, text, .. }) = cause.downcast_ref::<WacoreIq>() {
        return Some((*code, text.clone()));
    }
    // #96: the lib rewrites a 409 on the description update into this unit
    // variant, which loses the code. Put it back so the edge sees a 409.
    if let Some(err @ GroupError::DescriptionConflict) = cause.downcast_ref::<GroupError>() {
        return Some((409, err.to_string()));
    }
    mex_server_rejection(cause)
}

/// A MEX query the server refused arrives as a GraphQL error with a code, not
/// as an IQ error stanza. #56: a channel this account does not follow answered
/// `code=405, message='Not Allowed'` in production, and the edge saw
/// Unavailable ("the core is down"). A code outside `u16` is not an HTTP-style
/// status and stays an opaque client error.
fn mex_server_rejection(cause: &(dyn std::error::Error + 'static)) -> Option<(u16, String)> {
    use whatsapp_rust::features::MexError;
    let Some(MexError::ExtensionError { code, message }) = cause.downcast_ref::<MexError>() else {
        return None;
    };
    Some((u16::try_from(*code).ok()?, message.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::request::IqError;

    fn status_for(err: anyhow::Error) -> tonic::Status {
        tonic::Status::from(client_err(err))
    }

    fn iq(code: u16, text: &str) -> anyhow::Error {
        anyhow::Error::new(IqError::ServerError {
            code,
            text: text.into(),
            error_type: None,
            backoff: None,
            response: rejection_stanza(code),
        })
    }

    /// main (#30) hands the `type="error"` stanza over whole next to the
    /// summary. Built through marshal + unpack, the path the receive side takes.
    fn rejection_stanza(code: u16) -> whatsapp_rust::request::RejectionStanza {
        use whatsapp_rust::wacore_binary::{OwnedNodeRef, marshal::marshal, util::unpack};
        let node = whatsapp_rust::NodeBuilder::new("iq")
            .attr("type", "error")
            .children([whatsapp_rust::NodeBuilder::new("error")
                .attr("code", code.to_string())
                .build()])
            .build();
        // unwrap: marshalling and re-reading a node built one line above.
        let packed = marshal(&node).unwrap();
        let bytes = unpack(&packed).unwrap().into_owned();
        std::sync::Arc::new(OwnedNodeRef::new(bytes).unwrap()).into()
    }

    // Regression for edge-review-insights.md achado #3: WhatsApp auth errors
    // must not surface as Unavailable ("core down" / 503 at the edge).
    #[test]
    fn iq_401_maps_to_unauthenticated() {
        let status = status_for(iq(401, "not-authorized"));
        assert_eq!(status.code(), tonic::Code::Unauthenticated);
        assert!(status.message().contains("code=401"));
    }

    #[test]
    fn iq_403_maps_to_permission_denied_even_under_context() {
        let status = status_for(iq(403, "forbidden").context("query group invite link"));
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert!(status.message().contains("forbidden"));
    }

    #[test]
    fn server_error_code_wrapper_is_detected() {
        let err = anyhow::Error::new(wacore::request::ServerErrorCode {
            code: 404,
            text: "item-not-found".into(),
            error_type: None,
            backoff: None,
        });
        assert_eq!(status_for(err).code(), tonic::Code::NotFound);
    }

    #[test]
    fn unmapped_iq_code_stays_unavailable() {
        assert_eq!(
            status_for(iq(500, "internal-server-error")).code(),
            tonic::Code::Unavailable
        );
    }

    // The structured contract: the edge reads wa-code/wa-text trailers, never
    // regexes the prose message (whose wording is free to change).
    #[test]
    fn iq_rejection_carries_wa_code_and_wa_text_metadata() {
        let status = status_for(iq(403, "forbidden"));
        assert_eq!(status.metadata().get("wa-code").unwrap(), "403");
        assert_eq!(status.metadata().get("wa-text").unwrap(), "forbidden");
    }

    // Even codes that collapse to Unavailable stay distinguishable by trailer.
    #[test]
    fn unmapped_iq_code_still_carries_wa_code_metadata() {
        let status = status_for(iq(409, "conflict"));
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert_eq!(status.metadata().get("wa-code").unwrap(), "409");
    }

    fn mex(code: i32, message: &str) -> anyhow::Error {
        anyhow::Error::new(whatsapp_rust::features::MexError::ExtensionError {
            code,
            message: message.into(),
        })
    }

    // #56: measured in production on a channel the account does not follow.
    #[test]
    fn mex_405_maps_to_permission_denied_with_wa_metadata() {
        let status = status_for(mex(405, "Not Allowed"));
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert_eq!(status.metadata().get("wa-code").unwrap(), "405");
        assert_eq!(status.metadata().get("wa-text").unwrap(), "Not Allowed");
    }

    // The library wraps the MEX error in its feature error; the chain walk
    // still finds it.
    #[test]
    fn mex_rejection_is_found_under_a_newsletter_error() {
        let wrapped = whatsapp_rust::features::NewsletterError::Mex(
            whatsapp_rust::features::MexError::ExtensionError {
                code: 404,
                message: "Not Found".into(),
            },
        );
        assert_eq!(
            status_for(anyhow::Error::new(wrapped)).code(),
            tonic::Code::NotFound
        );
    }

    #[test]
    fn iq_405_maps_to_permission_denied() {
        assert_eq!(
            status_for(iq(405, "not-allowed")).code(),
            tonic::Code::PermissionDenied
        );
    }

    #[test]
    fn a_mex_code_outside_u16_stays_an_opaque_client_error() {
        let status = status_for(mex(-1, "weird"));
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(status.metadata().get("wa-code").is_none());
    }

    #[test]
    fn non_iq_client_error_has_no_wa_metadata() {
        let status = status_for(anyhow::anyhow!("websocket torn down"));
        assert!(status.metadata().get("wa-code").is_none());
    }

    #[test]
    fn non_iq_client_error_stays_unavailable_and_generic() {
        let status = status_for(anyhow::anyhow!("websocket torn down"));
        assert_eq!(status.code(), tonic::Code::Unavailable);
        // Opaque failures keep the generic non-leaking message.
        assert_eq!(status.message(), "whatsapp operation failed");
    }
}

#[cfg(test)]
#[path = "error_group_tests.rs"]
mod group_tests;

#[cfg(test)]
#[path = "error_invalid_request_tests.rs"]
mod invalid_request_tests;
