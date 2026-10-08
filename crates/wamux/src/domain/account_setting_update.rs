//! Account-wide settings changed on a linked device (#149, part 2 of #141):
//! link previews and the status audience. They reached the socket as RawEvent
//! with the library's JSON.

use wacore::types::events::{DisableLinkPreviewsUpdate, StatusPrivacyUpdate};
use wamux_types::relay_jid;
use whatsapp_rust::buffa::EnumValue;
use whatsapp_rust::waproto::whatsapp::sync_action_value::StatusPrivacyAction;
use whatsapp_rust::waproto::whatsapp::sync_action_value::status_privacy_action::{
    CustomList, StatusDistributionMode,
};

use crate::proto::v1 as pb;
use crate::proto::v1::account_setting_update::Setting;

/// The action's own flag, not the library's derived `previews_disabled`.
pub fn link_previews_update_of(update: &DisableLinkPreviewsUpdate) -> pb::AccountSettingUpdate {
    pb::AccountSettingUpdate {
        timestamp: update.timestamp.timestamp_millis(),
        // The library struct keeps no time of the mutation's own here.
        action_timestamp: None,
        from_full_sync: update.from_full_sync,
        setting: Some(Setting::LinkPreviews(pb::LinkPreviewsSetting {
            previews_disabled: update.action.is_previews_disabled,
        })),
    }
}

/// The audience as sent: each mode as the contract's enum, plus the server's
/// number only when it is UNKNOWN (#73; the library keeps this enum open), and
/// the listed users as jids.
pub fn status_privacy_update_of(update: &StatusPrivacyUpdate) -> pb::AccountSettingUpdate {
    pb::AccountSettingUpdate {
        timestamp: update.timestamp.timestamp_millis(),
        action_timestamp: update.action_timestamp.map(|at| at.timestamp_millis()),
        from_full_sync: update.from_full_sync,
        setting: Some(Setting::StatusPrivacy(status_privacy_of(&update.action))),
    }
}

fn status_privacy_of(action: &StatusPrivacyAction) -> pb::StatusPrivacySetting {
    pb::StatusPrivacySetting {
        mode: action.mode.map(status_audience_of),
        users: jids_of(&action.user_jid),
        share_to_fb: action.share_to_fb,
        share_to_ig: action.share_to_ig,
        custom_lists: action.custom_lists.iter().map(custom_list_of).collect(),
        modes: action
            .modes
            .iter()
            .copied()
            .map(status_audience_of)
            .collect(),
    }
}

fn custom_list_of(list: &CustomList) -> pb::StatusCustomList {
    pb::StatusCustomList {
        list_id: list.list_id.clone(),
        name: list.name.clone(),
        emoji: list.emoji.clone(),
        is_selected: list.is_selected,
        users: jids_of(&list.user_jid),
    }
}

/// Through `relay_jid`, so an empty string is dropped instead of relayed as a
/// jid with no value (#120).
fn jids_of(values: &[String]) -> Vec<pb::Jid> {
    values.iter().filter_map(relay_jid).collect()
}

/// The library keeps this enum open (#73): a number it does not know arrives
/// as `Unknown(n)` and crosses as UNKNOWN plus `n`, never as a guess.
fn status_audience_of(mode: EnumValue<StatusDistributionMode>) -> pb::StatusAudience {
    match mode {
        EnumValue::Known(known) => pb::StatusAudience {
            mode: status_audience_mode_of(known) as i32,
            mode_code: 0,
        },
        EnumValue::Unknown(code) => pb::StatusAudience {
            mode: pb::StatusAudienceMode::Unknown as i32,
            mode_code: code,
        },
    }
}

/// One arm per variant, no wildcard: a mode added upstream fails the build.
fn status_audience_mode_of(mode: StatusDistributionMode) -> pb::StatusAudienceMode {
    use pb::StatusAudienceMode as Kind;
    match mode {
        StatusDistributionMode::ALLOW_LIST => Kind::AllowList,
        StatusDistributionMode::DENY_LIST => Kind::DenyList,
        StatusDistributionMode::CONTACTS => Kind::Contacts,
        StatusDistributionMode::CLOSE_FRIENDS => Kind::CloseFriends,
        StatusDistributionMode::CUSTOM_LIST => Kind::CustomList,
    }
}

#[cfg(test)]
#[path = "account_setting_update_tests.rs"]
mod tests;
