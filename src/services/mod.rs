//! Thin gRPC service impls: validate the request, resolve the account, call a
//! `domain` function, map the result/error to a `Status`. No business logic here.

// tonic's `Status` is the idiomatic gRPC error type and is intentionally large;
// boxing it everywhere would be non-idiomatic noise.
#![allow(clippy::result_large_err)]

pub mod account_service;
pub mod admin_service;
pub mod contact_service;
pub mod event_service;
pub mod group_service;
pub mod media_service;
pub mod messaging_service;

use std::sync::Arc;

use tonic::Status;
use whatsapp_rust::Client;

use crate::proto::v1 as pb;
use crate::state::{AccountHandle, AccountRegistry};

/// Project a handle into the proto `Account` (jid filled separately when needed).
pub(crate) fn account_to_proto(handle: &AccountHandle) -> pb::Account {
    pb::Account {
        uuid: handle.uuid.to_string(),
        external_ref: handle.external_ref.clone().unwrap_or_default(),
        state: handle.current_state() as i32,
        jid: None,
        push_name: handle.push_name.clone().unwrap_or_default(),
    }
}

/// Resolve an account ref to its live `Client`, or `FailedPrecondition`.
pub(crate) async fn client_of(
    registry: &AccountRegistry,
    account_ref: Option<&pb::AccountRef>,
) -> Result<Arc<Client>, Status> {
    let handle = registry.resolve(account_ref)?;
    handle
        .client()
        .await
        .ok_or_else(|| Status::failed_precondition("account is not connected"))
}

/// Extract a `Jid` string from an optional proto `Jid`, erroring if absent.
pub(crate) fn require_jid(jid: Option<pb::Jid>) -> Result<String, Status> {
    jid.map(|j| j.value)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Status::invalid_argument("missing jid"))
}
