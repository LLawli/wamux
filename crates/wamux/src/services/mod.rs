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
pub mod newsletter_service;

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
    Ok(account_of(registry, account_ref).await?.1)
}

/// Resolve to the handle AND its live client. The handle owns the event bus, so
/// a send that has to publish its own echo (issue #22) needs both.
pub(crate) async fn account_of(
    registry: &AccountRegistry,
    account_ref: Option<&pb::AccountRef>,
) -> Result<(Arc<AccountHandle>, Arc<Client>), Status> {
    let handle = registry.resolve(account_ref)?;
    let client = handle
        .client()
        .await
        .ok_or_else(|| Status::failed_precondition("account is not connected"))?;
    Ok((handle, client))
}

/// This account's own jid, for the `sender` of a message it sent. Empty when
/// the client has no phone jid yet -- absent relays as the proto3 default, the
/// core does not substitute a placeholder.
pub(crate) fn own_jid(client: &Client) -> String {
    client.pn().map(|jid| jid.to_string()).unwrap_or_default()
}

/// Extract a `Jid` string from an optional proto `Jid`, erroring if absent.
pub(crate) fn require_jid(jid: Option<pb::Jid>) -> Result<String, Status> {
    jid.map(|j| j.value)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Status::invalid_argument("missing jid"))
}

/// Extract a required proto sub-message, or `InvalidArgument("missing <name>")`.
pub(crate) fn require_field<T>(field: Option<T>, name: &str) -> Result<T, Status> {
    field.ok_or_else(|| Status::invalid_argument(format!("missing {name}")))
}
