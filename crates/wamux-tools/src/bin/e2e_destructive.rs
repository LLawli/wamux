//! The destructive half of the old e2e_all (#64): profile name and picture
//! changed then restored, a NEW group created with WAMUX_LIVE_DEST as its
//! participant, then modified and left. Logout and DeleteAccount of the paired
//! account are NOT here: `logout_e2e` covers them.
//!
//! Refuses to run unless `WAMUX_E2E_DESTRUCTIVE` is exactly `yes`.
//!
//! Env: WAMUX_REF (required), WAMUX_LIVE_DEST (required, the group member),
//!      WAMUX_E2E_DESTRUCTIVE=yes, WAMUX_SOCKET_PATH.
//! See crates/wamux-tools/README.md.

use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::proto::v1::group_service_client::GroupServiceClient;
use wamux_tools::live_env::{
    account_ref_from, destructive_ack_from, live_dest_from, process_env, refuse_own_number,
    socket_path_from,
};
use wamux_tools::media_kit::{fetch_url_capped, jpeg_bytes};
use wamux_tools::profile::{PHOTO_CAP_BYTES, expect_photo_changed, photo_url, restore_photo};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};

const TEST_NAME: &str = "wamux e2e name";
const NEW_SUBJECT: &str = "wamux e2e renomeado";
const NEW_DESCRIPTION: &str = "descricao de teste wamux";
const INVITE_PREFIX: &str = "https://chat.whatsapp.com/";

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    // The whole config first: nothing is opened until every variable is valid.
    destructive_ack_from(&process_env)?;
    let external_ref: String = account_ref_from(&process_env)?;
    let dest: String = live_dest_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let acct = account_ref(&external_ref);
    let own = wait_connected(&mut account, &acct, Duration::from_secs(30)).await?;
    refuse_own_number(&own, &dest)?;
    println!("=== DESTRUCTIVE PHASE (authorized) as {own} ===");

    let mut report = Report::new();
    let mut contacts = ContactServiceClient::new(channel.clone());
    check_push_name(&mut contacts, &mut report, &acct).await;
    check_profile_picture(&mut contacts, &mut report, &acct, &own).await;
    let mut groups = GroupServiceClient::new(channel);
    run_group(&mut groups, &mut report, &acct, &dest).await;
    Ok(report.finish())
}

// ---- Profile push name (set + restore) ----

async fn push_name_of(
    contacts: &mut ContactServiceClient<Channel>,
    acct: &pb::AccountRef,
) -> Result<String, tonic::Status> {
    let response = contacts.get_push_name(acct.clone()).await?;
    Ok(response.into_inner().push_name)
}

async fn set_push_name(
    contacts: &mut ContactServiceClient<Channel>,
    acct: &pb::AccountRef,
    name: &str,
) -> Result<tonic::Response<pb::Empty>, tonic::Status> {
    contacts
        .set_push_name(pb::SetPushNameRequest {
            account: Some(acct.clone()),
            push_name: name.to_string(),
        })
        .await
}

/// Set the test name, prove it reads back, then restore and prove that too.
async fn check_push_name(
    contacts: &mut ContactServiceClient<Channel>,
    report: &mut Report,
    acct: &pb::AccountRef,
) {
    // The original must be known and non-empty before anything is written: an
    // unreadable name and an empty one both make the restore unverifiable
    // (restoring an empty name would wipe the real one).
    let original = match push_name_of(contacts, acct).await {
        Ok(name) if !name.is_empty() => name,
        Ok(_) => {
            return report.fail(
                "Contact.GetPushName (original)",
                "the original push name is empty, so a restore could not be told from a failure; not changing it",
            );
        }
        Err(status) => {
            return report.fail("Contact.GetPushName (original)", status.to_string());
        }
    };
    let set = set_push_name(contacts, acct, TEST_NAME).await;
    if report.accepted_rpc("Contact.SetPushName", set).is_some() {
        let now = push_name_of(contacts, acct).await.unwrap_or_default();
        report.verify(
            "Contact.GetPushName after set",
            now == TEST_NAME,
            format!("read back {now:?}, expected {TEST_NAME:?}"),
        );
    }
    let restored = set_push_name(contacts, acct, &original).await;
    if report
        .accepted_rpc("Contact.SetPushName(restore)", restored)
        .is_some()
    {
        let now = push_name_of(contacts, acct).await.unwrap_or_default();
        report.verify(
            "Contact.GetPushName after restore",
            now == original,
            format!("read back {now:?}, expected {original:?}"),
        );
    }
}

// ---- Profile picture (guarded: only test if we can restore) ----

async fn check_profile_picture(
    contacts: &mut ContactServiceClient<Channel>,
    report: &mut Report,
    acct: &pb::AccountRef,
    own: &str,
) {
    // A failed read of the ORIGINAL photo must not be taken for "no photo":
    // the restore would then delete a picture that was never looked at.
    let orig_url = match photo_url(contacts, acct, own).await {
        Ok(url) => url,
        Err(status) => {
            return report.fail("Contact.GetProfilePicture (original)", status.to_string());
        }
    };
    let had_photo = !orig_url.is_empty();
    let orig_bytes = match had_photo {
        true => fetch_url_capped(&orig_url, PHOTO_CAP_BYTES).await.ok(),
        false => None,
    };
    if had_photo && orig_bytes.is_none() {
        return report.fail(
            "Contact.SetProfilePicture",
            "SKIPPED: current photo present but unfetchable (protecting it)",
        );
    }
    let image = match jpeg_bytes(640) {
        Ok(bytes) => bytes,
        Err(e) => return report.fail("Contact.SetProfilePicture", e.to_string()),
    };
    let request = pb::SetProfilePictureRequest {
        account: Some(acct.clone()),
        image,
    };
    let set = contacts.set_profile_picture(request).await;
    if report
        .accepted_rpc("Contact.SetProfilePicture", set)
        .is_some()
    {
        expect_photo_changed(contacts, report, acct, own, &orig_url).await;
    }
    restore_photo(contacts, report, acct, orig_bytes, had_photo).await;
}

// ---- Groups: create NEW, modify, leave ----

/// What every group call names: the account, the new group and its member.
struct GroupRun {
    acct: pb::AccountRef,
    jid: String,
    member: String,
}

impl GroupRun {
    fn gref(&self) -> pb::GroupRef {
        pb::GroupRef {
            account: Some(self.acct.clone()),
            group: Some(pb::Jid {
                value: self.jid.clone(),
            }),
        }
    }

    fn text(&self, text: &str) -> pb::GroupTextRequest {
        pb::GroupTextRequest {
            account: Some(self.acct.clone()),
            group: Some(pb::Jid {
                value: self.jid.clone(),
            }),
            text: text.to_string(),
        }
    }

    fn members(&self) -> pb::ParticipantsRequest {
        pb::ParticipantsRequest {
            account: Some(self.acct.clone()),
            group: Some(pb::Jid {
                value: self.jid.clone(),
            }),
            participants: vec![pb::Jid {
                value: self.member.clone(),
            }],
        }
    }
}

async fn run_group(
    groups: &mut GroupServiceClient<Channel>,
    report: &mut Report,
    acct: &pb::AccountRef,
    dest: &str,
) {
    let created = groups
        .create_group(pb::CreateGroupRequest {
            account: Some(acct.clone()),
            subject: format!("wamux e2e {}", nanos()),
            participants: vec![pb::Jid {
                value: dest.to_string(),
            }],
        })
        .await;
    let jid = match created {
        Ok(r) => r
            .into_inner()
            .group
            .map(|group| group.value)
            .unwrap_or_default(),
        Err(e) => return report.fail("Group.CreateGroup", e.to_string()),
    };
    report.verify(
        "Group.CreateGroup",
        jid.ends_with("@g.us"),
        format!("jid={jid}"),
    );
    let run = GroupRun {
        acct: acct.clone(),
        jid,
        member: dest.to_string(),
    };
    // Whatever came back, if it names something the group exists: try to leave
    // it instead of abandoning it, even when the jid is not shaped like a group.
    if !run.jid.ends_with("@g.us") {
        if !run.jid.is_empty() {
            let left = groups.leave_group(run.gref()).await;
            report.accepted_rpc("Group.LeaveGroup (unexpected jid)", left);
        }
        return;
    }
    rename_and_describe(groups, report, &run).await;
    check_invite_link(groups, report, &run).await;
    change_members(groups, report, &run).await;
    let left = groups.leave_group(run.gref()).await;
    report.accepted_rpc("Group.LeaveGroup", left);
}

/// Subject and description answer Empty, so the proof is the metadata read
/// back: it must carry both new values.
async fn rename_and_describe(
    groups: &mut GroupServiceClient<Channel>,
    report: &mut Report,
    run: &GroupRun,
) {
    let subject = groups.set_group_subject(run.text(NEW_SUBJECT)).await;
    report.accepted_rpc("Group.SetGroupSubject", subject);
    let description = groups
        .set_group_description(run.text(NEW_DESCRIPTION))
        .await;
    report.accepted_rpc("Group.SetGroupDescription", description);
    match groups.get_group_metadata(run.gref()).await {
        Ok(r) => {
            let metadata = r.into_inner().metadata.unwrap_or_default();
            report.verify(
                "Group.GetGroupMetadata after subject+description",
                metadata.subject.as_deref() == Some(NEW_SUBJECT)
                    && metadata.description.as_deref() == Some(NEW_DESCRIPTION),
                format!(
                    "subject {:?}, description {:?}",
                    metadata.subject, metadata.description
                ),
            );
        }
        Err(e) => report.fail(
            "Group.GetGroupMetadata after subject+description",
            e.to_string(),
        ),
    }
}

/// A fresh link has the invite prefix, and revoking it hands back a different one.
async fn check_invite_link(
    groups: &mut GroupServiceClient<Channel>,
    report: &mut Report,
    run: &GroupRun,
) {
    let first = match groups.get_invite_link(run.gref()).await {
        Ok(r) => r.into_inner().link,
        Err(e) => return report.fail("Group.GetInviteLink", e.to_string()),
    };
    report.verify(
        "Group.GetInviteLink",
        first.starts_with(INVITE_PREFIX),
        format!("link={first}"),
    );
    match groups.revoke_invite_link(run.gref()).await {
        Ok(r) => {
            let second = r.into_inner().link;
            report.verify(
                "Group.RevokeInviteLink",
                second.starts_with(INVITE_PREFIX) && second != first,
                format!("new link={second}"),
            );
        }
        Err(e) => report.fail("Group.RevokeInviteLink", e.to_string()),
    }
}

async fn change_members(
    groups: &mut GroupServiceClient<Channel>,
    report: &mut Report,
    run: &GroupRun,
) {
    let promoted = groups.promote_admins(run.members()).await;
    judge_members(report, "Group.PromoteAdmins", promoted);
    let demoted = groups.demote_admins(run.members()).await;
    judge_members(report, "Group.DemoteAdmins", demoted);
    let removed = groups.remove_participants(run.members()).await;
    judge_members(report, "Group.RemoveParticipants", removed);
}

/// The server answers per participant: a pass needs an answer for the member
/// and no per-participant error in it.
fn judge_members(
    report: &mut Report,
    name: &str,
    result: Result<tonic::Response<pb::ParticipantsResponse>, tonic::Status>,
) {
    let results = match result {
        Ok(r) => r.into_inner().results,
        Err(e) => return report.fail(name, e.to_string()),
    };
    let errors: Vec<&str> = results
        .iter()
        .map(|c| c.error.as_str())
        .filter(|e| !e.is_empty())
        .collect();
    report.verify(
        name,
        !results.is_empty() && errors.is_empty(),
        format!("{} answer(s), errors={errors:?}", results.len()),
    );
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}
