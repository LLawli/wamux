//! The rule on the repository's own migration directories.

use std::path::{Path, PathBuf};

use crate::check::{Migration, alignment_drift, read_migrations};

fn crate_dir(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(sub)
}

#[test]
fn the_repository_migrations_are_aligned() {
    let postgres = read_migrations(&crate_dir("migrations")).expect("read migrations/");
    let sqlite = read_migrations(&crate_dir("migrations_sqlite")).expect("read migrations_sqlite/");
    // A floor, so an empty read cannot pass for an aligned one.
    assert!(postgres.len() >= 4, "{postgres:?}");
    assert!(sqlite.len() >= 3, "{sqlite:?}");
    let drift = alignment_drift(&postgres, &sqlite);
    assert!(
        drift.is_empty(),
        "migrations/ and migrations_sqlite/ disagree: {drift:#?}\n\
         From 0005 on, a change has the same number and name in both directories; \
         a change for one engine gets a no-op file of the same name on the other (#66)."
    );
}

#[test]
fn read_migrations_parses_version_and_name_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    for (file, body) in [
        ("0002_second.sql", "SELECT 2;"),
        ("0001_initial.sql", "SELECT 1;"),
        ("README.md", "not a migration"),
        ("0003_draft.sql.bak", "not a migration"),
    ] {
        std::fs::write(dir.path().join(file), body).expect("write");
    }
    let found = read_migrations(dir.path()).expect("read");
    let expected = vec![
        Migration {
            version: 1,
            name: "initial".into(),
        },
        Migration {
            version: 2,
            name: "second".into(),
        },
    ];
    assert_eq!(found, expected);
}
