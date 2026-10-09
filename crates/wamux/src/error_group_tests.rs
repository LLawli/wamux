//! #96: the library rewrites a 409 on the description set into
//! `GroupError::DescriptionConflict`, which has no source, so `client_err` found
//! no code and the edge saw Unavailable without `wa-code`. The code comes back.

use whatsapp_rust::features::GroupError;

use super::client_err;

fn trailer(status: &tonic::Status, key: &str) -> Option<String> {
    status
        .metadata()
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[test]
fn description_conflict_relays_as_wa_code_409() {
    let status = tonic::Status::from(client_err(GroupError::DescriptionConflict));
    // 409 has no gRPC mapping of its own, so it keeps the Unavailable every
    // unmapped server code gets; the code rides in the trailer.
    assert_eq!(status.code(), tonic::Code::Unavailable);
    assert_eq!(trailer(&status, "wa-code").as_deref(), Some("409"));
}

#[test]
fn description_conflict_keeps_the_library_text() {
    let wrapped = anyhow::Error::new(GroupError::DescriptionConflict).context("set description");
    let status = tonic::Status::from(client_err(wrapped));
    assert_eq!(
        trailer(&status, "wa-text").as_deref(),
        Some("the group description changed since it was read")
    );
}

#[test]
fn other_group_errors_stay_opaque_client_errors() {
    let status = tonic::Status::from(client_err(GroupError::InvalidRequest("x".into())));
    assert_eq!(status.code(), tonic::Code::Unavailable);
    assert!(trailer(&status, "wa-code").is_none());
}
