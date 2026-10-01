//! Read-only checks against the destination and the account's first group.
//! Calls that answer with a value no property of which can be asserted (an
//! about text, a photo URL) are `accepted`, not `pass`.

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::proto::v1::group_service_client::GroupServiceClient;
use wamux_tools::report::Report;

use crate::E2eCtx;

pub async fn run_contact_and_group_reads(channel: &Channel, report: &mut Report, ctx: &E2eCtx) {
    let mut contacts = ContactServiceClient::new(channel.clone());
    check_on_whatsapp(&mut contacts, report, ctx).await;
    let jid_req = pb::JidRequest {
        account: Some(ctx.acct.clone()),
        jid: ctx.dest.clone(),
    };
    // GetPushName answers the ACCOUNT's own name, not the target's (issue #1).
    let name = contacts.get_push_name(ctx.acct.clone()).await;
    report.accepted_rpc("Contact.GetPushName", name);
    let about = contacts.get_about(jid_req.clone()).await;
    report.accepted_rpc("Contact.GetAbout", about);
    let photo = contacts.get_profile_picture(jid_req.clone()).await;
    report.accepted_rpc("Contact.GetProfilePicture", photo);
    let business = contacts.get_business_profile(jid_req).await;
    report.accepted_rpc("Contact.GetBusinessProfile", business);
    let presence = contacts
        .subscribe_presence(pb::SubscribePresenceRequest {
            account: Some(ctx.acct.clone()),
            jid: ctx.dest.clone(),
        })
        .await;
    report.accepted_rpc("Contact.SubscribePresence", presence);
    let mut groups = GroupServiceClient::new(channel.clone());
    check_groups(&mut groups, report, ctx).await;
}

/// The destination must be a registered number: that is the one value here a
/// wrong WAMUX_LIVE_DEST would get wrong.
async fn check_on_whatsapp(
    contacts: &mut ContactServiceClient<Channel>,
    report: &mut Report,
    ctx: &E2eCtx,
) {
    let checked = contacts
        .check_on_whats_app(pb::CheckOnWhatsAppRequest {
            account: Some(ctx.acct.clone()),
            jids: vec![ctx.dest.clone()],
        })
        .await;
    match checked {
        Ok(r) => {
            let results = r.into_inner().results;
            let found = results.first();
            report.verify(
                "Contact.CheckOnWhatsApp",
                found.is_some_and(|x| x.is_on_whatsapp),
                found.map_or("no result".to_string(), |x| {
                    format!("on_wa={} jid={}", x.is_on_whatsapp, x.jid)
                }),
            );
        }
        Err(e) => report.fail("Contact.CheckOnWhatsApp", e.to_string()),
    }
}

/// ListParticipating, then the metadata of the first group when there is one.
async fn check_groups(groups: &mut GroupServiceClient<Channel>, report: &mut Report, ctx: &E2eCtx) {
    let Some(listed) = report.accepted_rpc(
        "Group.ListParticipating",
        groups.list_participating(ctx.acct.clone()).await,
    ) else {
        return;
    };
    let Some(first) = listed.into_inner().groups.into_iter().next() else {
        return;
    };
    let gref = pb::GroupRef {
        account: Some(ctx.acct.clone()),
        group_jid: first.jid.clone(),
    };
    match groups.get_group_metadata(gref).await {
        Ok(r) => {
            let meta = r.into_inner().metadata;
            report.verify(
                "Group.GetGroupMetadata",
                !meta.is_empty(),
                format!("{} meta bytes for {}", meta.len(), first.jid),
            );
        }
        Err(e) => report.fail("Group.GetGroupMetadata", e.to_string()),
    }
}
