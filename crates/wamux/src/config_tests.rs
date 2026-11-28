//! #164: `store_key_file` is a PATH, from the TOML or the environment, and unset
//! by default (a plaintext store).

use figment::Figment;
use figment::providers::{Format, Serialized, Toml};

use super::{
    Config, PLATFORM, TERMUX_PREFIX, default_database_url, default_media_spool_dir,
    default_socket_path, require_platform_settings,
};

#[test]
fn store_key_file_is_unset_by_default() {
    assert_eq!(Config::default().store_key_file, None);
}

#[test]
fn store_key_file_is_a_path_from_toml() {
    let config: Config = Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string(
            "store_key_file = \"/run/credentials/wamux.service/store-key\"",
        ))
        .extract()
        .unwrap();
    assert_eq!(
        config.store_key_file.as_deref(),
        Some("/run/credentials/wamux.service/store-key")
    );
}

/// #77: the `SO_PEERCRED` allowlist is off unless configured.
#[test]
fn allowed_uids_and_gids_default_empty() {
    let config = Config::default();
    assert!(config.allowed_uids.is_empty());
    assert!(config.allowed_gids.is_empty());
}

#[test]
fn allowed_uids_come_from_toml() {
    let config: Config = Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string(
            "allowed_uids = [1000, 1001]\nallowed_gids = [967]",
        ))
        .extract()
        .unwrap();
    assert_eq!(config.allowed_uids, [1000, 1001]);
    assert_eq!(config.allowed_gids, [967]);
}

/// #79: the media spool lives in the system cache directory by default, with a
/// 2 GiB budget for every transfer in flight.
#[test]
fn media_spool_defaults() {
    let config = Config::default();
    // A Termux build keeps its spool under the Termux prefix (#277).
    if !cfg!(feature = "termux") {
        assert_eq!(config.media_spool_dir, "/var/cache/wamux");
    }
    assert_eq!(config.media_spool_max_bytes, 2 * 1024 * 1024 * 1024);
}

#[test]
fn media_spool_keys_come_from_toml() {
    let config: Config = Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string(
            "media_spool_dir = \"/srv/wamux/spool\"\nmedia_spool_max_bytes = 0",
        ))
        .extract()
        .unwrap();
    assert_eq!(config.media_spool_dir, "/srv/wamux/spool");
    assert_eq!(config.media_spool_max_bytes, 0);
}

/// #80: the replay ring caps an entry at 256 KiB by default, and the registry's
/// own default agrees, so a bin or test that builds a registry without a config
/// gets the same cap the daemon does.
#[test]
fn replay_max_event_bytes_defaults_to_256_kib() {
    assert_eq!(Config::default().replay_max_event_bytes, 256 * 1024);
    assert_eq!(
        crate::state::RegistryTuning::default().replay_max_event_bytes,
        Config::default().replay_max_event_bytes
    );
}

/// #231: `/run` cannot be created on macOS (the system volume is read-only), so
/// there the default is `/var/run`, the same root-only runtime directory that
/// is cleared at boot.
#[test]
fn socket_path_default_on_macos_is_var_run() {
    assert_eq!(default_socket_path("macos"), Some("/var/run/wamux.sock"));
}

#[test]
fn socket_path_default_elsewhere_is_run() {
    assert_eq!(default_socket_path("linux"), Some("/run/wamux.sock"));
    assert_eq!(default_socket_path("freebsd"), Some("/run/wamux.sock"));
}

#[test]
fn config_default_socket_path_follows_the_running_os() {
    assert_eq!(
        Some(Config::default().socket_path.as_str()),
        default_socket_path(PLATFORM)
    );
}

/// #277: Android has no writable `/run`, `/var/cache` or Postgres. A Termux
/// build (the `termux` feature) puts everything under Termux's fixed prefix, so
/// the daemon starts there with no `WAMUX_*` variable at all. Plain Android has
/// no default: an app that embeds the daemon owns a different directory, and a
/// guessed Termux path would be wrong there, so the config must name each one.
#[test]
fn termux_prefix_is_the_fixed_termux_install_prefix() {
    assert_eq!(TERMUX_PREFIX, "/data/data/com.termux/files/usr");
}

#[test]
fn platform_is_the_running_os_without_the_termux_feature() {
    if cfg!(feature = "termux") {
        assert_eq!(PLATFORM, "termux");
    } else {
        assert_eq!(PLATFORM, std::env::consts::OS);
    }
}

#[test]
fn termux_defaults_live_under_the_termux_prefix() {
    assert_eq!(
        default_socket_path("termux"),
        Some("/data/data/com.termux/files/usr/var/run/wamux.sock")
    );
    assert_eq!(
        default_media_spool_dir("termux"),
        Some("/data/data/com.termux/files/usr/var/cache/wamux")
    );
    assert_eq!(
        default_database_url("termux"),
        Some("sqlite:///data/data/com.termux/files/usr/var/lib/wamux/wamux.db")
    );
}

#[test]
fn plain_android_has_no_path_defaults() {
    assert_eq!(default_socket_path("android"), None);
    assert_eq!(default_media_spool_dir("android"), None);
    assert_eq!(default_database_url("android"), None);
}

#[test]
fn default_media_spool_dir_elsewhere_is_var_cache() {
    for os in ["linux", "macos", "freebsd"] {
        assert_eq!(
            default_media_spool_dir(os),
            Some("/var/cache/wamux"),
            "{os}"
        );
    }
}

#[test]
fn default_database_url_elsewhere_is_postgres() {
    for os in ["linux", "macos", "freebsd"] {
        assert_eq!(
            default_database_url(os),
            Some("postgres://wamux:wamux@localhost:5432/wamux"),
            "{os}"
        );
    }
}

#[test]
fn config_defaults_follow_the_platform() {
    let config = Config::default();
    assert_eq!(
        Some(config.media_spool_dir.as_str()),
        default_media_spool_dir(PLATFORM)
    );
    assert_eq!(
        Some(config.database_url.as_str()),
        default_database_url(PLATFORM)
    );
}

#[test]
fn require_platform_settings_accepts_a_config_with_every_path() {
    assert!(require_platform_settings(&Config::default()).is_ok());
}

#[test]
fn require_platform_settings_names_every_empty_setting_in_order() {
    let config = Config {
        socket_path: String::new(),
        media_spool_dir: String::new(),
        database_url: String::new(),
        ..Config::default()
    };
    let error = require_platform_settings(&config).expect_err("nothing is set");
    assert_eq!(
        error.variables,
        [
            "WAMUX_SOCKET_PATH",
            "WAMUX_MEDIA_SPOOL_DIR",
            "WAMUX_DATABASE_URL"
        ]
    );
    let message = error.to_string();
    for variable in &error.variables {
        assert!(message.contains(variable), "{message}");
    }
}

#[test]
fn require_platform_settings_names_only_what_is_empty() {
    let config = Config {
        media_spool_dir: String::new(),
        ..Config::default()
    };
    let error = require_platform_settings(&config).expect_err("the spool is unset");
    assert_eq!(error.variables, ["WAMUX_MEDIA_SPOOL_DIR"]);
}

/// #239: after the accounts stop, the runtime waits at most this long for its
/// blocking tasks. 5 s keeps the worst exit (10 s grace + 3 s stop + 5 s) far
/// under systemd's 90 s stop timeout.
#[test]
fn blocking_shutdown_timeout_defaults_to_5s() {
    assert_eq!(Config::default().blocking_shutdown_timeout_ms, 5000);
}

#[test]
fn blocking_shutdown_timeout_comes_from_toml() {
    let config: Config = Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string("blocking_shutdown_timeout_ms = 1500"))
        .extract()
        .unwrap();
    assert_eq!(config.blocking_shutdown_timeout_ms, 1500);
}
