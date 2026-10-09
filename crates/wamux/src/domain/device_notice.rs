//! Someone's devices or identity key changed (#151, part 4 of #141). Both
//! reached the socket as RawEvent with the library's JSON.

use wacore::stanza::devices::KeyIndexInfo;
use wacore::types::events::{
    DeviceListUpdate, DeviceListUpdateType, DeviceNotificationInfo, IdentityChange,
};
use wamux_types::{relay_lib_jid, relay_optional_lib_jid};

use crate::domain::wire_time::millis_from_signed_seconds;
use crate::proto::v1 as pb;

pub fn device_list_notice_of(update: &DeviceListUpdate) -> pb::DeviceNotice {
    pb::DeviceNotice {
        notice: Some(pb::device_notice::Notice::DeviceList(
            pb::DeviceListChange {
                user: relay_lib_jid(&update.user),
                lid_user: relay_optional_lib_jid(update.lid_user.as_ref()),
                update_type: device_list_type_of(update.update_type) as i32,
                devices: update.devices.iter().map(device_key_index_of).collect(),
                key_index: update.key_index.as_ref().map(key_index_list_of),
                contact_hash: update.contact_hash.clone(),
            },
        )),
    }
}

pub fn identity_notice_of(change: &IdentityChange) -> pb::DeviceNotice {
    pb::DeviceNotice {
        notice: Some(pb::device_notice::Notice::Identity(
            pb::IdentityChangeInfo {
                user: relay_lib_jid(&change.user),
                lid_user: relay_optional_lib_jid(change.lid_user.as_ref()),
                implicit: change.implicit,
            },
        )),
    }
}

fn device_list_type_of(kind: DeviceListUpdateType) -> pb::DeviceListChangeType {
    match kind {
        DeviceListUpdateType::Add => pb::DeviceListChangeType::Add,
        DeviceListUpdateType::Remove => pb::DeviceListChangeType::Remove,
        DeviceListUpdateType::Update => pb::DeviceListChangeType::Update,
    }
}

fn device_key_index_of(device: &DeviceNotificationInfo) -> pb::DeviceKeyIndex {
    pb::DeviceKeyIndex {
        device_id: device.device_id,
        key_index: device.key_index,
    }
}

fn key_index_list_of(info: &KeyIndexInfo) -> pb::KeyIndexList {
    pb::KeyIndexList {
        // Measured in seconds on the wire; the contract's instants are ms (#151).
        timestamp: millis_from_signed_seconds(info.timestamp),
        signed_bytes: info.signed_bytes.clone(),
    }
}

#[cfg(test)]
#[path = "device_notice_tests.rs"]
mod tests;
