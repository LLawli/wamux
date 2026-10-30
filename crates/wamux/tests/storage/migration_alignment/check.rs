//! The rule #66 enforces: from 0005 on, the same change has the same number
//! and name in `migrations/` (Postgres) and `migrations_sqlite/`. What came
//! before is pinned as one named exception, because sqlx stores each applied
//! migration's version and checksum: renumbering would break every store.

use std::path::Path;

use anyhow::Context;

/// One migration file, `NNNN_name.sql`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migration {
    pub version: u32,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Postgres,
    Sqlite,
}

/// One way the two directories disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drift {
    /// A migration past the baseline exists in one directory only.
    OnlyIn {
        engine: Engine,
        migration: Migration,
    },
    /// The same change (same name) carries two different numbers.
    NumberedApart {
        name: String,
        postgres: u32,
        sqlite: u32,
    },
    /// A migration that is not historical but is numbered at or below the
    /// baseline (e.g. a new SQLite `0004_*`).
    BelowBaseline {
        engine: Engine,
        migration: Migration,
    },
    /// The historical listing of an engine is not exactly what was applied.
    HistoryRewritten {
        engine: Engine,
        expected: Vec<Migration>,
        found: Vec<Migration>,
    },
}

/// Last number of the historical offset. Numbers above it must match.
const BASELINE: u32 = 4;

/// Postgres history as applied. 0002 is a change SQLite never needed, which is
/// why every SQLite number below the baseline is one lower. sqlx stores each
/// applied version and checksum, so none of this can be renumbered or renamed.
const POSTGRES_HISTORY: [(u32, &str); 4] = [
    (1, "initial"),
    (2, "drop_connection_policy"),
    (3, "msg_secrets"),
    (4, "blob_format"),
];

/// SQLite history as applied (see `POSTGRES_HISTORY` for why it is frozen).
const SQLITE_HISTORY: [(u32, &str); 3] = [(1, "initial"), (2, "msg_secrets"), (3, "blob_format")];

/// Every `NNNN_name.sql` in `dir`, sorted by version. Other files are ignored.
pub fn read_migrations(dir: &Path) -> anyhow::Result<Vec<Migration>> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading migrations directory {}", dir.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("listing {}", dir.display()))?;
        let file_name = entry.file_name();
        if let Some(migration) = file_name.to_str().and_then(parse_migration_file_name) {
            found.push(migration);
        }
    }
    found.sort_by_key(|migration| migration.version);
    Ok(found)
}

/// `0007_add_index.sql` gives version 7 and name `add_index`; anything else is None.
fn parse_migration_file_name(file_name: &str) -> Option<Migration> {
    let stem = file_name.strip_suffix(".sql")?;
    let (digits, name) = stem.split_once('_')?;
    let is_version = digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_digit());
    if !is_version || name.is_empty() {
        return None;
    }
    Some(Migration {
        version: digits.parse().ok()?,
        name: name.to_string(),
    })
}

/// Every disagreement between the two listings, empty when they are aligned.
pub fn alignment_drift(postgres: &[Migration], sqlite: &[Migration]) -> Vec<Drift> {
    let mut drift = history_drift(Engine::Postgres, postgres, &POSTGRES_HISTORY);
    drift.extend(history_drift(Engine::Sqlite, sqlite, &SQLITE_HISTORY));
    drift.extend(numbering_drift(postgres, sqlite));
    drift
}

fn expected_history(history: &[(u32, &str)]) -> Vec<Migration> {
    history
        .iter()
        .map(|&(version, name)| Migration {
            version,
            name: name.to_string(),
        })
        .collect()
}

/// Anything at or below the baseline must be exactly the applied history.
fn history_drift(engine: Engine, listing: &[Migration], history: &[(u32, &str)]) -> Vec<Drift> {
    let expected = expected_history(history);
    let found: Vec<Migration> = listing
        .iter()
        .filter(|migration| migration.version <= BASELINE)
        .cloned()
        .collect();
    let mut drift: Vec<Drift> = found
        .iter()
        .filter(|migration| !expected.contains(migration))
        .map(|migration| Drift::BelowBaseline {
            engine,
            migration: migration.clone(),
        })
        .collect();
    if found != expected {
        drift.push(Drift::HistoryRewritten {
            engine,
            expected,
            found,
        });
    }
    drift
}

/// Sorted here so the output order does not depend on the caller's order.
fn past_baseline(listing: &[Migration]) -> Vec<&Migration> {
    let mut past: Vec<&Migration> = listing.iter().filter(|m| m.version > BASELINE).collect();
    past.sort_by_key(|migration| migration.version);
    past
}

/// Past the baseline the engines are matched by name: same name, same number.
fn numbering_drift(postgres: &[Migration], sqlite: &[Migration]) -> Vec<Drift> {
    let (pg_past, sqlite_past) = (past_baseline(postgres), past_baseline(sqlite));
    let mut drift = Vec::new();
    for pg in &pg_past {
        match sqlite_past.iter().find(|s| s.name == pg.name) {
            Some(s) if s.version != pg.version => drift.push(Drift::NumberedApart {
                name: pg.name.clone(),
                postgres: pg.version,
                sqlite: s.version,
            }),
            Some(_) => {}
            None => drift.push(only_in(Engine::Postgres, pg)),
        }
    }
    for s in &sqlite_past {
        if !pg_past.iter().any(|pg| pg.name == s.name) {
            drift.push(only_in(Engine::Sqlite, s));
        }
    }
    drift
}

fn only_in(engine: Engine, migration: &Migration) -> Drift {
    Drift::OnlyIn {
        engine,
        migration: migration.clone(),
    }
}
