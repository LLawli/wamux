//! What a socket suite reads off `MockWaServer`, and how it compares a
//! transcription with a live capture. Shared by the service suites (#68, #70):
//! each one asserts the `<iq>` an RPC put on the wire and replays answers the
//! real server sent.

use std::time::Duration;

use wacore_binary::Node;
use wamux::proto::v1 as pb;
use wamux::stress::MockWaServer;

pub fn account_ref(uuid: &str) -> pb::AccountRef {
    pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::Uuid(uuid.to_string())),
    }
}

pub fn attr(node: &Node, key: &str) -> Option<String> {
    node.attrs.get(key).map(|value| value.as_str().into_owned())
}

/// The first child of an `<iq>`: the operation it carries.
pub fn operation(iq: &Node) -> Option<&Node> {
    iq.children().and_then(|children| children.first())
}

fn is_iq(iq: &Node, xmlns: &str, iq_type: &str, child: &str) -> bool {
    let tag = operation(iq).map(|c| c.tag.as_ref()).unwrap_or("");
    attr(iq, "xmlns").as_deref() == Some(xmlns)
        && attr(iq, "type").as_deref() == Some(iq_type)
        && tag == child
}

/// The newest `<iq>` the client sent with this namespace, type and first
/// child (`""` for an IQ with no child), waiting for it to reach the mock.
pub async fn sent_iq(mock: &MockWaServer, xmlns: &str, iq_type: &str, child: &str) -> Node {
    super::poll_until(
        &format!("an <iq xmlns={xmlns} type={iq_type}><{child}>"),
        Duration::from_secs(5),
        || async {
            mock.client_iqs()
                .into_iter()
                .rev()
                .find(|iq| is_iq(iq, xmlns, iq_type, child))
        },
    )
    .await
}

/// How many `<iq>` of a namespace the client has sent so far.
pub fn iqs_in(mock: &MockWaServer, xmlns: &str) -> usize {
    mock.client_iqs()
        .iter()
        .filter(|iq| attr(iq, "xmlns").as_deref() == Some(xmlns))
        .count()
}

/// Assert that `node`, the answer a suite feeds the mock, is what the server
/// sent: `line` is one `<iq>` of a capture file, and only what is inside the
/// `<iq>` is compared, since the mock writes its own envelope.
pub fn assert_transcribes(line: &str, node: &Node) {
    let start = line.find('>').expect("end of the <iq> open tag") + 1;
    let end = line.rfind("</iq>").expect("closing </iq>");
    let rendered = wacore::xml::DisplayableNode(node).to_string();
    assert_eq!(
        attrs_sorted(&rendered),
        attrs_sorted(&line[start..end]),
        "transcription drifted from the capture"
    );
}

/// The same XML with each tag's attributes in name order. An owned `Node`
/// renders its attributes sorted, the logged `NodeRef` in wire order; the
/// comparison is about content, so both sides are put in one order.
pub fn attrs_sorted(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('>').expect("tag closes") + open;
        out.push_str(&tag_sorted(&rest[open + 1..close]));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// One tag body (`name a="1" b="2"/`) rebuilt with sorted attributes.
fn tag_sorted(body: &str) -> String {
    let self_closing = body.ends_with('/');
    let body = body.trim_end_matches('/');
    let (name, mut attrs) = match body.split_once(' ') {
        Some((name, attrs)) => (name, attrs.split("\" ").map(str::to_string).collect()),
        None => (body, Vec::new()),
    };
    for attr in &mut attrs {
        *attr = attr.trim_end_matches('"').to_string();
    }
    attrs.sort();
    let attrs: String = attrs.iter().map(|a| format!(" {a}\"")).collect();
    format!("<{name}{attrs}{}>", if self_closing { "/" } else { "" })
}
