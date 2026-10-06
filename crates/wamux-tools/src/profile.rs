//! Profile picture helpers shared by `set_pfp` and `e2e_destructive` (#64):
//! read the current picture, prove a change landed, and put the original back.

use std::time::Duration;

use tonic::transport::Channel;
use wamux::proto::v1 as pb;
use wamux::proto::v1::contact_service_client::ContactServiceClient;

use crate::report::Report;

/// The largest current picture worth fetching back for the restore.
pub const PHOTO_CAP_BYTES: usize = 16 * 1024 * 1024;
/// How many one-second reads wait for the server to show a changed picture.
const READBACK_TRIES: usize = 10;

/// The URL of `jid`'s profile picture; an empty string means the account
/// genuinely has none. A failed read is an error, never "no photo": the
/// restore step would otherwise delete a picture that was never looked at.
pub async fn photo_url(
    contacts: &mut ContactServiceClient<Channel>,
    acct: &pb::AccountRef,
    jid: &str,
) -> Result<String, tonic::Status> {
    let request = pb::JidRequest {
        account: Some(acct.clone()),
        jid: Some(pb::Jid {
            value: jid.to_string(),
        }),
    };
    let response = contacts.get_profile_picture(request).await?;
    Ok(response.into_inner().url)
}

/// The asserted value after a set: the URL read back differs from the one
/// before (or exists, when there was none). Polled, since the server can lag
/// a moment behind the set.
pub async fn expect_photo_changed(
    contacts: &mut ContactServiceClient<Channel>,
    report: &mut Report,
    acct: &pb::AccountRef,
    own: &str,
    before: &str,
) {
    let mut now = String::new();
    for _ in 0..READBACK_TRIES {
        // A failed read is "not changed yet": the check then fails below.
        now = photo_url(contacts, acct, own).await.unwrap_or_default();
        if !now.is_empty() && now != before {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    report.verify(
        "Contact.GetProfilePicture after set",
        !now.is_empty() && now != before,
        format!("url before={before:?} after={now:?}"),
    );
}

/// Put the original photo back, or remove ours when there was none. The
/// result is a check: a failed restore leaves the account changed and the
/// run must say so.
pub async fn restore_photo(
    contacts: &mut ContactServiceClient<Channel>,
    report: &mut Report,
    acct: &pb::AccountRef,
    original: Option<Vec<u8>>,
    had_photo: bool,
) {
    if let Some(bytes) = original {
        let request = pb::SetProfilePictureRequest {
            account: Some(acct.clone()),
            image: bytes,
        };
        let result = contacts.set_profile_picture(request).await;
        report.accepted_rpc("Contact.SetProfilePicture (restore original)", result);
    } else if had_photo {
        report.fail(
            "Contact.SetProfilePicture (restore original)",
            "the original photo could not be fetched back; the account keeps the test image",
        );
    } else {
        let result = contacts.remove_profile_picture(acct.clone()).await;
        report.accepted_rpc("Contact.RemoveProfilePicture (restore: had none)", result);
    }
}
