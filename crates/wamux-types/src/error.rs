//! `WamuxError` and its mapping to `tonic::Status` (#63, #114), moved here
//! from the daemon so every layer shares one error type.
//!
//! This file is the ONE place a `Status` is constructed
//! (`scripts/check-status-sites.py`): domain and services return
//! `WamuxError`, and `?` converts at the service boundary. The client never
//! sees an internal error string.
//!
//! Building a `WaServer` or `Client` from a whatsapp-rust error stays in the
//! daemon (`wamux::error::client_err`): it downcasts the library's types, and
//! this crate does not depend on whatsapp-rust.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WamuxError {
    #[error("account not found: {0}")]
    AccountNotFound(String),

    /// Something other than an account is missing, a channel for one (#116).
    /// The message says what and relays as written: `AccountNotFound` would
    /// wrap it in "account ... not found", which named the wrong thing.
    #[error("not found: {0}")]
    NotFound(String),

    #[error("account is not connected")]
    NotConnected,

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("resource exhausted: {0}")]
    ResourceExhausted(String),

    #[error("storage error")]
    Store(#[from] wacore::store::error::StoreError),

    #[error("whatsapp client error: {0}")]
    Client(String),

    /// Upstream WhatsApp server refused the request with an IQ error stanza
    /// (e.g. `<error code="403"/>`) or a fatal MEX (GraphQL) error carrying a
    /// code (#56). Code + text relay verbatim so the boundary can map
    /// auth-shaped codes honestly instead of a blanket Unavailable
    /// (edge-review-insights.md achado #3).
    #[error("whatsapp server rejected the request: code={code}, text='{text}'")]
    WaServer { code: u16, text: String },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
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
///   compose policy on it, also as `wa-code` / `wa-text` trailers.
impl From<WamuxError> for tonic::Status {
    fn from(err: WamuxError) -> Self {
        use tonic::Status;
        match &err {
            WamuxError::AccountNotFound(id) => {
                tracing::debug!(account = %id, "account not found");
                Status::not_found(format!("account {id} not found"))
            }
            WamuxError::NotFound(message) => {
                tracing::debug!(%message, "not found");
                Status::not_found(message.clone())
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
            WamuxError::Store(_) | WamuxError::Other(_) => {
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
        // "not-allowed" (IQ) / "Not Allowed" (MEX): the server refuses this
        // account the operation, e.g. reading a channel it does not follow.
        405 => Code::PermissionDenied,
        429 => Code::ResourceExhausted,
        _ => Code::Unavailable,
    };
    let mut status = Status::new(grpc_code, message);
    // u16 digits are always valid ASCII metadata, so this never skips.
    if let Ok(value) = MetadataValue::try_from(code.to_string()) {
        status.metadata_mut().insert("wa-code", value);
    }
    // IQ text is normally an ASCII token ("not-authorized"). `MetadataValue`
    // alone would also accept bytes above 0x7f (HTTP header obs-text) and ship
    // them in an ASCII trailer, so non-ASCII is checked explicitly and left
    // out: the prose still has it (#114).
    if let Some(value) = text
        .is_ascii()
        .then(|| MetadataValue::try_from(text).ok())
        .flatten()
    {
        status.metadata_mut().insert("wa-text", value);
    }
    status
}
