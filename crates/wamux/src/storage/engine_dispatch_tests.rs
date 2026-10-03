//! `open_engine` and the `turso://` scheme (#106): opened when the daemon is
//! built with the `turso` feature, refused with a clear error otherwise.

use wacore::store::error::StoreError;

use super::open_engine;

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_scheme_opens_a_turso_engine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("dispatch.db");
    let engine = open_engine(&format!("turso://{}", path.display()), 5)
        .await
        .expect("turso:// opens with the feature on");
    assert!(engine.ping_storage().await, "a fresh turso engine is ready");
    let row = engine
        .create_account(Some("dispatch"))
        .await
        .expect("create an account");
    assert_eq!(engine.list_accounts().await.unwrap().len(), 1);
    assert!(engine.delete_account(row.uuid).await.unwrap());
    assert!(path.exists(), "the file is created where the DSN points");
}

#[cfg(not(feature = "turso"))]
#[tokio::test]
async fn turso_scheme_without_the_feature_is_invalid_config() {
    let result = open_engine("turso:///var/lib/wamux/wamux.db", 5).await;
    match result {
        Err(StoreError::InvalidConfig(msg)) => assert!(
            msg.contains("`turso` feature"),
            "the error must name the missing feature, got {msg:?}"
        ),
        Err(other) => panic!("expected InvalidConfig, got {other:?}"),
        Ok(_) => panic!("turso:// must not open without the feature"),
    }
}

#[tokio::test]
async fn unknown_scheme_lists_turso_among_the_expected() {
    match open_engine("mysql://localhost/wamux", 5).await {
        Err(StoreError::InvalidConfig(msg)) => assert!(
            msg.contains("turso://"),
            "the error must list turso:// as an option, got {msg:?}"
        ),
        Err(other) => panic!("expected InvalidConfig, got {other:?}"),
        Ok(_) => panic!("mysql:// must not open"),
    }
}
