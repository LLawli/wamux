//! `AccountId`, `ExternalRef`, `AccountRef` (#114). The messages are the ones
//! `AccountRegistry::resolve` answers today.

use wamux_proto::v1 as pb;
use wamux_proto::v1::account_ref::Ref;

use crate::{AccountId, AccountRef, ExternalRef, WamuxError};

const UUID: &str = "08d9f022-cba8-4ed7-8baa-3d077b219d8b";

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

fn proto(reference: Option<Ref>) -> pb::AccountRef {
    pb::AccountRef { r#ref: reference }
}

#[test]
fn an_account_id_parses_a_uuid_and_prints_it_back() {
    let id = AccountId::parse(UUID).unwrap();
    assert_eq!(id.to_string(), UUID);
    assert_eq!(id.as_uuid().to_string(), UUID);
}

#[test]
fn a_bad_uuid_names_the_offending_value() {
    assert_eq!(
        invalid_argument(AccountId::parse("nope")),
        "bad uuid 'nope'"
    );
}

/// Not validated on purpose: an empty one is looked up and answers NotFound,
/// as before (#114 changes nothing on the wire).
#[test]
fn an_external_ref_keeps_its_text_even_empty() {
    assert_eq!(ExternalRef::new("trabalho").as_str(), "trabalho");
    assert_eq!(ExternalRef::new("").as_str(), "");
}

#[test]
fn an_absent_account_is_missing_account_ref() {
    assert_eq!(
        invalid_argument(AccountRef::from_proto(None)),
        "missing account ref"
    );
}

#[test]
fn an_account_with_no_ref_set_is_missing_account_ref() {
    assert_eq!(
        invalid_argument(AccountRef::from_proto(Some(&proto(None)))),
        "missing account ref"
    );
}

#[test]
fn a_uuid_ref_becomes_an_id() {
    let account = AccountRef::from_proto(Some(&proto(Some(Ref::Uuid(UUID.into()))))).unwrap();
    assert_eq!(account, AccountRef::Id(AccountId::parse(UUID).unwrap()));
}

#[test]
fn a_bad_uuid_ref_names_the_offending_value() {
    let result = AccountRef::from_proto(Some(&proto(Some(Ref::Uuid("nope".into())))));
    assert_eq!(invalid_argument(result), "bad uuid 'nope'");
}

#[test]
fn an_external_ref_becomes_external_even_empty() {
    let account =
        AccountRef::from_proto(Some(&proto(Some(Ref::ExternalRef("pessoal".into()))))).unwrap();
    assert_eq!(account, AccountRef::External(ExternalRef::new("pessoal")));
    let empty =
        AccountRef::from_proto(Some(&proto(Some(Ref::ExternalRef(String::new()))))).unwrap();
    assert_eq!(empty, AccountRef::External(ExternalRef::new("")));
}

#[test]
fn try_from_and_back_to_proto_round_trip() {
    for reference in [Ref::Uuid(UUID.into()), Ref::ExternalRef("pessoal".into())] {
        let wire = proto(Some(reference));
        let account = AccountRef::try_from(wire.clone()).unwrap();
        assert_eq!(pb::AccountRef::from(account), wire);
    }
}
