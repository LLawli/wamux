//! Contact / profile helpers over `client.contacts()` and `client.profile()`.

use std::sync::Arc;

use whatsapp_rust::Client;

use crate::domain::jid_parse::parse_jid;
use crate::error::WamuxError;
use crate::proto::v1 as pb;

fn client_err<E: std::fmt::Display>(e: E) -> WamuxError {
    WamuxError::Client(format!("{e:#}"))
}

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
        push_name: client.get_push_name().await,
    })
}

pub async fn set_push_name(client: &Client, name: &str) -> Result<(), WamuxError> {
    client
        .profile()
        .set_push_name(name)
        .await
        .map_err(client_err)
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
