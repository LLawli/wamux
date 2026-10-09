//! What the server says about a contact (#151, part 4 of #141): a picture,
//! an about text, a profile, a phone number, a re-sync request, a
//! disappearing-messages timer and a business status. Each reached the socket
//! as RawEvent with the library's JSON.

use wacore::stanza::business::BusinessSubscription;
use wacore::types::events::{
    BusinessStatusUpdate, BusinessUpdateType, ContactNumberChanged, ContactSyncRequested,
    ContactUpdated, DisappearingModeChanged, PictureUpdate, UserAboutUpdate,
};
use wamux_types::{relay_lib_jid, relay_optional_lib_jid};

use crate::domain::wire_time::millis_from_signed_seconds;
use crate::proto::v1 as pb;

fn notice(notice: pb::contact_notice::Notice) -> pb::ContactNotice {
    pb::ContactNotice {
        notice: Some(notice),
    }
}

pub fn picture_notice_of(update: &PictureUpdate) -> pb::ContactNotice {
    notice(pb::contact_notice::Notice::Picture(pb::PictureChange {
        jid: relay_lib_jid(&update.jid),
        author: relay_optional_lib_jid(update.author.as_ref()),
        timestamp: update.timestamp.timestamp_millis(),
        removed: update.removed,
        picture_id: update.picture_id.clone(),
    }))
}

pub fn about_notice_of(update: &UserAboutUpdate) -> pb::ContactNotice {
    notice(pb::contact_notice::Notice::About(pb::AboutChange {
        jid: relay_lib_jid(&update.jid),
        about: update.status.clone(),
        timestamp: update.timestamp.timestamp_millis(),
    }))
}

pub fn profile_notice_of(update: &ContactUpdated) -> pb::ContactNotice {
    notice(pb::contact_notice::Notice::Profile(
        pb::ContactProfileChange {
            jid: relay_lib_jid(&update.jid),
            timestamp: update.timestamp.timestamp_millis(),
        },
    ))
}

pub fn number_notice_of(change: &ContactNumberChanged) -> pb::ContactNotice {
    notice(pb::contact_notice::Notice::Number(pb::NumberChange {
        old_pn: relay_lib_jid(&change.old_jid),
        new_pn: relay_lib_jid(&change.new_jid),
        old_lid: relay_optional_lib_jid(change.old_lid.as_ref()),
        new_lid: relay_optional_lib_jid(change.new_lid.as_ref()),
        timestamp: change.timestamp.timestamp_millis(),
    }))
}

pub fn sync_requested_notice_of(request: &ContactSyncRequested) -> pb::ContactNotice {
    notice(pb::contact_notice::Notice::SyncRequested(
        pb::ContactSyncRequest {
            after: request.after.map(|after| after.timestamp_millis()),
            timestamp: request.timestamp.timestamp_millis(),
        },
    ))
}

pub fn disappearing_mode_notice_of(change: &DisappearingModeChanged) -> pb::ContactNotice {
    notice(pb::contact_notice::Notice::DisappearingMode(
        pb::DisappearingModeChange {
            jid: relay_lib_jid(&change.from),
            duration_seconds: change.duration,
            setting_timestamp: change.setting_timestamp.timestamp_millis(),
        },
    ))
}

pub fn business_notice_of(update: &BusinessStatusUpdate) -> pb::ContactNotice {
    notice(pb::contact_notice::Notice::Business(
        pb::BusinessStatusChange {
            jid: relay_lib_jid(&update.jid),
            update_type: business_update_type_of(update.update_type) as i32,
            timestamp: update.timestamp.timestamp_millis(),
            target: relay_optional_lib_jid(update.target_jid.as_ref()),
            hash: update.hash.clone(),
            verified_name: update.verified_name.clone(),
            product_ids: update.product_ids.clone(),
            collection_ids: update.collection_ids.clone(),
            subscriptions: update.subscriptions.iter().map(subscription_of).collect(),
        },
    ))
}

fn business_update_type_of(kind: BusinessUpdateType) -> pb::BusinessUpdateType {
    match kind {
        BusinessUpdateType::RemovedAsBusiness => pb::BusinessUpdateType::RemovedAsBusiness,
        BusinessUpdateType::VerifiedNameChanged => pb::BusinessUpdateType::VerifiedNameChanged,
        BusinessUpdateType::ProfileUpdated => pb::BusinessUpdateType::ProfileUpdated,
        BusinessUpdateType::ProductsUpdated => pb::BusinessUpdateType::ProductsUpdated,
        BusinessUpdateType::CollectionsUpdated => pb::BusinessUpdateType::CollectionsUpdated,
        BusinessUpdateType::SubscriptionsUpdated => pb::BusinessUpdateType::SubscriptionsUpdated,
        BusinessUpdateType::Unknown => pb::BusinessUpdateType::Unknown,
    }
}

/// The stanza sends both instants in seconds; the contract's are ms (#151).
fn subscription_of(subscription: &BusinessSubscription) -> pb::BusinessSubscriptionInfo {
    pb::BusinessSubscriptionInfo {
        id: subscription.id.clone(),
        status: subscription.status.clone(),
        expiration_date: subscription.expiration_date.map(millis_from_signed_seconds),
        creation_time: subscription.creation_time.map(millis_from_signed_seconds),
    }
}

#[cfg(test)]
#[path = "contact_notice_tests.rs"]
mod tests;
