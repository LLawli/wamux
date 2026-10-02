//! Small readers over a `wacore_binary::Node` the mock modules share (#71).

use wacore_binary::{Node, NodeContent};
use whatsapp_rust::Jid;

/// The attribute as a string, if present.
pub(super) fn str_attr(node: &Node, key: &str) -> Option<String> {
    node.attrs.get(key).map(|value| value.as_str().into_owned())
}

/// The attribute parsed as a jid, if present and well-formed.
pub(super) fn jid_attr(node: &Node, key: &str) -> Option<Jid> {
    node.attrs.get(key).and_then(|value| value.to_jid())
}

/// The node's binary content, or an error naming the node.
pub(super) fn bytes_of(node: &Node) -> anyhow::Result<&[u8]> {
    match &node.content {
        Some(NodeContent::Bytes(bytes)) => Ok(bytes),
        _ => Err(anyhow::anyhow!("<{}> carries no binary content", node.tag)),
    }
}

/// The direct children with `tag`.
pub(super) fn children_named<'a>(node: &'a Node, tag: &'a str) -> impl Iterator<Item = &'a Node> {
    node.children()
        .into_iter()
        .flatten()
        .filter(move |c| c.tag == tag)
}
