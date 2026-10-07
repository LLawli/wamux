//! Test-only (#133): a stanza line as the `Client/Recv` log prints it, back
//! into a `Node`, so a captured server stanza can be fed to the library's own
//! parser. It reads what the captures hold and nothing more: elements, double
//! quoted attributes, text content and self-closing tags; no entities, no
//! comments, no mixed content. `xml_line_round_trips` proves it on every line.

use whatsapp_rust::wacore_binary::Node;
use whatsapp_rust::wacore_binary::builder::NodeBuilder;
use whatsapp_rust::wacore_binary::marshal::{marshal, unmarshal_ref};

/// The lines of a capture file that hold a stanza (the header comment is skipped).
pub(crate) fn stanza_lines(file: &str) -> Vec<&str> {
    file.lines()
        .filter(|l| l.starts_with('<') && !l.starts_with("<!--"))
        .collect()
}

/// One stanza line as an owned `Node`. Panics on anything it does not read:
/// a fixture that does not parse is a broken test, not a case to tolerate.
pub(crate) fn node_of_xml(xml: &str) -> Node {
    let mut reader = XmlReader { rest: xml.trim() };
    let node = reader.element();
    assert!(
        reader.rest.is_empty(),
        "trailing input after the stanza: {}",
        reader.rest
    );
    node
}

/// The node as the client receives it: through the binary codec the server
/// and the library use, so content that is not a protocol token arrives as
/// bytes exactly as in production. `marshal` leads with a flag byte.
pub(crate) fn through_the_wire(node: &Node) -> Node {
    let bytes = marshal(node).expect("a capture marshals");
    unmarshal_ref(&bytes[1..])
        .expect("and unmarshals")
        .to_owned()
}

struct XmlReader<'a> {
    rest: &'a str,
}

impl XmlReader<'_> {
    fn element(&mut self) -> Node {
        self.expect("<");
        let tag = self.name().to_string();
        let attributes = self.attributes();
        let mut node = if self.rest.starts_with("/>") {
            self.expect("/>");
            NodeBuilder::new_dynamic(tag).build()
        } else {
            self.expect(">");
            let builder = self.content(NodeBuilder::new_dynamic(tag.clone()));
            self.close(&tag);
            builder.build()
        };
        // `NodeBuilder::attr` takes only static keys; a parsed key is owned.
        for (key, value) in attributes {
            node.attrs.insert(key, value);
        }
        node
    }

    /// Text content, or the child elements, up to the closing tag.
    fn content(&mut self, builder: NodeBuilder) -> NodeBuilder {
        if !self.rest.starts_with('<') {
            let end = self.rest.find('<').expect("text content closes");
            let text = self.rest[..end].to_string();
            self.rest = &self.rest[end..];
            return builder.string_content(text);
        }
        let mut children = Vec::new();
        while !self.rest.starts_with("</") {
            children.push(self.element());
        }
        builder.children(children)
    }

    fn attributes(&mut self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        loop {
            self.rest = self.rest.trim_start();
            if self.rest.starts_with('>') || self.rest.starts_with("/>") {
                return out;
            }
            let key = self.name().to_string();
            self.expect("=\"");
            let end = self.rest.find('"').expect("attribute value closes");
            out.push((key, self.rest[..end].to_string()));
            self.rest = &self.rest[end + 1..];
        }
    }

    fn name(&mut self) -> &str {
        let end = self.rest.find([' ', '>', '/', '=']).expect("a name ends");
        let (name, rest) = self.rest.split_at(end);
        self.rest = rest;
        name
    }

    fn close(&mut self, tag: &str) {
        self.expect("</");
        self.expect(tag);
        self.expect(">");
    }

    fn expect(&mut self, token: &str) {
        assert!(
            self.rest.starts_with(token),
            "expected {token:?} at {:?}",
            self.rest
        );
        self.rest = &self.rest[token.len()..];
    }
}

/// The same XML with each tag's attributes in name order: an owned `Node`
/// renders them sorted, the logged one in wire order.
pub(crate) fn attributes_sorted(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('>').expect("tag closes") + open;
        out.push('<');
        out.push_str(&tag_sorted(&rest[open + 1..close]));
        out.push('>');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn tag_sorted(body: &str) -> String {
    let self_closing = body.ends_with('/');
    let body = body.trim_end_matches('/');
    let (name, attrs) = body.split_once(' ').unwrap_or((body, ""));
    let mut pairs: Vec<&str> = attrs
        .split("\" ")
        .map(|p| p.trim_end_matches('"'))
        .collect();
    pairs.retain(|p| !p.is_empty());
    pairs.sort_unstable();
    let mut out = name.to_string();
    for pair in pairs {
        out.push(' ');
        out.push_str(pair);
        out.push('"');
    }
    if self_closing {
        out.push('/');
    }
    out
}
