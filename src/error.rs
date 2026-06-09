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

    #[error(transparent)]
    Other(#[from] anyhow::Error),
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
        }
    }
}
