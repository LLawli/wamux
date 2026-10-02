//! The rule on synthetic listings, so each kind of drift is shown failing.

use crate::check::{Drift, Engine, Migration, alignment_drift};

fn m(version: u32, name: &str) -> Migration {
    Migration {
        version,
        name: name.to_string(),
    }
}

/// What the two directories hold today: Postgres has one change SQLite never
/// needed (0002), so every later number is one apart.
fn postgres_today() -> Vec<Migration> {
    vec![
        m(1, "initial"),
        m(2, "drop_connection_policy"),
        m(3, "msg_secrets"),
        m(4, "blob_format"),
    ]
}

fn sqlite_today() -> Vec<Migration> {
    vec![m(1, "initial"), m(2, "msg_secrets"), m(3, "blob_format")]
}

fn with(mut base: Vec<Migration>, extra: Migration) -> Vec<Migration> {
    base.push(extra);
    base
}

#[test]
fn today_s_listings_have_no_drift() {
    assert_eq!(
        alignment_drift(&postgres_today(), &sqlite_today()),
        Vec::new()
    );
}

#[test]
fn a_migration_in_only_one_directory_is_drift() {
    let added = m(5, "add_index");
    let pg_only = alignment_drift(&with(postgres_today(), added.clone()), &sqlite_today());
    assert_eq!(
        pg_only,
        vec![Drift::OnlyIn {
            engine: Engine::Postgres,
            migration: added.clone(),
        }]
    );
    let sqlite_only = alignment_drift(&postgres_today(), &with(sqlite_today(), added.clone()));
    assert_eq!(
        sqlite_only,
        vec![Drift::OnlyIn {
            engine: Engine::Sqlite,
            migration: added,
        }]
    );
}

#[test]
fn the_same_change_under_two_numbers_is_drift() {
    let drift = alignment_drift(
        &with(postgres_today(), m(5, "add_index")),
        &with(sqlite_today(), m(6, "add_index")),
    );
    assert_eq!(
        drift,
        vec![Drift::NumberedApart {
            name: "add_index".into(),
            postgres: 5,
            sqlite: 6,
        }]
    );
}

// A change for one engine only gets a no-op file of the same name on the
// other, and then the numbers line up.
#[test]
fn a_noop_counterpart_keeps_the_numbers_aligned() {
    let drift = alignment_drift(
        &with(postgres_today(), m(5, "postgres_only_tweak")),
        &with(sqlite_today(), m(5, "postgres_only_tweak")),
    );
    assert_eq!(drift, Vec::new());
}

// SQLite's next migration is 0005, not 0004: 4 is Postgres's blob_format.
#[test]
fn a_new_migration_cannot_reuse_a_historical_number() {
    let reused = m(4, "add_index");
    let drift = alignment_drift(&postgres_today(), &with(sqlite_today(), reused.clone()));
    assert!(
        drift.contains(&Drift::BelowBaseline {
            engine: Engine::Sqlite,
            migration: reused,
        }),
        "{drift:?}"
    );
}

#[test]
fn rewriting_an_applied_migration_is_drift() {
    let mut renamed = sqlite_today();
    renamed[1] = m(2, "message_secrets");
    let drift = alignment_drift(&postgres_today(), &renamed);
    assert!(
        drift.contains(&Drift::HistoryRewritten {
            engine: Engine::Sqlite,
            expected: sqlite_today(),
            found: renamed.clone(),
        }),
        "{drift:?}"
    );
    let mut dropped = postgres_today();
    dropped.remove(1);
    let drift = alignment_drift(&dropped, &sqlite_today());
    assert!(
        drift.iter().any(|d| matches!(
            d,
            Drift::HistoryRewritten {
                engine: Engine::Postgres,
                ..
            }
        )),
        "{drift:?}"
    );
}
