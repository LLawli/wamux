//! #149 (part 2 of #141): account settings, routed through `map_event`, so
//! each test also proves the event no longer falls into the RawEvent
//! catch-all. The link-preview values are the ones the user's phone sent on
//! 2026-10-08 18:49Z (read off the production subscription). The status
//! audience is hand-built: changing it on the phone reached no app-state
//! mutation.

#![allow(clippy::field_reassign_with_default)] // the generated actions are built by assignment

use wacore::types::events::{DisableLinkPreviewsUpdate, Event, StatusPrivacyUpdate};
use whatsapp_rust::buffa::EnumValue;
use whatsapp_rust::waproto::whatsapp::sync_action_value::status_privacy_action::{
    CustomList, StatusDistributionMode,
};
use whatsapp_rust::waproto::whatsapp::sync_action_value::{
    PrivacySettingDisableLinkPreviewsAction, StatusPrivacyAction,
};

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::account_setting_update::Setting;
use crate::proto::v1::event_envelope::Event as PbEvent;

const FRIEND: &str = "5511900000149@s.whatsapp.net";
const OTHER: &str = "100000000000149@lid";

/// The instant `ms` after the epoch, as the library's builders take it. A
/// macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at_ms {
    ($ms:expr) => {
        wacore::time::from_millis($ms).expect("test instant")
    };
}

/// A known mode as the contract carries it: the case, no number.
fn audience(mode: pb::StatusAudienceMode) -> pb::StatusAudience {
    pb::StatusAudience {
        mode: mode as i32,
        mode_code: 0,
    }
}

fn wire(value: &str) -> pb::Jid {
    pb::Jid {
        value: value.to_string(),
    }
}

/// The one AccountSettingUpdate the event maps to; anything else is a failure.
fn setting_of(event: Event) -> pb::AccountSettingUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::AccountSetting(update)] => update.clone(),
        other => panic!("expected one AccountSettingUpdate, got {other:?}"),
    }
}

/// The capture: previews disabled, then enabled again. The setting carries
/// no time of its own, so `action_timestamp` stays unset.
#[test]
fn link_previews_relays_the_action_flag() {
    for (disabled, ms) in [(true, 1_791_485_343_345), (false, 1_791_485_347_071)] {
        let mut action = PrivacySettingDisableLinkPreviewsAction::default();
        action.is_previews_disabled = Some(disabled);
        let event = Event::DisableLinkPreviewsUpdate(
            DisableLinkPreviewsUpdate::builder()
                .previews_disabled(disabled)
                .timestamp(at_ms!(ms))
                .action(Box::new(action))
                .from_full_sync(false)
                .build(),
        );
        let expected = pb::AccountSettingUpdate {
            timestamp: ms,
            action_timestamp: None,
            from_full_sync: false,
            setting: Some(Setting::LinkPreviews(pb::LinkPreviewsSetting {
                previews_disabled: Some(disabled),
            })),
        };
        assert_eq!(setting_of(event), expected);
    }
}

fn status_event(action: StatusPrivacyAction) -> Event {
    Event::StatusPrivacyUpdate(
        StatusPrivacyUpdate::builder()
            .timestamp(at_ms!(1_791_485_400_000))
            .action_timestamp(at_ms!(1_791_485_400_000))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    )
}

#[test]
fn status_privacy_relays_mode_users_and_lists() {
    let mut list = CustomList::default();
    list.list_id = Some("L1".to_string());
    list.name = Some("Família".to_string());
    list.emoji = Some("🏠".to_string());
    list.is_selected = Some(true);
    list.user_jid = vec![OTHER.to_string()];
    let mut action = StatusPrivacyAction::default();
    action.mode = Some(EnumValue::Known(StatusDistributionMode::DENY_LIST));
    action.user_jid = vec![FRIEND.to_string()];
    action.share_to_fb = Some(false);
    action.share_to_ig = Some(true);
    action.custom_lists = vec![list];
    action.modes = vec![
        EnumValue::Known(StatusDistributionMode::DENY_LIST),
        EnumValue::Known(StatusDistributionMode::CUSTOM_LIST),
    ];
    let expected = pb::AccountSettingUpdate {
        timestamp: 1_791_485_400_000,
        action_timestamp: Some(1_791_485_400_000),
        from_full_sync: false,
        setting: Some(Setting::StatusPrivacy(pb::StatusPrivacySetting {
            mode: Some(audience(pb::StatusAudienceMode::DenyList)),
            users: vec![wire(FRIEND)],
            share_to_fb: Some(false),
            share_to_ig: Some(true),
            custom_lists: vec![pb::StatusCustomList {
                list_id: Some("L1".to_string()),
                name: Some("Família".to_string()),
                emoji: Some("🏠".to_string()),
                is_selected: Some(true),
                users: vec![wire(OTHER)],
            }],
            modes: vec![
                audience(pb::StatusAudienceMode::DenyList),
                audience(pb::StatusAudienceMode::CustomList),
            ],
        })),
    };
    assert_eq!(setting_of(status_event(action)), expected);
}

/// Each mode the library names crosses as its own case, with no number.
#[test]
fn status_privacy_maps_every_mode() {
    let cases = [
        (
            StatusDistributionMode::ALLOW_LIST,
            pb::StatusAudienceMode::AllowList,
        ),
        (
            StatusDistributionMode::DENY_LIST,
            pb::StatusAudienceMode::DenyList,
        ),
        (
            StatusDistributionMode::CONTACTS,
            pb::StatusAudienceMode::Contacts,
        ),
        (
            StatusDistributionMode::CLOSE_FRIENDS,
            pb::StatusAudienceMode::CloseFriends,
        ),
        (
            StatusDistributionMode::CUSTOM_LIST,
            pb::StatusAudienceMode::CustomList,
        ),
    ];
    for (lib_mode, wire_mode) in cases {
        let mut action = StatusPrivacyAction::default();
        action.mode = Some(EnumValue::Known(lib_mode));
        let update = setting_of(status_event(action));
        let Some(Setting::StatusPrivacy(setting)) = update.setting else {
            panic!("expected a status privacy, got {:?}", update.setting);
        };
        assert_eq!(setting.mode, Some(audience(wire_mode)), "for {lib_mode:?}");
    }
}

/// The library keeps this enum open (waproto/build.rs, `open_enums_in`): a
/// number it does not know arrives, and crosses as UNKNOWN with the number,
/// in `mode` and in every `modes` entry. It is not "all contacts".
#[test]
fn status_privacy_with_an_unknown_mode_relays_its_code() {
    let mut action = StatusPrivacyAction::default();
    action.mode = Some(EnumValue::Unknown(9));
    action.modes = vec![
        EnumValue::Known(StatusDistributionMode::CONTACTS),
        EnumValue::Unknown(12),
    ];
    let update = setting_of(status_event(action));
    let Some(Setting::StatusPrivacy(setting)) = update.setting else {
        panic!("expected a status privacy, got {:?}", update.setting);
    };
    let unknown = |code| pb::StatusAudience {
        mode: pb::StatusAudienceMode::Unknown as i32,
        mode_code: code,
    };
    assert_eq!(setting.mode, Some(unknown(9)));
    assert_eq!(
        setting.modes,
        vec![audience(pb::StatusAudienceMode::Contacts), unknown(12)]
    );
}

/// No mode on the mutation is an unset audience, never an UNSPECIFIED one.
#[test]
fn status_privacy_without_a_mode_leaves_it_unset() {
    let update = setting_of(status_event(StatusPrivacyAction::default()));
    let Some(Setting::StatusPrivacy(setting)) = update.setting else {
        panic!("expected a status privacy, got {:?}", update.setting);
    };
    assert_eq!(setting.mode, None);
    assert!(setting.users.is_empty() && setting.modes.is_empty());
}
