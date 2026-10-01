//! Test SetProfilePicture (square JPEG) + restore on the already-paired,
//! stably-connected account.
//!
//! Env: WAMUX_REF (required), WAMUX_SOCKET_PATH.

use std::process::ExitCode;
use std::time::Duration;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux_tools::live_env::{account_ref_from, process_env, socket_path_from};
use wamux_tools::media_kit::{fetch_url_capped, jpeg_bytes};
use wamux_tools::profile::{PHOTO_CAP_BYTES, expect_photo_changed, photo_url, restore_photo};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut contacts = ContactServiceClient::new(channel);
    let acct = account_ref(&external_ref);

    let own = wait_connected(&mut account, &acct, Duration::from_secs(30)).await?;
    println!("connected; own={own}");
    let image = jpeg_bytes(640)?;
    let mut report = Report::new();

    // A failed read of the ORIGINAL photo must not be taken for "no photo":
    // the restore would then delete a picture that was never looked at.
    let orig_url = match photo_url(&mut contacts, &acct, &own).await {
        Ok(url) => url,
        Err(status) => {
            report.fail("Contact.GetProfilePicture (original)", status.to_string());
            return Ok(report.finish());
        }
    };
    let had_photo = !orig_url.is_empty();
    let orig = match had_photo {
        true => fetch_url_capped(&orig_url, PHOTO_CAP_BYTES).await.ok(),
        false => None,
    };
    println!(
        "current photo: {}",
        if had_photo { "present" } else { "none" }
    );
    let request = pb::SetProfilePictureRequest {
        account: Some(acct.clone()),
        image,
    };
    let set = contacts.set_profile_picture(request).await;
    if report
        .accepted_rpc("Contact.SetProfilePicture", set)
        .is_some()
    {
        expect_photo_changed(&mut contacts, &mut report, &acct, &own, &orig_url).await;
    }
    restore_photo(&mut contacts, &mut report, &acct, orig, had_photo).await;
    Ok(report.finish())
}
