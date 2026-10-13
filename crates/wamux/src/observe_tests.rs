//! #80: metric labels come only from the methods this daemon serves; any
//! other path is `unknown`, so made-up paths cannot grow the series count.

use prost::Message;

use super::{KnownMethods, code_name};

fn known() -> KnownMethods {
    KnownMethods::wamux()
}

#[test]
fn a_known_method_is_its_own_label() {
    let known = known();
    for method in [
        "wamux.v1.AccountService/ListAccounts",
        "wamux.v1.EventService/SubscribeEvents",
        "wamux.v1.MediaService/DownloadMedia",
        "wamux.v1.AdminService/GetMetrics",
    ] {
        assert_eq!(known.label(&format!("/{method}")), method);
    }
}

#[test]
fn every_method_in_the_descriptor_is_known() {
    let known = known();
    let set = prost_types::FileDescriptorSet::decode(crate::proto::FILE_DESCRIPTOR_SET)
        .expect("our own descriptor set");
    let mut methods = 0;
    for file in &set.file {
        for service in &file.service {
            for method in &service.method {
                let label = format!("{}.{}/{}", file.package(), service.name(), method.name());
                assert_eq!(known.label(&format!("/{label}")), label);
                methods += 1;
            }
        }
    }
    // An absolute floor (74 RPCs on 2026-10-13): a set that decoded nothing
    // must not pass by checking nothing.
    assert!(
        methods >= 70,
        "only {methods} methods in the descriptor set"
    );
}

#[test]
fn reflection_is_a_known_method() {
    let method = "grpc.reflection.v1.ServerReflection/ServerReflectionInfo";
    assert_eq!(known().label(&format!("/{method}")), method);
}

#[test]
fn anything_else_is_unknown() {
    let known = known();
    for path in [
        "",
        "/",
        "/x",
        "/wamux.v1.AccountService",
        "/wamux.v1.AccountService/NoSuchMethod",
        "/wamux.v1.NoSuchService/ListAccounts",
        "/wamux.v1.AccountService/ListAccounts/extra",
        "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo",
    ] {
        assert_eq!(known.label(path), "unknown", "{path:?}");
    }
}

#[test]
fn a_garbage_descriptor_set_is_refused() {
    assert!(KnownMethods::from_descriptor_set(&[0xff, 0xff, 0xff, 0xff]).is_err());
}

#[test]
fn code_name_maps_known_and_unknown() {
    assert_eq!(code_name(None), "ok");
    assert_eq!(code_name(Some("0")), "ok");
    assert_eq!(code_name(Some("5")), "not_found");
    assert_eq!(code_name(Some("9")), "failed_precondition");
    assert_eq!(code_name(Some("99")), "unknown");
}
