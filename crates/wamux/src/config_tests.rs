//! #164: `store_key_file` is a PATH, from the TOML or the environment, and unset
//! by default (a plaintext store, as before).

use figment::Figment;
use figment::providers::{Format, Serialized, Toml};

use super::Config;

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
    assert_eq!(config.media_spool_dir, "/var/cache/wamux");
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
