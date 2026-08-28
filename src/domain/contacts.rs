//! Contact / profile helpers over `client.contacts()` and `client.profile()`.

use std::sync::Arc;

use whatsapp_rust::Client;

use crate::domain::jid_parse::parse_jid;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

pub async fn check_on_whatsapp(
    client: Arc<Client>,
    jids: &[String],
) -> Result<Vec<pb::CheckResult>, WamuxError> {
    // The core does NOT normalize identity: the edge sends well-formed JIDs
    // (e.g. "<number>@s.whatsapp.net"). We only parse and validate them.
    let jids = jids
        .iter()
        .map(|j| parse_jid(j))
        .collect::<Result<Vec<_>, _>>()?;
    // The library's usync future isn't Send (HRTB), so it cannot live inside the
    // #[async_trait] future. Drive it to completion on its own current-thread
    // runtime in a blocking task; only the owned result crosses back.
    let results = crate::domain::isolate::run_isolated(move || async move {
        client.contacts().is_on_whatsapp(&jids).await
    })
    .await?;
    Ok(results
        .into_iter()
        .map(|r| pb::CheckResult {
            query: r.jid.to_string(),
            is_on_whatsapp: r.is_registered,
            jid: r.jid.to_string(),
        })
        .collect())
}

pub async fn get_profile_picture(
    client: &Client,
    jid: &str,
) -> Result<pb::ProfilePictureResponse, WamuxError> {
    let jid = parse_jid(jid)?;
    let picture = client
        .contacts()
        .get_profile_picture(&jid, false)
        .await
        .map_err(client_err)?;
    Ok(pb::ProfilePictureResponse {
        url: picture.map(|p| p.url).unwrap_or_default(),
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
        // 0.7 dropped the `get_` prefix and made it sync: the device snapshot
        // is an Arc read now, so there is nothing left to await.
        push_name: client.push_name(),
    })
}

pub async fn set_push_name(client: &Client, name: &str) -> Result<(), WamuxError> {
    validate_push_name(name)?;
    client
        .profile()
        .set_push_name(name)
        .await
        .map_err(client_err)
}

/// An empty push name is structurally invalid: the lib rejects it locally,
/// before any network I/O, with a bare `anyhow` (no IQ code). `client_err`
/// would then launder that into `Client` -> `Unavailable`/503, so a plain
/// caller mistake reads as "upstream down" at the edge. Reject it up front as
/// InvalidArgument, mirroring the empty-input guards in jid_parse / chat_actions
/// (E2E triage 2026-06-16). We match the lib's own rule exactly (empty only):
/// trimming/blank policy is the edge's call, not the core's.
fn validate_push_name(name: &str) -> Result<(), WamuxError> {
    if name.is_empty() {
        return Err(WamuxError::InvalidArgument(
            "push name cannot be empty".to_string(),
        ));
    }
    Ok(())
}

pub async fn get_about(client: Arc<Client>, jid: &str) -> Result<pb::AboutResponse, WamuxError> {
    let jid = parse_jid(jid)?;
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

pub async fn get_business_profile(
    client: &Client,
    jid: &str,
) -> Result<pb::BusinessProfileResponse, WamuxError> {
    let jid = parse_jid(jid)?;
    let profile = client
        .get_business_profile(&jid)
        .await
        .map_err(client_err)?;
    Ok(pb::BusinessProfileResponse {
        raw: serde_json::to_vec(&profile).unwrap_or_default(),
    })
}

pub async fn subscribe_presence(client: &Client, jid: &str) -> Result<(), WamuxError> {
    let jid = parse_jid(jid)?;
    client.presence().subscribe(&jid).await.map_err(client_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    // E2E triage 2026-06-16: an empty push name must fail fast as
    // InvalidArgument (the lib rejects it pre-network with a bare anyhow that
    // client_err would otherwise mislabel as Unavailable/503 at the edge).
    #[test]
    fn empty_push_name_is_invalid_argument() {
        let err = validate_push_name("").expect_err("empty name must be rejected");
        assert!(matches!(err, WamuxError::InvalidArgument(_)));
    }

    #[test]
    fn non_empty_push_name_is_accepted() {
        assert!(validate_push_name("Ana").is_ok());
        // A single space is non-empty: the core mirrors the lib's empty-only
        // rule and leaves any trim/blank policy to the edge.
        assert!(validate_push_name(" ").is_ok());
    }
}
