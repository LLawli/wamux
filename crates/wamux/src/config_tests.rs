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
