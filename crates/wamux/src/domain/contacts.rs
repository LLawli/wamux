//! Contact / profile helpers over `client.contacts()` and `client.profile()`.

use std::sync::Arc;

use wamux_types::{Jid, relay_lib_jid};
use whatsapp_rust::{
    Client, ProfilePictureRequest, ProfilePictureTarget, ProfilePictureType, PushNameOutcome,
};

use crate::domain::business_profile::business_profile_of;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

pub async fn check_on_whatsapp(
    client: Arc<Client>,
    jids: Vec<Jid>,
) -> Result<Vec<pb::CheckResult>, WamuxError> {
    // The core does NOT normalize identity: the edge sends well-formed JIDs
    // (e.g. "<number>@s.whatsapp.net"); the service already parsed them.
    let jids: Vec<whatsapp_rust::Jid> = jids.into_iter().map(Jid::into_lib).collect();
    // The library's usync future isn't Send (HRTB), so it cannot live inside the
    // #[async_trait] future. Drive it to completion in a blocking task on the
    // daemon's own runtime (`run_isolated`); only the owned result crosses back.
    let results = crate::domain::isolate::run_isolated(move || async move {
        client.contacts().is_on_whatsapp(&jids).await
    })
    .await?;
    Ok(results
        .into_iter()
        .map(|r| pb::CheckResult {
            query: relay_lib_jid(&r.jid),
            is_on_whatsapp: r.is_registered,
            jid: relay_lib_jid(&r.jid),
        })
        .collect())
}

pub async fn get_profile_picture(
    client: &Client,
    jid: &Jid,
) -> Result<pb::ProfilePictureResponse, WamuxError> {
    let request = ProfilePictureRequest::new(
        ProfilePictureTarget::Contact(jid.as_lib()),
        ProfilePictureType::Full,
    );
    let lookup = client
        .pictures()
        .lookup(request)
        .await
        .map_err(client_err)?;
    // Only a freshly found URL is relayed: "unchanged", "not found" and "not
    // authorized" all read as no URL, as they always did.
    Ok(pb::ProfilePictureResponse {
        url: lookup.into_found().map(|p| p.url).unwrap_or_default(),
        image: Vec::new(),
    })
}

pub async fn set_profile_picture(client: &Client, image: Vec<u8>) -> Result<(), WamuxError> {
    client
        .profile()
        .set_profile_picture(image)
        .await
        .map_err(client_err)?;
    Ok(())
}

pub async fn remove_profile_picture(client: &Client) -> Result<(), WamuxError> {
    client
        .profile()
        .remove_profile_picture()
        .await
        .map_err(client_err)?;
    Ok(())
}

pub async fn get_push_name(client: &Client) -> Result<pb::PushNameResponse, WamuxError> {
    Ok(pb::PushNameResponse {
        // The device snapshot is a sync `Arc` read, so there is nothing to await.
        push_name: client.push_name(),
    })
}

pub async fn set_push_name(client: &Client, name: &str) -> Result<(), WamuxError> {
    validate_push_name(name)?;
    let outcome = client
        .profile()
        .set_push_name(name)
        .await
        .map_err(client_err)?;
    // The name is set and presence was sent either way; only the app-state sync
    // to the other devices is pending. The wire has no field for that, so it is
    // logged for the operator and the edge can retry by setting the name again.
    if let PushNameOutcome::SyncPending { source } = outcome {
        tracing::warn!(error = %source, "push name set, app-state sync pending");
    }
    Ok(())
}

/// An empty push name is structurally invalid: the lib rejects it locally,
/// before any network I/O, with a bare `anyhow` (no IQ code). `client_err`
/// would then launder that into `Client` -> `Unavailable`/503, so a plain
/// caller mistake reads as "upstream down" at the edge. Reject it up front as
/// InvalidArgument, mirroring the empty-input guards in `wamux_types::Jid`.
/// We match the lib's own rule exactly (empty only):
/// trimming/blank policy is the edge's call, not the core's.
fn validate_push_name(name: &str) -> Result<(), WamuxError> {
    if name.is_empty() {
        return Err(WamuxError::InvalidArgument(
            "push name cannot be empty".to_string(),
        ));
    }
    Ok(())
}

pub async fn get_about(client: Arc<Client>, jid: Jid) -> Result<pb::AboutResponse, WamuxError> {
    let jid: whatsapp_rust::Jid = jid.into_lib();
    let lookup = jid.clone();
    let info = crate::domain::isolate::run_isolated(move || async move {
        client.contacts().get_user_info(&[lookup]).await
    })
    .await?;
    let about = info
        .get(&jid)
        .and_then(|u| u.status.clone())
        .unwrap_or_default();
    Ok(pb::AboutResponse { about })
}

pub async fn set_about(client: &Client, about: &str) -> Result<(), WamuxError> {
    client
        .profile()
        .set_status_text(about)
        .await
        .map_err(client_err)
}

pub async fn get_business_profile(
    client: &Client,
    jid: &Jid,
) -> Result<pb::BusinessProfileResponse, WamuxError> {
    let profile = client
        .get_business_profile(jid.as_lib())
        .await
        .map_err(client_err)?;
    Ok(pb::BusinessProfileResponse {
        profile: profile.as_ref().map(business_profile_of),
    })
}

pub async fn subscribe_presence(client: &Client, jid: &Jid) -> Result<(), WamuxError> {
    client
        .presence()
        .subscribe(jid.as_lib())
        .await
        .map_err(client_err)
}

#[cfg(test)]
#[path = "contacts_tests.rs"]
mod tests;
