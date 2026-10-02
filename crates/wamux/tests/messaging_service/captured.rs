//! The four DM sends captured live on 2026-10-02 (`captured-2026-10-02.xml`),
//! held against what the client sends the mock for the same four RPCs. What
//! is compared is the skeleton: which attributes the `<message>` carries (with
//! the values of `type` and `edit`), which children it has, and the shape of
//! every `<enc>`. Ids, jids, ciphertext and lengths differ by nature.
//!
//! Normalized on purpose, and why:
//! - `msg` and `pkmsg` are one shape: which one a device gets depends on
//!   whether a session existed, and live every session did.
//! - `<tctoken>` is left out: it is the privacy token a contact issued
//!   earlier, state the mock has no reason to fake.

use std::collections::BTreeSet;

use wacore_binary::Node;
use wamux::proto::v1 as pb;

use crate::common::mock_wire::sent_message;
use crate::harness::{fixture, send_to_peer};

#[derive(Debug, PartialEq, Eq)]
struct Skeleton {
    root: Vec<String>,
    children: Vec<String>,
    encs: BTreeSet<String>,
}

/// One tag body (`name a="1" b="2"`) as its name and `(attr, value)` pairs.
fn tag_parts(body: &str) -> (String, Vec<(String, String)>) {
    let body = body.trim_end_matches('/');
    let (name, rest) = body.split_once(' ').unwrap_or((body, ""));
    let attrs = rest
        .split("\" ")
        .filter(|a| !a.is_empty())
        .filter_map(|a| a.split_once("=\""))
        .map(|(k, v)| (k.to_string(), v.trim_end_matches('"').to_string()))
        .collect();
    (name.to_string(), attrs)
}

/// The attributes of a tag as compared: every name, and the value only for
/// the ones whose value is part of the shape.
fn shaped(attrs: &[(String, String)], keep_values: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = attrs
        .iter()
        .map(|(k, v)| match keep_values.contains(&k.as_str()) {
            true => format!("{k}={}", if v == "pkmsg" { "msg" } else { v }),
            false => k.clone(),
        })
        .collect();
    out.sort();
    out
}

fn skeleton(xml: &str) -> Skeleton {
    let mut skeleton = Skeleton {
        root: Vec::new(),
        children: Vec::new(),
        encs: BTreeSet::new(),
    };
    let mut depth = 0usize;
    for raw in xml.split('<').skip(1) {
        let body = &raw[..raw.find('>').expect("tag closes")];
        if body.starts_with("!--") || body.starts_with('/') {
            depth -= usize::from(body.starts_with('/'));
            continue;
        }
        let (name, attrs) = tag_parts(body);
        match (depth, name.as_str()) {
            (0, _) => skeleton.root = shaped(&attrs, &["type", "edit"]),
            (1, "tctoken") => {}
            (1, _) => skeleton.children.push(name.clone()),
            (_, "enc") => {
                skeleton
                    .encs
                    .insert(shaped(&attrs, &["type", "v", "decrypt-fail"]).join(","));
            }
            _ => {}
        }
        depth += usize::from(!body.ends_with('/'));
    }
    skeleton
}

fn rendered(node: &Node) -> String {
    wacore::xml::DisplayableNode(node).to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_and_mock_sends_have_the_same_skeleton() {
    let file = include_str!("captured-2026-10-02.xml");
    let live: Vec<&str> = file
        .lines()
        .filter(|l| l.starts_with("<message "))
        .collect();
    assert_eq!(live.len(), 4, "text, reaction, edit, revoke");

    let mut f = fixture("live_and_mock_sends_have_the_same_skeleton").await;
    let target = send_to_peer(&mut f, "wamux capture 71").await;
    let reaction = f.messages.send_reaction(pb::SendReactionRequest {
        account: f.a(),
        target: Some(target.clone()),
        emoji: "\u{1F44D}".into(),
    });
    let reaction = reaction
        .await
        .expect("react")
        .into_inner()
        .key
        .expect("key");
    let edit = f.messages.edit_message(pb::EditMessageRequest {
        account: f.a(),
        target: Some(target.clone()),
        new_text: "wamux capture 71 (edited)".into(),
    });
    let edit = edit.await.expect("edit").into_inner().key.expect("key");
    // The edit's stanza lands before the count, or the revoke's search for the
    // newest message could find the edit (#71).
    sent_message(&f.mock, &edit.id).await;
    let before = f.mock.client_messages().len();
    let revoke = pb::DeleteMessageRequest {
        account: f.a(),
        target: Some(target.clone()),
        for_everyone: true,
    };
    f.messages.delete_message(revoke).await.expect("revoke");
    let revoke = crate::harness::newest_stanza(&f.mock, "message", before).await;

    let mock = [
        sent_message(&f.mock, &target.id).await,
        sent_message(&f.mock, &reaction.id).await,
        sent_message(&f.mock, &edit.id).await,
        revoke,
    ];
    for (label, (live, mock)) in ["text", "reaction", "edit", "revoke"]
        .iter()
        .zip(live.iter().zip(&mock))
    {
        assert_eq!(skeleton(&rendered(mock)), skeleton(live), "{label}");
    }
    f.cleanup().await;
}

#[test]
fn the_skeleton_keeps_shape_and_drops_values() {
    let a = r#"<message id="A" to="1@s.whatsapp.net" type="text"><participants><to jid="1@s.whatsapp.net"><enc type="pkmsg" v="2"><!-- 9 bytes --></enc></to></participants><device-identity><!-- 1 bytes --></device-identity><tctoken><!-- 2 bytes --></tctoken></message>"#;
    let b = r#"<message type="text" to="2@s.whatsapp.net" id="B"><participants><to jid="2@s.whatsapp.net"><enc v="2" type="msg"><!-- 7 bytes --></enc></to></participants><device-identity><!-- 3 bytes --></device-identity></message>"#;
    assert_eq!(skeleton(a), skeleton(b));
    assert_eq!(
        skeleton(a).children,
        vec!["participants", "device-identity"]
    );
    let edited = r#"<message edit="1" id="A" to="1@s.whatsapp.net" type="text"></message>"#;
    assert_ne!(skeleton(a).root, skeleton(edited).root);
}
