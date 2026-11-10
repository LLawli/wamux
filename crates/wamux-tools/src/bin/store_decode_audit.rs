//! `store_decode_audit <database_url>`: decode every stored blob with the
//! pinned library and print per-table counts. Exit 1 when any blob fails.
//! Point it at a COPY of the store (`sqlite3 wamux.db ".backup copy.db"`),
//! never at a store a daemon is running on. Unsealed stores only: a sealed
//! blob would need the store key, and every row would read as a failure.
//! See `wamux_tools::store_decode_audit`.

use std::process::ExitCode;

use sqlx::{PgPool, SqlitePool};
use wamux_tools::store_decode_audit::{BlobTable, TableTally, decode_blob};

const USAGE: &str = "usage: store_decode_audit <sqlite://path | postgres://...>";

/// The two engines sqlx reads; turso stores are SQLite files on disk.
enum AuditPool {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl AuditPool {
    async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        if url.starts_with("postgres") {
            return Ok(Self::Postgres(PgPool::connect(url).await?));
        }
        Ok(Self::Sqlite(
            SqlitePool::connect(&format!("{url}?mode=ro")).await?,
        ))
    }

    async fn blobs(&self, sql: &'static str) -> Result<Vec<Vec<u8>>, sqlx::Error> {
        let rows: Vec<(Vec<u8>,)> = match self {
            Self::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await?,
            Self::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await?,
        };
        Ok(rows.into_iter().map(|(bytes,)| bytes).collect())
    }
}

async fn tally(pool: &AuditPool, table: BlobTable) -> Result<TableTally, sqlx::Error> {
    let mut tally = TableTally::default();
    for bytes in pool.blobs(table.select_all()).await? {
        tally.record(decode_blob(table, &bytes));
    }
    Ok(tally)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let Some(url) = std::env::args().nth(1) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let pool = match AuditPool::connect(&url).await {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("cannot open the store: {err}");
            return ExitCode::from(2);
        }
    };
    let mut failed = 0;
    for table in BlobTable::ALL {
        let tally = match tally(&pool, table).await {
            Ok(tally) => tally,
            Err(err) => {
                eprintln!("cannot read {table:?}: {err}");
                return ExitCode::from(2);
            }
        };
        println!(
            "{table:?}: {} decoded, {} failed",
            tally.decoded,
            tally.failed()
        );
        for (error, count) in &tally.failures {
            println!("  {count} x {error}");
        }
        failed += tally.failed();
    }
    if failed > 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
