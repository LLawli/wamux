//! `commit_bench <database_url> [accounts] [writes] [batch_cap]` (#172): N
//! accounts write M sessions at once, each write waiting for its own commit,
//! and the run prints writes per second, p50/p99 latency and jobs per commit.
//! See `wamux_tools::commit_bench` and the runbook.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use wamux::storage::StorageEngine;
use wamux::storage::sql::SqlStore;
use wamux_tools::commit_bench::{
    ALLOW_TMPFS_VAR, BenchReport, database_file_of, fs_type_of, run_writers, tmpfs_refusal,
};

const USAGE: &str = "usage: commit_bench <database_url> [accounts=10] [writes=30] [batch_cap]\n\
    batch_cap applies to sqlite://, and to turso:// (it needs the `turso` feature)";

struct Args {
    database_url: String,
    accounts: usize,
    writes: usize,
    batch_cap: Option<usize>,
}

/// A positive integer argument, or the default when it is absent.
fn count_arg(
    args: &[String],
    index: usize,
    default: Option<usize>,
) -> Result<Option<usize>, String> {
    match args.get(index) {
        None => Ok(default),
        Some(text) => match text.parse::<usize>() {
            Ok(count) if count > 0 => Ok(Some(count)),
            _ => Err(format!(
                "argument {index} must be a positive integer, got '{text}'"
            )),
        },
    }
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let database_url = args.first().ok_or("missing <database_url>")?.clone();
    if args.len() > 4 {
        return Err("too many arguments".to_string());
    }
    Ok(Args {
        database_url,
        accounts: count_arg(args, 1, Some(10))?.unwrap_or(10),
        writes: count_arg(args, 2, Some(30))?.unwrap_or(30),
        batch_cap: count_arg(args, 3, None)?,
    })
}

/// The filesystem the store file will live on, read from the parent directory
/// (the file may not exist yet).
fn store_fs_type(database_url: &str) -> Option<String> {
    let file = database_file_of(database_url)?;
    let parent = file.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = std::fs::canonicalize(parent.unwrap_or(Path::new("."))).ok()?;
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    fs_type_of(&dir.join("store"), &mounts)
}

/// A store opened with the given batch cap, or `None` when the engine named
/// by the DSN has no cap to set. `sqlite://` takes it always (#173); `turso://`
/// only in a build with the `turso` feature.
async fn open_with_cap(
    database_url: &str,
    cap: usize,
) -> anyhow::Result<Option<Arc<dyn StorageEngine>>> {
    if database_url.starts_with("sqlite:") {
        let store = SqlStore::open_sqlite_with_batch_cap(database_url, cap).await?;
        return Ok(Some(Arc::new(store)));
    }
    open_turso_with_cap(database_url, cap).await
}

#[cfg(feature = "turso")]
async fn open_turso_with_cap(
    database_url: &str,
    cap: usize,
) -> anyhow::Result<Option<Arc<dyn StorageEngine>>> {
    use wamux::storage::turso::TursoStore;
    if !database_url.starts_with("turso://") {
        return Ok(None);
    }
    let store = TursoStore::open_with_batch_cap(database_url, cap).await?;
    Ok(Some(Arc::new(store)))
}

#[cfg(not(feature = "turso"))]
async fn open_turso_with_cap(
    _database_url: &str,
    _cap: usize,
) -> anyhow::Result<Option<Arc<dyn StorageEngine>>> {
    Ok(None)
}

async fn open_store(args: &Args) -> anyhow::Result<Arc<dyn StorageEngine>> {
    if let Some(cap) = args.batch_cap {
        if let Some(store) = open_with_cap(&args.database_url, cap).await? {
            return Ok(store);
        }
        eprintln!(
            "note: batch_cap is ignored for this database_url (sqlite:// always, turso:// with the `turso` feature)"
        );
    }
    Ok(wamux::storage::open_engine(&args.database_url, 16).await?)
}

async fn run(args: Args) -> anyhow::Result<ExitCode> {
    let fs_type = store_fs_type(&args.database_url);
    let allow_tmpfs = std::env::var(ALLOW_TMPFS_VAR).is_ok_and(|value| value == "1");
    if let Some(refusal) = tmpfs_refusal(fs_type.as_deref(), allow_tmpfs) {
        eprintln!("{refusal}");
        return Ok(ExitCode::from(1));
    }
    let engine = open_store(&args).await?;
    let run = run_writers(&engine, args.accounts, args.writes).await?;
    let report = BenchReport {
        engine: args
            .database_url
            .split("://")
            .next()
            .unwrap_or("unknown")
            .to_string(),
        accounts: args.accounts,
        writes_per_account: args.writes,
        elapsed: run.elapsed,
        latencies_ms: run.latencies_ms,
        stats: run.stats,
        tmpfs_override: allow_tmpfs && fs_type.as_deref() == Some("tmpfs"),
        fs_type,
    };
    println!("{}", report.render());
    Ok(ExitCode::SUCCESS)
}

#[tokio::main]
async fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(args) => args,
        Err(reason) => {
            eprintln!("{reason}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("commit_bench failed: {error:#}");
            ExitCode::from(1)
        }
    }
}
