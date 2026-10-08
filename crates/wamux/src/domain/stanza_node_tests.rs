//! #138: a library `Node` crosses whole, with the content in the library's own
//! shape. Each expected value is written out, never built with the function
//! under test.

use std::collections::HashMap;

use whatsapp_rust::wacore_binary::builder::NodeBuilder;
use whatsapp_rust::wacore_binary::{Attrs, Node, NodeContent, NodeValue};

use super::stanza_node_of;
use crate::domain::test_xml::{node_of_xml, through_the_wire};
use crate::proto::v1 as pb;
use pb::stanza_node::Content;

fn attrs_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn leaf(tag: &str, pairs: &[(&str, &str)]) -> pb::StanzaNode {
    pb::StanzaNode {
        tag: tag.to_string(),
        attrs: attrs_of(pairs),
        content: None,
    }
}

#[test]
fn stanza_node_keeps_tag_and_attributes() {
    let node = node_of_xml(r#"<failure reason="403" vt="1"/>"#);
    assert_eq!(
        stanza_node_of(&node),
        leaf("failure", &[("reason", "403"), ("vt", "1")])
    );
}

#[test]
fn stanza_node_relays_text_content_as_text() {
    let node = NodeBuilder::new("text").string_content("hello").build();
    assert_eq!(
        stanza_node_of(&node).content,
        Some(Content::Text("hello".to_string()))
    );
}

/// Bytes stay bytes, including ones that are not UTF-8.
#[test]
fn stanza_node_relays_bytes_content_verbatim() {
    let raw = vec![0xff, 0x00, 0xfe, b'a'];
    let node = NodeBuilder::new("enc").bytes(raw.clone()).build();
    assert_eq!(stanza_node_of(&node).content, Some(Content::Bytes(raw)));
}

#[test]
fn stanza_node_nests_children_in_order() {
    let node = node_of_xml(
        r#"<stream:error code="516"><conflict type="device_removed"><detail k="v"/></conflict><b/></stream:error>"#,
    );
    let conflict = pb::StanzaNode {
        tag: "conflict".to_string(),
        attrs: attrs_of(&[("type", "device_removed")]),
        content: Some(Content::Children(pb::StanzaNodeList {
            nodes: vec![leaf("detail", &[("k", "v")])],
        })),
    };
    let expected = pb::StanzaNode {
        tag: "stream:error".to_string(),
        attrs: attrs_of(&[("code", "516")]),
        content: Some(Content::Children(pb::StanzaNodeList {
            nodes: vec![conflict, leaf("b", &[])],
        })),
    };
    assert_eq!(stanza_node_of(&node), expected);
}

/// An empty child list is content the element had, so it is not unset.
#[test]
fn stanza_node_with_an_empty_child_list_keeps_the_list() {
    let node = Node::new("list", Attrs::new(), Some(NodeContent::Nodes(Vec::new())));
    assert_eq!(
        stanza_node_of(&node).content,
        Some(Content::Children(pb::StanzaNodeList { nodes: Vec::new() }))
    );
}

#[test]
fn stanza_node_without_content_leaves_content_unset() {
    let node = Node::new("ping", Attrs::new(), None);
    assert_eq!(stanza_node_of(&node), leaf("ping", &[]));
}

/// The codec decodes a jid-shaped attribute into `NodeValue::Jid`; it crosses
/// as the jid's wire text, the same string the server sent.
#[test]
fn stanza_node_writes_a_jid_attribute_as_its_wire_string() {
    let node = through_the_wire(&node_of_xml(
        r#"<failure jid="5511999999999@s.whatsapp.net" reason="403"/>"#,
    ));
    assert!(
        matches!(node.attrs.get("jid"), Some(NodeValue::Jid(_))),
        "precondition: the codec decoded a jid value, got {:?}",
        node.attrs.get("jid")
    );
    assert_eq!(
        stanza_node_of(&node).attrs,
        attrs_of(&[("jid", "5511999999999@s.whatsapp.net"), ("reason", "403")])
    );
}
