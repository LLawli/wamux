//! #280: the schema the snippet page is generated from. The page trusts these
//! shapes to pick a streaming call, to fill one branch of a oneof, and to
//! name a proto3 `optional` field, so each is pinned against the real contract.

use std::collections::BTreeSet;

use serde_json::Value;
use wamux_tools::proto_schema::schema_json;

fn schema() -> Value {
    let text: String =
        schema_json(wamux::proto::FILE_DESCRIPTOR_SET).expect("the descriptor exports");
    serde_json::from_str(&text).expect("the export is JSON")
}

fn named<'a>(list: &'a Value, full_name: &str) -> &'a Value {
    let items = list.as_array().expect("a list of declarations");
    let found = items.iter().find(|item| item["fullName"] == full_name);
    found.unwrap_or_else(|| panic!("{full_name} is missing from the schema"))
}

fn method<'a>(schema: &'a Value, service: &str, name: &str) -> &'a Value {
    let methods = named(&schema["services"], service)["methods"]
        .as_array()
        .expect("methods");
    let found = methods.iter().find(|m| m["name"] == name);
    found.unwrap_or_else(|| panic!("{service}/{name} is missing"))
}

fn field<'a>(schema: &'a Value, message: &str, name: &str) -> &'a Value {
    let fields = named(&schema["messages"], message)["fields"]
        .as_array()
        .expect("fields");
    let found = fields.iter().find(|f| f["name"] == name);
    found.unwrap_or_else(|| panic!("{message}.{name} is missing"))
}

#[test]
fn the_export_is_byte_identical_across_runs_and_sorted() {
    let first: String = schema_json(wamux::proto::FILE_DESCRIPTOR_SET).expect("export");
    let second: String = schema_json(wamux::proto::FILE_DESCRIPTOR_SET).expect("export");
    assert_eq!(first, second);
    assert!(
        first.ends_with("}\n"),
        "one final newline, like the committed file"
    );
    let schema: Value = serde_json::from_str(&first).expect("JSON");
    for list in ["services", "messages", "enums"] {
        let names: Vec<&str> = schema[list]
            .as_array()
            .expect(list)
            .iter()
            .map(|i| i["fullName"].as_str().expect("name"))
            .collect();
        let mut sorted: Vec<&str> = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "{list} sorted by full name");
    }
}

#[test]
fn every_service_of_the_contract_is_there_and_nothing_of_the_store() {
    let schema: Value = schema();
    let services: Vec<&Value> = schema["services"]
        .as_array()
        .expect("services")
        .iter()
        .collect();
    assert!(
        services.len() >= 9,
        "absolute floor, not a derived count: {}",
        services.len()
    );
    for name in [
        "AccountService",
        "MessagingService",
        "EventService",
        "MediaService",
        "AdminService",
    ] {
        named(&schema["services"], &format!("wamux.v1.{name}"));
    }
    let messages = schema["messages"].as_array().expect("messages");
    assert!(messages.iter().all(|m| {
        m["fullName"]
            .as_str()
            .is_some_and(|n| n.starts_with("wamux.v1."))
    }));
}

#[test]
fn every_method_type_resolves_to_a_message_of_the_schema() {
    let schema: Value = schema();
    let messages: BTreeSet<&str> = schema["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|m| m["fullName"].as_str())
        .collect();
    for service in schema["services"].as_array().expect("services") {
        for method in service["methods"].as_array().expect("methods") {
            for side in ["inputType", "outputType"] {
                let name: &str = method[side].as_str().expect("type name");
                assert!(
                    messages.contains(name),
                    "{} {side} {name} has no message",
                    method["name"]
                );
            }
        }
    }
}

#[test]
fn streaming_flags_follow_the_proto() {
    let schema: Value = schema();
    let cases = [
        ("wamux.v1.MessagingService", "SendText", false, false),
        ("wamux.v1.MessagingService", "SendMedia", true, false),
        ("wamux.v1.AccountService", "PairWithQr", false, true),
        ("wamux.v1.EventService", "SubscribeEvents", false, true),
    ];
    for (service, name, client, server) in cases {
        let found: &Value = method(&schema, service, name);
        assert_eq!(found["clientStreaming"], client, "{name} client streaming");
        assert_eq!(found["serverStreaming"], server, "{name} server streaming");
    }
    let send_text: &Value = method(&schema, "wamux.v1.MessagingService", "SendText");
    assert_eq!(send_text["inputType"], "wamux.v1.SendTextRequest");
    assert_eq!(send_text["outputType"], "wamux.v1.SendResult");
}

#[test]
fn a_oneof_names_its_members_and_proto3_optional_is_not_one() {
    let schema: Value = schema();
    assert_eq!(
        named(&schema["messages"], "wamux.v1.AccountRef")["oneofs"],
        serde_json::json!(["ref"])
    );
    let external: &Value = field(&schema, "wamux.v1.AccountRef", "external_ref");
    assert_eq!(external["oneof"], "ref");
    assert_eq!(external["jsonName"], "externalRef");
    let create: &Value = named(&schema["messages"], "wamux.v1.CreateAccountRequest");
    assert_eq!(
        create["oneofs"],
        serde_json::json!([]),
        "the synthetic oneof of `optional` is hidden"
    );
    let optional: &Value = field(&schema, "wamux.v1.CreateAccountRequest", "external_ref");
    assert_eq!(optional["label"], "optional");
    assert!(optional.get("oneof").is_none());
}

#[test]
fn message_enum_repeated_and_map_fields_carry_their_types() {
    let schema: Value = schema();
    let to: &Value = field(&schema, "wamux.v1.SendTextRequest", "to");
    assert_eq!(
        (&to["kind"], &to["typeName"], &to["label"]),
        (
            &"message".into(),
            &"wamux.v1.Jid".into(),
            &"singular".into()
        )
    );
    let mentions: &Value = field(&schema, "wamux.v1.SendTextRequest", "mentions");
    assert_eq!(mentions["label"], "repeated");
    let media_type: &Value = field(&schema, "wamux.v1.SendMediaHeader", "media_type");
    assert_eq!(
        (&media_type["kind"], &media_type["typeName"]),
        (&"enum".into(), &"wamux.v1.MediaType".into())
    );
    let attrs: &Value = field(&schema, "wamux.v1.StanzaNode", "attrs");
    assert_eq!(
        (&attrs["label"], &attrs["mapKey"], &attrs["kind"]),
        (&"map".into(), &"string".into(), &"string".into())
    );
    let messages = schema["messages"].as_array().expect("messages");
    assert!(
        !messages.iter().any(|m| m["name"] == "AttrsEntry"),
        "map entries are folded into the field"
    );
}

#[test]
fn comments_come_from_the_proto_source() {
    let schema: Value = schema();
    let pair: &Value = method(&schema, "wamux.v1.AccountService", "PairWithQr");
    assert!(
        pair["comment"]
            .as_str()
            .is_some_and(|c| c.contains("FailedPrecondition")),
        "{pair}"
    );
    let trailing: &Value = field(&schema, "wamux.v1.PairedInfo", "business_name");
    assert_eq!(
        trailing["comment"],
        "empty for personal (non-business) accounts"
    );
    let media = named(&schema["enums"], "wamux.v1.MediaType");
    assert_eq!(media["values"][0]["name"], "MEDIA_TYPE_UNSPECIFIED");
    assert!(
        media["comment"]
            .as_str()
            .is_some_and(|c| c.starts_with("The kind of a media payload"))
    );
}
