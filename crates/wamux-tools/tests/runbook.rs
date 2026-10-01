//! #64: the runbook documents every binary that exists, and only those.

use std::collections::BTreeSet;
use std::path::Path;

const LABELS: &[&str] = &[
    "- **Needs:**",
    "- **Env:**",
    "- **Proves:**",
    "- **Writes to WhatsApp:**",
];

/// Binary names under src/bin: `x.rs` and `x/main.rs`.
fn binaries() -> BTreeSet<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin");
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("read src/bin") {
        let path = entry.expect("dir entry").path();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string);
        let is_bin = path.extension().is_some_and(|e| e == "rs") || path.join("main.rs").is_file();
        if let (true, Some(stem)) = (is_bin, stem) {
            names.insert(stem);
        }
    }
    names
}

fn runbook() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("crates/wamux-tools/README.md must exist")
}

/// `### name` sections, each with the text up to the next heading of level <= 3.
fn sections(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("### ") {
            out.push((name.trim().trim_matches('`').to_string(), String::new()));
        } else if line.starts_with("## ") || line.starts_with("# ") {
            out.push((String::new(), String::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    out.retain(|(name, _)| !name.is_empty());
    out
}

#[test]
fn the_binary_set_is_the_one_the_issue_left() {
    let names = binaries();
    assert!(
        names.len() >= 18,
        "absolute floor, not a derived count: {names:?}"
    );
    for gone in ["whois", "send_group", "e2e", "validate1"] {
        assert!(!names.contains(gone), "{gone} was removed by #64");
    }
    assert!(names.contains("e2e_destructive"));
}

#[test]
fn every_binary_has_a_runbook_section_with_all_four_labels() {
    let documented = sections(&runbook());
    for name in binaries() {
        let Some((_, body)) = documented.iter().find(|(n, _)| *n == name) else {
            panic!("README has no `### {name}` section");
        };
        for label in LABELS {
            assert!(body.contains(label), "`### {name}` lacks {label}");
        }
    }
}

#[test]
fn the_runbook_documents_no_binary_that_does_not_exist() {
    let names = binaries();
    for (name, _) in sections(&runbook()) {
        assert!(
            names.contains(&name),
            "README documents `{name}`, which is not a binary"
        );
    }
}
