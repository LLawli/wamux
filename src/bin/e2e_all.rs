//! Comprehensive E2E across the whole wamux socket API (non-destructive subset).
//! Prints PASS/FAIL per RPC and a summary. Destructive ops (create/modify group,
//! profile changes, logout/delete of the paired account) are intentionally excluded.
//!
//! Usage: e2e_all [socket_path] [target_number]
//!   defaults: /tmp/wamux.sock  5511999999999
//! Requires the daemon running with the "pair-socket" account already paired.
//! The core relays to the JID verbatim (no routing); this client targets @c.us
//! so delivery dodges the library's PN->LID upgrade.

use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::admin_service_client::AdminServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::group_service_client::GroupServiceClient;
use wamux::proto::v1::media_service_client::MediaServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

const EXTERNAL_REF: &str = "pair-socket";
type Results = Arc<Mutex<Vec<(String, bool, String)>>>;

fn rec(results: &Results, name: &str, ok: bool, detail: String) {
    let mark = if ok { "PASS" } else { "FAIL" };
    println!("[{mark}] {name} :: {detail}");
    results.lock().unwrap().push((name.to_string(), ok, detail));
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/wamux.sock".to_string());
    let number = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "5511999999999".to_string());
    let target = format!("{number}@c.us");
    let results: Results = Arc::new(Mutex::new(Vec::new()));

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut admin = AdminServiceClient::new(channel.clone());
    let mut contacts = ContactServiceClient::new(channel.clone());
    let mut groups = GroupServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut media = MediaServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);

    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(EXTERNAL_REF.to_string())),
    };

    // --- AdminService ---
    match admin.get_metrics(pb::Empty {}).await {
        Ok(r) => rec(
            &results,
            "Admin.GetMetrics",
            true,
            format!("{} bytes", r.into_inner().prometheus.len()),
        ),
        Err(e) => rec(&results, "Admin.GetMetrics", false, e.message().to_string()),
    }

    // --- AccountService lifecycle on a THROWAWAY account (reversible) ---
    let tmp_ref = format!("e2e-tmp-{}", nanos());
    let tmp = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(tmp_ref.clone())),
    };
    match account
        .create_account(pb::CreateAccountRequest {
            external_ref: Some(tmp_ref.clone()),
        })
        .await
    {
        Ok(r) => rec(
            &results,
            "Account.CreateAccount",
            true,
            format!("uuid={}", r.into_inner().uuid),
        ),
        Err(e) => rec(
            &results,
            "Account.CreateAccount",
            false,
            e.message().to_string(),
        ),
    }
    match account.list_accounts(pb::ListAccountsRequest {}).await {
        Ok(r) => {
            let n = r.into_inner().accounts.len();
            rec(
                &results,
                "Account.ListAccounts",
                true,
                format!("{n} accounts"),
            );
        }
        Err(e) => rec(
            &results,
            "Account.ListAccounts",
            false,
            e.message().to_string(),
        ),
    }
    match account.get_account_status(tmp.clone()).await {
        Ok(r) => rec(
            &results,
            "Account.GetAccountStatus(tmp)",
            true,
            format!("state={}", r.into_inner().state),
        ),
        Err(e) => rec(
            &results,
            "Account.GetAccountStatus(tmp)",
            false,
            e.message().to_string(),
        ),
    }
    match account.delete_account(tmp.clone()).await {
        Ok(_) => rec(
            &results,
            "Account.DeleteAccount(tmp)",
            true,
            "deleted".to_string(),
        ),
        Err(e) => rec(
            &results,
            "Account.DeleteAccount(tmp)",
            false,
            e.message().to_string(),
        ),
    }

    // --- Connect the paired account ---
    match account
        .connect_account(pb::ConnectAccountRequest {
            account: Some(acct.clone()),
            backfill_history: false,
        })
        .await
    {
        Ok(_) => rec(&results, "Account.ConnectAccount", true, "ok".to_string()),
        Err(e) => rec(
            &results,
            "Account.ConnectAccount",
            false,
            e.message().to_string(),
        ),
    }
    let mut connected = false;
    for _ in 0..100 {
        if let Ok(s) = account.get_account_status(acct.clone()).await
            && s.into_inner().state == pb::ConnectionState::Connected as i32
        {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    rec(
        &results,
        "Account reached CONNECTED",
        connected,
        format!("connected={connected}"),
    );
    if !connected {
        print_summary(&results);
        anyhow::bail!("not connected; aborting");
    }

    // --- EventService subscription (background) capturing inbound + media ---
    let captured_media: Arc<Mutex<Option<pb::MediaDescriptor>>> = Arc::new(Mutex::new(None));
    let inbound_count = Arc::new(Mutex::new(0usize));
    {
        let sub = pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            replay_from_ring: 0,
        };
        match events.subscribe_events(sub).await {
            Ok(stream) => {
                rec(
                    &results,
                    "Event.SubscribeEvents",
                    true,
                    "stream open".to_string(),
                );
                let cm = captured_media.clone();
                let ic = inbound_count.clone();
                let mut s = stream.into_inner();
                tokio::spawn(async move {
                    while let Ok(Some(env)) = s.message().await {
                        if let Some(pb::event_envelope::Event::Message(m)) = env.event {
                            *ic.lock().unwrap() += 1;
                            println!(
                                "   [recv] from={} text={:?} media={:?}",
                                m.sender,
                                m.text,
                                m.media.as_ref().map(|d| &d.media_type)
                            );
                            if let Some(d) = m.media {
                                *cm.lock().unwrap() = Some(d);
                            }
                        }
                    }
                });
            }
            Err(e) => rec(
                &results,
                "Event.SubscribeEvents",
                false,
                e.message().to_string(),
            ),
        }
    }

    // --- ContactService (read-only) ---
    match contacts
        .check_on_whats_app(pb::CheckOnWhatsAppRequest {
            account: Some(acct.clone()),
            jids: vec![format!("{number}@s.whatsapp.net")],
        })
        .await
    {
        Ok(r) => {
            let res = r.into_inner().results;
            let detail = res
                .first()
                .map(|x| format!("on_wa={} jid={}", x.is_on_whatsapp, x.jid))
                .unwrap_or_else(|| "no result".to_string());
            rec(&results, "Contact.CheckOnWhatsApp", !res.is_empty(), detail);
        }
        Err(e) => rec(
            &results,
            "Contact.CheckOnWhatsApp",
            false,
            e.message().to_string(),
        ),
    }
    let jid_req = |j: &str| pb::JidRequest {
        account: Some(acct.clone()),
        jid: j.to_string(),
    };
    match contacts.get_push_name(jid_req(&target)).await {
        Ok(r) => rec(
            &results,
            "Contact.GetPushName",
            true,
            format!("name={:?}", r.into_inner().push_name),
        ),
        Err(e) => rec(
            &results,
            "Contact.GetPushName",
            false,
            e.message().to_string(),
        ),
    }
    match contacts.get_about(jid_req(&target)).await {
        Ok(r) => rec(
            &results,
            "Contact.GetAbout",
            true,
            format!("about={:?}", r.into_inner().about),
        ),
        Err(e) => rec(&results, "Contact.GetAbout", false, e.message().to_string()),
    }
    match contacts.get_profile_picture(jid_req(&target)).await {
        Ok(r) => rec(
            &results,
            "Contact.GetProfilePicture",
            true,
            format!("url_len={}", r.into_inner().url.len()),
        ),
        Err(e) => rec(
            &results,
            "Contact.GetProfilePicture",
            false,
            e.message().to_string(),
        ),
    }
    match contacts.get_business_profile(jid_req(&target)).await {
        Ok(r) => rec(
            &results,
            "Contact.GetBusinessProfile",
            true,
            format!("raw_len={}", r.into_inner().raw.len()),
        ),
        Err(e) => rec(
            &results,
            "Contact.GetBusinessProfile",
            false,
            e.message().to_string(),
        ),
    }
    match contacts
        .subscribe_presence(pb::SubscribePresenceRequest {
            account: Some(acct.clone()),
            jid: target.clone(),
        })
        .await
    {
        Ok(_) => rec(
            &results,
            "Contact.SubscribePresence",
            true,
            "ok".to_string(),
        ),
        Err(e) => rec(
            &results,
            "Contact.SubscribePresence",
            false,
            e.message().to_string(),
        ),
    }

    // --- GroupService (read-only) ---
    let mut first_group: Option<String> = None;
    match groups.list_participating(acct.clone()).await {
        Ok(r) => {
            let gs = r.into_inner().groups;
            first_group = gs.first().map(|g| g.jid.clone());
            rec(
                &results,
                "Group.ListParticipating",
                true,
                format!("{} groups", gs.len()),
            );
        }
        Err(e) => rec(
            &results,
            "Group.ListParticipating",
            false,
            e.message().to_string(),
        ),
    }
    if let Some(gjid) = first_group.clone() {
        let gref = pb::GroupRef {
            account: Some(acct.clone()),
            group_jid: gjid.clone(),
        };
        match groups.get_group_metadata(gref.clone()).await {
            Ok(r) => rec(
                &results,
                "Group.GetGroupMetadata",
                true,
                format!("meta_len={}", r.into_inner().metadata.len()),
            ),
            Err(e) => rec(
                &results,
                "Group.GetGroupMetadata",
                false,
                e.message().to_string(),
            ),
        }
        match groups.get_invite_link(gref).await {
            Ok(r) => rec(&results, "Group.GetInviteLink", true, r.into_inner().link),
            Err(e) => rec(
                &results,
                "Group.GetInviteLink",
                false,
                format!("{} (often needs admin)", e.message()),
            ),
        }
    }

    // --- MessagingService to target ---
    let mut text_id = String::new();
    match messaging
        .send_text(pb::SendTextRequest {
            account: Some(acct.clone()),
            to: Some(pb::Jid {
                value: target.clone(),
            }),
            text: "wamux e2e_all: texto ✅".to_string(),
            mentions: vec![],
            quote: None,
        })
        .await
    {
        Ok(r) => {
            text_id = r.into_inner().key.map(|k| k.id).unwrap_or_default();
            rec(
                &results,
                "Messaging.SendText",
                true,
                format!("id={text_id}"),
            );
        }
        Err(e) => rec(
            &results,
            "Messaging.SendText",
            false,
            e.message().to_string(),
        ),
    }
    let key = pb::MessageKey {
        remote_jid: target.clone(),
        id: text_id.clone(),
        from_me: true,
        participant: String::new(),
    };
    match messaging
        .send_presence(pb::SendPresenceRequest {
            account: Some(acct.clone()),
            chat: Some(pb::Jid {
                value: target.clone(),
            }),
            state: "composing".to_string(),
        })
        .await
    {
        Ok(_) => rec(
            &results,
            "Messaging.SendPresence(composing)",
            true,
            "ok".to_string(),
        ),
        Err(e) => rec(
            &results,
            "Messaging.SendPresence(composing)",
            false,
            e.message().to_string(),
        ),
    }
    if !text_id.is_empty() {
        match messaging
            .send_reaction(pb::SendReactionRequest {
                account: Some(acct.clone()),
                target: Some(key.clone()),
                emoji: "👍".to_string(),
            })
            .await
        {
            Ok(r) => rec(
                &results,
                "Messaging.SendReaction",
                true,
                format!(
                    "id={}",
                    r.into_inner().key.map(|k| k.id).unwrap_or_default()
                ),
            ),
            Err(e) => rec(
                &results,
                "Messaging.SendReaction",
                false,
                e.message().to_string(),
            ),
        }
        match messaging
            .edit_message(pb::EditMessageRequest {
                account: Some(acct.clone()),
                target: Some(key.clone()),
                new_text: "wamux e2e_all: texto (editado) ✏️".to_string(),
            })
            .await
        {
            Ok(r) => rec(
                &results,
                "Messaging.EditMessage",
                true,
                format!(
                    "id={}",
                    r.into_inner().key.map(|k| k.id).unwrap_or_default()
                ),
            ),
            Err(e) => rec(
                &results,
                "Messaging.EditMessage",
                false,
                e.message().to_string(),
            ),
        }
    }
    match send_media(&mut messaging, &acct, &target, generate_png()?).await {
        Ok(id) => rec(
            &results,
            "Messaging.SendMedia(image)",
            true,
            format!("id={id}"),
        ),
        Err(e) => rec(&results, "Messaging.SendMedia(image)", false, e),
    }
    match messaging
        .mark_read(pb::MarkReadRequest {
            account: Some(acct.clone()),
            chat: Some(pb::Jid {
                value: target.clone(),
            }),
            message_ids: vec![],
            sender: None,
        })
        .await
    {
        Ok(_) => rec(&results, "Messaging.MarkRead", true, "ok".to_string()),
        Err(e) => rec(
            &results,
            "Messaging.MarkRead",
            false,
            e.message().to_string(),
        ),
    }

    // --- Reception window + MediaService.DownloadMedia ---
    println!(
        "\n>>> Send a TEXT and an IMAGE to the paired account now (for reception + DownloadMedia). Waiting 60s ...\n"
    );
    for _ in 0..60 {
        if captured_media.lock().unwrap().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let got_inbound = *inbound_count.lock().unwrap() > 0;
    rec(
        &results,
        "Event reception (inbound msg)",
        got_inbound,
        format!("{} inbound", *inbound_count.lock().unwrap()),
    );
    let desc = captured_media.lock().unwrap().clone();
    match desc {
        Some(d) => {
            match media
                .download_media(pb::DownloadMediaRequest {
                    account: Some(acct.clone()),
                    descriptor: Some(d),
                })
                .await
            {
                Ok(stream) => {
                    let mut s = stream.into_inner();
                    let mut total = 0usize;
                    while let Ok(Some(chunk)) = s.message().await {
                        if let Some(pb::media_chunk::Part::Chunk(b)) = chunk.part {
                            total += b.len();
                        }
                    }
                    rec(
                        &results,
                        "Media.DownloadMedia",
                        total > 0,
                        format!("{total} bytes"),
                    );
                }
                Err(e) => rec(
                    &results,
                    "Media.DownloadMedia",
                    false,
                    e.message().to_string(),
                ),
            }
        }
        None => rec(
            &results,
            "Media.DownloadMedia",
            false,
            "skipped: no inbound media received in window".to_string(),
        ),
    }

    // --- delete (revoke) our text to exercise DeleteMessage (reversible: our own msg) ---
    if !text_id.is_empty() {
        match messaging
            .delete_message(pb::DeleteMessageRequest {
                account: Some(acct.clone()),
                target: Some(key),
                for_everyone: true,
            })
            .await
        {
            Ok(_) => rec(
                &results,
                "Messaging.DeleteMessage(revoke)",
                true,
                "ok".to_string(),
            ),
            Err(e) => rec(
                &results,
                "Messaging.DeleteMessage(revoke)",
                false,
                e.message().to_string(),
            ),
        }
    }

    if std::env::var("WAMUX_E2E_DESTRUCTIVE").is_ok() {
        run_destructive(
            &mut account,
            &mut contacts,
            &mut groups,
            &acct,
            &number,
            &results,
        )
        .await;
    }

    print_summary(&results);
    Ok(())
}

/// Destructive phase (authorized): groups created NEW then left; profile
/// name/photo changed then restored; logout + delete the paired account LAST.
async fn run_destructive(
    account: &mut AccountServiceClient<Channel>,
    contacts: &mut ContactServiceClient<Channel>,
    groups: &mut GroupServiceClient<Channel>,
    acct: &pb::AccountRef,
    number: &str,
    results: &Results,
) {
    println!("\n=== DESTRUCTIVE PHASE (authorized) ===");
    let jid_req = |j: &str| pb::JidRequest {
        account: Some(acct.clone()),
        jid: j.to_string(),
    };
    let own = account
        .get_account_status(acct.clone())
        .await
        .ok()
        .and_then(|r| r.into_inner().jid)
        .map(|j| j.value)
        .unwrap_or_default();

    // ---- Profile push name (set + restore) ----
    let orig_name = contacts
        .get_push_name(jid_req(&own))
        .await
        .map(|r| r.into_inner().push_name)
        .unwrap_or_default();
    match contacts
        .set_push_name(pb::SetPushNameRequest {
            account: Some(acct.clone()),
            push_name: "wamux e2e name".to_string(),
        })
        .await
    {
        Ok(_) => rec(
            results,
            "Contact.SetPushName",
            true,
            "set test name".to_string(),
        ),
        Err(e) => rec(
            results,
            "Contact.SetPushName",
            false,
            e.message().to_string(),
        ),
    }
    match contacts
        .set_push_name(pb::SetPushNameRequest {
            account: Some(acct.clone()),
            push_name: orig_name.clone(),
        })
        .await
    {
        Ok(_) => rec(
            results,
            "Contact.SetPushName(restore)",
            true,
            format!("restored to {orig_name:?}"),
        ),
        Err(e) => rec(
            results,
            "Contact.SetPushName(restore)",
            false,
            e.message().to_string(),
        ),
    }

    // ---- Profile picture (guarded: only test if we can restore) ----
    let orig_url = contacts
        .get_profile_picture(jid_req(&own))
        .await
        .map(|r| r.into_inner().url)
        .unwrap_or_default();
    let orig_bytes = if orig_url.is_empty() {
        None
    } else {
        fetch_url_bytes(&orig_url)
    };
    if orig_url.is_empty() || orig_bytes.is_some() {
        let test = generate_png().unwrap_or_default();
        match contacts
            .set_profile_picture(pb::SetProfilePictureRequest {
                account: Some(acct.clone()),
                image: test,
            })
            .await
        {
            Ok(_) => rec(
                results,
                "Contact.SetProfilePicture",
                true,
                "set test photo".to_string(),
            ),
            Err(e) => rec(
                results,
                "Contact.SetProfilePicture",
                false,
                e.message().to_string(),
            ),
        }
        if let Some(bytes) = orig_bytes {
            match contacts
                .set_profile_picture(pb::SetProfilePictureRequest {
                    account: Some(acct.clone()),
                    image: bytes,
                })
                .await
            {
                Ok(_) => rec(
                    results,
                    "Contact.SetProfilePicture(restore)",
                    true,
                    "restored original".to_string(),
                ),
                Err(e) => rec(
                    results,
                    "Contact.SetProfilePicture(restore)",
                    false,
                    e.message().to_string(),
                ),
            }
        } else {
            match contacts.remove_profile_picture(acct.clone()).await {
                Ok(_) => rec(
                    results,
                    "Contact.RemoveProfilePicture(restore none)",
                    true,
                    "no original; removed".to_string(),
                ),
                Err(e) => rec(
                    results,
                    "Contact.RemoveProfilePicture(restore none)",
                    false,
                    e.message().to_string(),
                ),
            }
        }
    } else {
        rec(
            results,
            "Contact.SetProfilePicture",
            false,
            "SKIPPED: current photo present but unfetchable (protecting it)".to_string(),
        );
    }

    // ---- Groups: create NEW, modify, leave ----
    let part = format!("{number}@c.us");
    match groups
        .create_group(pb::CreateGroupRequest {
            account: Some(acct.clone()),
            subject: format!("wamux e2e {}", nanos()),
            participants: vec![part.clone()],
        })
        .await
    {
        Ok(r) => {
            let gjid = r.into_inner().group_jid;
            rec(
                results,
                "Group.CreateGroup",
                !gjid.is_empty(),
                format!("jid={gjid}"),
            );
            let gref = pb::GroupRef {
                account: Some(acct.clone()),
                group_jid: gjid.clone(),
            };
            let gtext = |t: &str| pb::GroupTextRequest {
                account: Some(acct.clone()),
                group_jid: gjid.clone(),
                text: t.to_string(),
            };
            let gpart = pb::ParticipantsRequest {
                account: Some(acct.clone()),
                group_jid: gjid.clone(),
                participants: vec![part.clone()],
            };

            match groups.set_group_subject(gtext("wamux e2e renomeado")).await {
                Ok(_) => rec(results, "Group.SetGroupSubject", true, "ok".to_string()),
                Err(e) => rec(
                    results,
                    "Group.SetGroupSubject",
                    false,
                    e.message().to_string(),
                ),
            }
            match groups
                .set_group_description(gtext("descrição de teste wamux"))
                .await
            {
                Ok(_) => rec(results, "Group.SetGroupDescription", true, "ok".to_string()),
                Err(e) => rec(
                    results,
                    "Group.SetGroupDescription",
                    false,
                    e.message().to_string(),
                ),
            }
            match groups.get_invite_link(gref.clone()).await {
                Ok(r) => rec(results, "Group.GetInviteLink", true, r.into_inner().link),
                Err(e) => rec(
                    results,
                    "Group.GetInviteLink",
                    false,
                    e.message().to_string(),
                ),
            }
            match groups.revoke_invite_link(gref.clone()).await {
                Ok(_) => rec(results, "Group.RevokeInviteLink", true, "ok".to_string()),
                Err(e) => rec(
                    results,
                    "Group.RevokeInviteLink",
                    false,
                    e.message().to_string(),
                ),
            }
            match groups.promote_admins(gpart.clone()).await {
                Ok(_) => rec(results, "Group.PromoteAdmins", true, "ok".to_string()),
                Err(e) => rec(
                    results,
                    "Group.PromoteAdmins",
                    false,
                    e.message().to_string(),
                ),
            }
            match groups.demote_admins(gpart.clone()).await {
                Ok(_) => rec(results, "Group.DemoteAdmins", true, "ok".to_string()),
                Err(e) => rec(
                    results,
                    "Group.DemoteAdmins",
                    false,
                    e.message().to_string(),
                ),
            }
            match groups.remove_participants(gpart).await {
                Ok(_) => rec(results, "Group.RemoveParticipants", true, "ok".to_string()),
                Err(e) => rec(
                    results,
                    "Group.RemoveParticipants",
                    false,
                    e.message().to_string(),
                ),
            }
            match groups.leave_group(gref).await {
                Ok(_) => rec(
                    results,
                    "Group.LeaveGroup",
                    true,
                    "left test group".to_string(),
                ),
                Err(e) => rec(results, "Group.LeaveGroup", false, e.message().to_string()),
            }
        }
        Err(e) => rec(results, "Group.CreateGroup", false, e.message().to_string()),
    }

    // ---- Lifecycle LAST: logout + delete the paired account ----
    match account.logout(acct.clone()).await {
        Ok(_) => rec(
            results,
            "Account.Logout",
            true,
            "ok (note: disconnect; device-side unlink TODO)".to_string(),
        ),
        Err(e) => rec(results, "Account.Logout", false, e.message().to_string()),
    }
    match account.delete_account(acct.clone()).await {
        Ok(_) => rec(
            results,
            "Account.DeleteAccount(paired)",
            true,
            "purged".to_string(),
        ),
        Err(e) => rec(
            results,
            "Account.DeleteAccount(paired)",
            false,
            e.message().to_string(),
        ),
    }
}

fn fetch_url_bytes(url: &str) -> Option<Vec<u8>> {
    let resp = ureq::get(url).call().ok()?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(8 * 1024 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

fn print_summary(results: &Results) {
    let r = results.lock().unwrap();
    let pass = r.iter().filter(|x| x.1).count();
    println!("\n================ E2E SUMMARY ================");
    for (name, ok, detail) in r.iter() {
        println!("  {} {name}  ::  {detail}", if *ok { "✅" } else { "❌" });
    }
    println!("  ----  {pass}/{} passed  ----", r.len());
}

async fn send_media(
    messaging: &mut MessagingServiceClient<Channel>,
    account: &pb::AccountRef,
    target: &str,
    data: Vec<u8>,
) -> Result<String, String> {
    let header = pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::Header(pb::SendMediaHeader {
            account: Some(account.clone()),
            to: Some(pb::Jid {
                value: target.to_string(),
            }),
            mime_type: "image/png".to_string(),
            caption: "wamux e2e_all: imagem ✅".to_string(),
            mentions: vec![],
            quote: None,
            media_type: "image".to_string(),
            filename: "e2e.png".to_string(),
        })),
    };
    let mut chunks = vec![header];
    for c in data.chunks(64 * 1024) {
        chunks.push(pb::SendMediaChunk {
            part: Some(pb::send_media_chunk::Part::Chunk(c.to_vec())),
        });
    }
    let r = messaging
        .send_media(tokio_stream::iter(chunks))
        .await
        .map_err(|e| e.message().to_string())?;
    Ok(r.into_inner().key.map(|k| k.id).unwrap_or_default())
}

fn generate_png() -> anyhow::Result<Vec<u8>> {
    let mut img = image::RgbImage::new(500, 250);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgb([((x / 2) % 256) as u8, (y % 256) as u8, 200]);
    }
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)?;
    Ok(buf)
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

async fn connect_uds(path: String) -> anyhow::Result<Channel> {
    let channel = Endpoint::try_from("http://[::1]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                Ok::<_, std::io::Error>(TokioIo::new(tokio::net::UnixStream::connect(path).await?))
            }
        }))
        .await?;
    Ok(channel)
}
