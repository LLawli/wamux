//! A library `Node` as the wire's generic `StanzaNode` (#138). The core relays
//! the stanza whole and parses nothing in it: what the server put there (an
//! account lock's one-time `appeal_token`) is the edge's to read.

use std::collections::HashMap;

use whatsapp_rust::wacore_binary::{Node, NodeContent};

use crate::proto::v1 as pb;

/// The node, its attributes as text and its content in the library's own
/// shape, children included, recursively.
pub(crate) fn stanza_node_of(node: &Node) -> pb::StanzaNode {
    // A repeated key keeps the last value: inserting in order does it for free.
    let attrs: HashMap<String, String> = node
        .attrs
        .iter()
        .map(|(key, value)| (key.to_string(), value.as_str().into_owned()))
        .collect();
    pb::StanzaNode {
        tag: node.tag.to_string(),
        attrs,
        content: node.content.as_ref().map(content_of),
    }
}

fn content_of(content: &NodeContent) -> pb::stanza_node::Content {
    match content {
        NodeContent::Bytes(bytes) => pb::stanza_node::Content::Bytes(bytes.clone()),
        NodeContent::String(text) => pb::stanza_node::Content::Text(text.to_string()),
        NodeContent::Nodes(children) => {
            let nodes: Vec<pb::StanzaNode> = children.iter().map(stanza_node_of).collect();
            pb::stanza_node::Content::Children(pb::StanzaNodeList { nodes })
        }
    }
}

#[cfg(test)]
#[path = "stanza_node_tests.rs"]
mod tests;
