//! The `turso://<path>` DSN (#106), shaped like `sqlite://<path>`.

use wacore::store::error::StoreError;

use super::turso_path;

#[test]
fn turso_path_reads_absolute_and_relative() {
    assert_eq!(
        turso_path("turso:///var/lib/wamux/wamux.db").unwrap(),
        "/var/lib/wamux/wamux.db"
    );
    assert_eq!(turso_path("turso://wamux.db").unwrap(), "wamux.db");
    assert_eq!(
        turso_path("turso://data/wamux.db").unwrap(),
        "data/wamux.db"
    );
}

#[test]
fn turso_path_refuses_query_string_and_other_shapes() {
    for dsn in [
        "turso:///var/lib/wamux/wamux.db?mode=rwc",
        "turso:wamux.db",
        "turso://",
        "turso:///",
        "sqlite:///var/lib/wamux/wamux.db",
    ] {
        match turso_path(dsn) {
            Err(StoreError::InvalidConfig(msg)) => assert!(
                msg.contains(dsn),
                "the error for {dsn:?} must quote the DSN, got {msg:?}"
            ),
            other => panic!("{dsn:?} must be InvalidConfig, got {other:?}"),
        }
    }
}
