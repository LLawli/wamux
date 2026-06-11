//! Crate-level typed errors and their mapping to `tonic::Status`.
//!
//! Domain/storage code returns `WamuxError`; the service edge converts it to a
//! clean `Status` (the client never sees an internal error string). Storage
//! trait impls must return `wacore::store::error::StoreError`, so that mapping
//! lives in `storage::postgres::error_map`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WamuxError {
    #[error("account not found: {0}")]
    AccountNotFound(String),

    #[error("account is not connected")]
    NotConnected,

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("resource exhausted: {0}")]
    ResourceExhausted(String),

    #[error("storage error")]
    Store(#[from] wacore::store::error::StoreError),

    #[error("database error")]
    Database(#[from] sqlx::Error),

    #[error("whatsapp client error: {0}")]
    Client(String),

    /// Upstream WhatsApp server refused the request with an IQ error stanza
    /// (e.g. `<error code="403"/>`). Code + text relay verbatim so the boundary
    /// can map auth-shaped codes honestly instead of a blanket Unavailable
    /// (edge-review-insights.md achado #3).
    #[error("whatsapp server rejected the request: code={code}, text='{text}'")]
    WaServer { code: u16, text: String },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Wrap an error from a whatsapp-rust client call. Walks the cause chain for a
/// server IQ rejection and lifts its code into `WaServer`; anything else keeps
/// the full anyhow cause chain (`{:#}`) as an opaque `Client` error.
pub(crate) fn client_err(err: impl Into<anyhow::Error>) -> WamuxError {
    let err: anyhow::Error = err.into();
    match err.chain().find_map(iq_server_rejection) {
        Some((code, text)) => WamuxError::WaServer { code, text },
        None => WamuxError::Client(format!("{err:#}")),
    }
}

/// The lib surfaces server rejections as three types depending on the path:
/// `ServerErrorCode` (its own cross-crate wrapper), the high-level `IqError`,
/// or wacore's `IqError`. Probe all three.
fn iq_server_rejection(cause: &(dyn std::error::Error + 'static)) -> Option<(u16, String)> {
    use wacore::request::{IqError as WacoreIq, ServerErrorCode};
    use whatsapp_rust::request::IqError as ClientIq;
    if let Some(e) = cause.downcast_ref::<ServerErrorCode>() {
        return Some((e.code, e.text.clone()));
    }
    if let Some(ClientIq::ServerError { code, text }) = cause.downcast_ref::<ClientIq>() {
        return Some((*code, text.clone()));
    }
    if let Some(WacoreIq::ServerError { code, text }) = cause.downcast_ref::<WacoreIq>() {
        return Some((*code, text.clone()));
    }
    None
}

/// Full `Display` cause chain ("outer: middle: root"), for integral logging.
fn cause_chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

/// Convert at the service boundary: log the integral internal cause, then hand
/// the client a clean, non-leaking `Status`.
///
/// - Client-facing/expected errors (not_found, failed_precondition,
///   invalid_argument) carry a safe message and log at debug.
/// - Internal and upstream errors log the full chain at error/warn and the
///   client only sees a generic message + code.
/// - `WaServer` is the exception: the WhatsApp server's own code/text relay
///   verbatim (upstream protocol info, not internal state) so the edge can
///   compose policy on it.
impl From<WamuxError> for tonic::Status {
    fn from(err: WamuxError) -> Self {
        use tonic::Status;
        match &err {
            WamuxError::AccountNotFound(id) => {
                tracing::debug!(account = %id, "account not found");
                Status::not_found(format!("account {id} not found"))
            }
            WamuxError::NotConnected => {
                tracing::debug!("account is not connected");
                Status::failed_precondition("account is not connected")
            }
            WamuxError::InvalidArgument(message) => {
                tracing::debug!(reason = %message, "invalid argument");
                Status::invalid_argument(message.clone())
            }
            WamuxError::ResourceExhausted(message) => {
                tracing::warn!(reason = %message, "resource exhausted");
                Status::resource_exhausted(message.clone())
            }
            WamuxError::Store(_) | WamuxError::Database(_) | WamuxError::Other(_) => {
                tracing::error!(cause = %cause_chain(&err), "internal error at service boundary");
                Status::internal("internal error")
            }
            WamuxError::Client(_) => {
                tracing::warn!(cause = %cause_chain(&err), "upstream whatsapp error");
                Status::unavailable("whatsapp operation failed")
            }
            WamuxError::WaServer { code, text } => {
                tracing::warn!(code, text = %text, "whatsapp server rejected the request");
                wa_server_status(*code, text, err.to_string())
            }
        }
    }
}

/// Honest relay of an upstream IQ rejection: the request DID reach WhatsApp
/// and was refused, so auth-shaped codes must not read as "core down" (the
/// edge turned them into 503). Unmapped codes keep the legacy Unavailable.
///
/// The raw upstream code/text also ride as `wa-code`/`wa-text` trailers
/// (code-review 2026-06-11): the edge composes policy on the structured
/// primitive; the prose message is for humans and free to change.
fn wa_server_status(code: u16, text: &str, message: String) -> tonic::Status {
    use tonic::metadata::MetadataValue;
    use tonic::{Code, Status};
    let grpc_code = match code {
        400 => Code::InvalidArgument,
        401 => Code::Unauthenticated,
        403 => Code::PermissionDenied,
        404 => Code::NotFound,
        429 => Code::ResourceExhausted,
        _ => Code::Unavailable,
    };
    let mut status = Status::new(grpc_code, message);
    // u16 digits are always valid ASCII metadata, so this never skips.
    if let Ok(value) = MetadataValue::try_from(code.to_string()) {
        status.metadata_mut().insert("wa-code", value);
    }
    // IQ text is normally an ASCII token ("not-authorized"); anything tonic
    // can't carry as ASCII is silently omitted — the prose still has it.
    if let Ok(value) = MetadataValue::try_from(text) {
        status.metadata_mut().insert("wa-text", value);
    }
    status
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
        })
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
