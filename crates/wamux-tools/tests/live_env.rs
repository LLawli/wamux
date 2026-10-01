//! #64: the env contract every wamux-tools binary reads. Pure logic, driven
//! through an injected lookup (tests cannot set process variables).

use std::collections::HashMap;
use std::time::Duration;

use wamux_tools::live_env::{
    self, LiveEnvError, account_ref_from, delivery_window_from, live_dest_from, refuse_own_number,
    required_from, same_user, socket_path_from, user_of,
};

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |var: &str| map.get(var).filter(|v| !v.is_empty()).cloned()
}

#[test]
fn socket_path_defaults_to_the_production_unit_path_under_home() {
    let lookup = env(&[("HOME", "/home/someone")]);
    assert_eq!(
        socket_path_from(&lookup).unwrap(),
        "/home/someone/.local/state/wamux/wamux.sock"
    );
}

#[test]
fn socket_path_override_wins_over_home() {
    let lookup = env(&[
        ("HOME", "/home/someone"),
        ("WAMUX_SOCKET_PATH", "/run/w.sock"),
    ]);
    assert_eq!(socket_path_from(&lookup).unwrap(), "/run/w.sock");
}

#[test]
fn socket_path_without_home_or_override_is_an_error_naming_the_var() {
    let err = socket_path_from(&env(&[])).unwrap_err();
    assert!(
        err.to_string().contains("WAMUX_SOCKET_PATH"),
        "error must name the variable to set: {err}"
    );
}

#[test]
fn account_ref_is_required_and_has_no_default() {
    let err = account_ref_from(&env(&[])).unwrap_err();
    assert!(matches!(
        err,
        LiveEnvError::Missing {
            var: "WAMUX_REF",
            ..
        }
    ));
    assert!(err.to_string().contains("WAMUX_REF"));
}

#[test]
fn an_empty_account_ref_counts_as_missing() {
    let err = account_ref_from(&env(&[("WAMUX_REF", "")])).unwrap_err();
    assert!(matches!(
        err,
        LiveEnvError::Missing {
            var: "WAMUX_REF",
            ..
        }
    ));
}

#[test]
fn account_ref_is_read_verbatim() {
    let lookup = env(&[("WAMUX_REF", "pessoal")]);
    assert_eq!(account_ref_from(&lookup).unwrap(), "pessoal");
}

#[test]
fn required_from_names_the_var_and_carries_the_hint() {
    let err = required_from(&env(&[]), "WAMUX_PEER_REF", "the watching account").unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("WAMUX_PEER_REF") && text.contains("the watching account"),
        "{text}"
    );
}

#[test]
fn live_dest_is_required() {
    let err = live_dest_from(&env(&[])).unwrap_err();
    assert!(matches!(
        err,
        LiveEnvError::Missing {
            var: "WAMUX_LIVE_DEST",
            ..
        }
    ));
}

#[test]
fn live_dest_without_a_server_is_not_a_jid() {
    let err = live_dest_from(&env(&[("WAMUX_LIVE_DEST", "5561900000000")])).unwrap_err();
    assert!(matches!(err, LiveEnvError::NotAJid { .. }), "{err:?}");
}

#[test]
fn live_dest_with_an_empty_user_is_not_a_jid() {
    let err = live_dest_from(&env(&[("WAMUX_LIVE_DEST", "@s.whatsapp.net")])).unwrap_err();
    assert!(matches!(err, LiveEnvError::NotAJid { .. }), "{err:?}");
}

#[test]
fn live_dest_on_the_legacy_c_us_server_is_refused() {
    let err = live_dest_from(&env(&[("WAMUX_LIVE_DEST", "5561900000000@c.us")])).unwrap_err();
    assert!(matches!(err, LiveEnvError::LegacyServer { .. }), "{err:?}");
    assert!(
        err.to_string().contains("@s.whatsapp.net"),
        "must say the right spelling"
    );
}

#[test]
fn live_dest_accepts_the_modern_servers() {
    for jid in [
        "5561900000000@s.whatsapp.net",
        "123456789012345@lid",
        "120363000000000000@g.us",
    ] {
        let lookup = env(&[("WAMUX_LIVE_DEST", jid)]);
        assert_eq!(live_dest_from(&lookup).unwrap(), jid);
    }
}

#[test]
fn delivery_window_defaults_to_thirty_seconds() {
    assert_eq!(
        delivery_window_from(&env(&[])).unwrap(),
        Duration::from_secs(30)
    );
    assert_eq!(live_env::DEFAULT_DELIVERY_SECS, 30);
}

#[test]
fn delivery_window_reads_whole_seconds() {
    let lookup = env(&[("WAMUX_DELIVERY_SECS", "45")]);
    assert_eq!(
        delivery_window_from(&lookup).unwrap(),
        Duration::from_secs(45)
    );
}

#[test]
fn delivery_window_rejects_a_non_number() {
    let err = delivery_window_from(&env(&[("WAMUX_DELIVERY_SECS", "soon")])).unwrap_err();
    assert!(matches!(err, LiveEnvError::NotSeconds { .. }), "{err:?}");
}

#[test]
fn user_of_strips_server_and_device_suffix() {
    assert_eq!(user_of("5561900000000:16@s.whatsapp.net"), "5561900000000");
    assert_eq!(user_of("5561900000000@s.whatsapp.net"), "5561900000000");
    assert_eq!(user_of("123456789012345@lid"), "123456789012345");
}

#[test]
fn same_user_ignores_device_and_server() {
    assert!(same_user(
        "5561900000000:16@s.whatsapp.net",
        "5561900000000@s.whatsapp.net"
    ));
    assert!(!same_user(
        "5561900000000@s.whatsapp.net",
        "5561900000001@s.whatsapp.net"
    ));
}

#[test]
fn refuse_own_number_rejects_the_connected_account_itself() {
    let err = refuse_own_number(
        "5561900000000:7@s.whatsapp.net",
        "5561900000000@s.whatsapp.net",
    )
    .unwrap_err();
    assert!(matches!(err, LiveEnvError::OwnNumber { .. }), "{err:?}");
    assert!(err.to_string().contains("WAMUX_LIVE_DEST"));
}

#[test]
fn refuse_own_number_lets_another_chat_through() {
    refuse_own_number(
        "5561900000000:7@s.whatsapp.net",
        "5561900000001@s.whatsapp.net",
    )
    .unwrap();
}

#[test]
fn process_env_reads_an_empty_value_as_unset() {
    // PATH is set in any test run; a variable nobody sets is not.
    assert!(live_env::process_env("PATH").is_some());
    assert!(live_env::process_env("WAMUX_TOOLS_TEST_SURELY_UNSET_64").is_none());
}
