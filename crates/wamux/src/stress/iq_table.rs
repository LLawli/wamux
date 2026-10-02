//! The mock's IQ answer table (#68): which `<iq>` gets which answer. Split out
//! of `mock_wa_server.rs` so the handshake code and the answer policy each
//! stay a readable size.

use std::sync::Arc;

use wacore_binary::builder::NodeBuilder;

/// What the mock answers an IQ with, standing in for the server. A rule is
/// keyed by (`xmlns`, `type`, first child tag) because that is what tells one
/// RPC from another on the wire: the group queries all share `w:g2`, the
/// newsletter ones share `newsletter` (#68). Shared with every connection, so a
/// test sets it before the call it wants answered.
#[derive(Default)]
pub(super) struct IqAnswers {
    /// Oldest first; lookup walks newest first so a test can re-register a key
    /// to swap the answer between two calls.
    rules: Vec<IqRule>,
}

/// One registered answer and the IQ shape it applies to.
struct IqRule {
    xmlns: String,
    /// `"*"` matches any `type`.
    iq_type: String,
    /// Tag of the IQ's first child (`""` for an IQ without one); `"*"` matches any.
    child_tag: String,
    answer: IqAnswer,
}

/// The two things a server answers an IQ with.
pub(super) enum IqAnswer {
    /// `<iq type=result>` wrapping this node.
    Result(Box<wacore_binary::Node>),
    /// `<iq type=error><error code text/></iq>`.
    Refusal { code: u16, text: String },
}

/// The identifying parts of a client `<iq>`, taken once per stanza.
struct IqShape {
    id: String,
    xmlns: Option<String>,
    iq_type: Option<String>,
    child_tag: String,
}

impl IqAnswers {
    /// Register an answer; it wins over every older rule it overlaps.
    pub(super) fn push(&mut self, xmlns: &str, iq_type: &str, child_tag: &str, answer: IqAnswer) {
        self.rules.push(IqRule {
            xmlns: xmlns.to_string(),
            iq_type: iq_type.to_string(),
            child_tag: child_tag.to_string(),
            answer,
        });
    }

    /// The newest rule matching `shape`, if any.
    fn find(&self, shape: &IqShape) -> Option<&IqAnswer> {
        self.rules
            .iter()
            .rev()
            .find(|rule| rule.matches(shape))
            .map(|rule| &rule.answer)
    }
}

impl IqRule {
    fn matches(&self, shape: &IqShape) -> bool {
        let any = |want: &str, got: &str| want == "*" || want == got;
        shape.xmlns.as_deref() == Some(self.xmlns.as_str())
            && any(&self.iq_type, shape.iq_type.as_deref().unwrap_or(""))
            && any(&self.child_tag, &shape.child_tag)
    }
}

pub(super) type SharedIqAnswers = Arc<std::sync::Mutex<IqAnswers>>;

/// The reply to a client `<iq>`: the answer a test registered for its shape
/// (`MockWaServer::answer_iq`, `answer_iq_error`), else a bare
/// `<iq type=result>`. `None` when the IQ carries no id to answer.
pub(super) fn iq_reply_for(
    node: &wacore_binary::NodeRef<'_>,
    answers: &SharedIqAnswers,
) -> Option<wacore_binary::Node> {
    let id = node.get_attr("id").map(|v| v.to_string())?;
    let shape = IqShape {
        id,
        xmlns: node.get_attr("xmlns").map(|v| v.to_string()),
        iq_type: node.get_attr("type").map(|v| v.to_string()),
        child_tag: first_child_tag(node),
    };
    Some(iq_reply(&shape, answers))
}

/// Tag of the IQ's first child, `""` when it has none (the group photo remove).
fn first_child_tag(node: &wacore_binary::NodeRef<'_>) -> String {
    node.children()
        .and_then(|children| children.first())
        .map(|child| child.tag.to_string())
        .unwrap_or_default()
}

fn iq_reply(shape: &IqShape, answers: &SharedIqAnswers) -> wacore_binary::Node {
    let bare = |kind: &str| {
        NodeBuilder::new("iq")
            .attr("type", kind)
            .attr("id", shape.id.clone())
    };
    let Ok(answers) = answers.lock() else {
        return bare("result").build();
    };
    match answers.find(shape) {
        Some(IqAnswer::Result(body)) => bare("result").children([(**body).clone()]).build(),
        Some(IqAnswer::Refusal { code, text }) => {
            let error = NodeBuilder::new("error")
                .attr("code", code.to_string())
                .attr("text", text.clone())
                .build();
            bare("error").children([error]).build()
        }
        None => bare("result").build(),
    }
}
