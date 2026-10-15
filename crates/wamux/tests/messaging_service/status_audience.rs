//! GetStatusAudience (#89): the audience the library holds, read back through
//! the socket. The device is seeded before login with what a sync from the
//! phone would have stored (`Device.status_privacy`), because the mock has no
//! phone to change the audience on.

use std::sync::Arc;

use wamux::proto::v1 as pb;
use whatsapp_rust::buffa::EnumValue;
use whatsapp_rust::waproto::whatsapp::sync_action_value::StatusPrivacyAction;
use whatsapp_rust::waproto::whatsapp::sync_action_value::status_privacy_action::{
    CustomList, StatusDistributionMode,
};

use crate::harness::{Fixture, fixture, fixture_shaped};

const EXCLUDED_A: &str = "5511900000001@s.whatsapp.net";
const EXCLUDED_B: &str = "120363000000000089@lid";
const LISTED: &str = "5511900000002@s.whatsapp.net";

async fn audience_of(f: &mut Fixture) -> Option<pb::StatusPrivacySetting> {
    f.messages
        .get_status_audience(f.account.clone())
        .await
        .expect("GetStatusAudience")
        .into_inner()
        .audience
}

fn audience(mode: pb::StatusAudienceMode, code: i32) -> pb::StatusAudience {
    pb::StatusAudience {
        mode: mode as i32,
        mode_code: code,
    }
}

fn jid(value: &str) -> pb::Jid {
    pb::Jid {
        value: value.to_string(),
    }
}

/// "My contacts except..." as measured live on 2026-10-08 (#149), plus a
/// custom list, so every field of the setting has something to carry.
fn deny_list() -> StatusPrivacyAction {
    StatusPrivacyAction {
        mode: Some(StatusDistributionMode::DENY_LIST.into()),
        modes: vec![StatusDistributionMode::DENY_LIST.into()],
        user_jid: vec![EXCLUDED_A.into(), EXCLUDED_B.into()],
        share_to_fb: Some(false),
        custom_lists: vec![CustomList {
            list_id: Some("work".into()),
            name: Some("Trabalho".into()),
            is_selected: Some(true),
            user_jid: vec![LISTED.into()],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_audience_is_unset_when_none_was_synced() {
    let mut f = fixture("status_audience_is_unset_when_none_was_synced").await;
    assert_eq!(
        audience_of(&mut f).await,
        None,
        "no synced audience must not cross as a default one"
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_audience_relays_the_synced_deny_list() {
    let mut f = fixture_shaped("status_audience_relays_the_synced_deny_list", |device| {
        device.status_privacy = Some(Arc::new(deny_list()));
    })
    .await;
    let expected = pb::StatusPrivacySetting {
        mode: Some(audience(pb::StatusAudienceMode::DenyList, 0)),
        users: vec![jid(EXCLUDED_A), jid(EXCLUDED_B)],
        share_to_fb: Some(false),
        share_to_ig: None,
        custom_lists: vec![pb::StatusCustomList {
            list_id: Some("work".into()),
            name: Some("Trabalho".into()),
            emoji: None,
            is_selected: Some(true),
            users: vec![jid(LISTED)],
        }],
        modes: vec![audience(pb::StatusAudienceMode::DenyList, 0)],
    };
    assert_eq!(audience_of(&mut f).await, Some(expected));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_audience_keeps_an_unknown_mode_as_its_number() {
    let mut f = fixture_shaped(
        "status_audience_keeps_an_unknown_mode_as_its_number",
        |device| {
            device.status_privacy = Some(Arc::new(StatusPrivacyAction {
                mode: Some(EnumValue::Unknown(99)),
                modes: vec![EnumValue::Unknown(99)],
                ..Default::default()
            }));
        },
    )
    .await;
    let got = audience_of(&mut f).await.expect("a synced audience");
    let unknown = audience(pb::StatusAudienceMode::Unknown, 99);
    assert_eq!(
        got.mode,
        Some(unknown),
        "an unknown mode is never a known one"
    );
    assert_eq!(got.modes, vec![unknown]);
    assert!(got.users.is_empty(), "{got:?}");
    f.cleanup().await;
}
