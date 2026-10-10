//! `commit_bench` (#172): how many writes per second a store commits, and how
//! long each write waits for its own commit, with N accounts writing at once.
//!
//! The pure parts live here (the filesystem check, the percentiles, the
//! report); the binary in `src/bin/commit_bench.rs` opens the store and drives
//! the writers. On tmpfs an fsync costs nothing, so `synchronous = FULL` and
//! group commit both look free there: the binary refuses such a path unless
//! `WAMUX_BENCH_ALLOW_TMPFS=1`, and then the report says so.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wacore::store::traits::Backend;
use wamux::storage::{CommitStats, StorageEngine};

/// Set to `1` to run on tmpfs anyway; the report is then marked.
pub const ALLOW_TMPFS_VAR: &str = "WAMUX_BENCH_ALLOW_TMPFS";

/// The filesystem type of `path`, from the text of `/proc/mounts`: the mount
/// whose mount point is the longest whole-component prefix of the path.
pub fn fs_type_of(path: &Path, mounts: &str) -> Option<String> {
    mounts
        .lines()
        .filter_map(mount_entry)
        .filter(|(mount_point, _)| path.starts_with(mount_point))
        .max_by_key(|(mount_point, _)| mount_point.components().count())
        .map(|(_, fs_type)| fs_type)
}

/// `(mount point, fs type)` of one `/proc/mounts` line.
fn mount_entry(line: &str) -> Option<(PathBuf, String)> {
    let mut fields = line.split_whitespace();
    let _device = fields.next()?;
    let mount_point = fields.next()?;
    let fs_type = fields.next()?;
    // The kernel writes a space in a mount point as the octal escape \040.
    Some((
        PathBuf::from(mount_point.replace("\\040", " ")),
        fs_type.to_string(),
    ))
}

/// Why the run is refused, or `None` to go ahead. Only tmpfs is refused, and
/// only without the override; an unknown filesystem runs.
pub fn tmpfs_refusal(fs_type: Option<&str>, allow_tmpfs: bool) -> Option<String> {
    if fs_type != Some("tmpfs") || allow_tmpfs {
        return None;
    }
    Some(format!(
        "refusing to run on tmpfs: an fsync costs nothing there, so the numbers would hide \
         the whole effect of group commit. Use a real disk, or set {ALLOW_TMPFS_VAR}=1 to run anyway"
    ))
}

/// The nearest-rank percentile `p` (0 to 100) of an ascending sample.
pub fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    if sorted_ms.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0 * sorted_ms.len() as f64).ceil() as usize;
    sorted_ms[rank.clamp(1, sorted_ms.len()) - 1]
}

/// The file a `sqlite://` or `turso://` DSN names, without any query string;
/// `None` for the DSNs that name no local file (Postgres).
pub fn database_file_of(database_url: &str) -> Option<PathBuf> {
    let (scheme, rest) = database_url.split_once("://")?;
    if scheme != "sqlite" && scheme != "turso" {
        return None;
    }
    let path = rest.split('?').next().unwrap_or(rest);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// One run: what was measured and on what.
#[derive(Debug, Clone)]
pub struct BenchReport {
    pub engine: String,
    pub accounts: usize,
    pub writes_per_account: usize,
    pub elapsed: Duration,
    /// The wait of every write for its commit, in milliseconds.
    pub latencies_ms: Vec<f64>,
    /// `None` for an engine that does not group commits.
    pub stats: Option<CommitStats>,
    pub fs_type: Option<String>,
    /// The run went ahead on tmpfs through the override.
    pub tmpfs_override: bool,
}

impl BenchReport {
    pub fn writes_per_second(&self) -> f64 {
        let seconds = self.elapsed.as_secs_f64();
        if seconds <= 0.0 {
            return 0.0;
        }
        (self.accounts * self.writes_per_account) as f64 / seconds
    }

    /// The report as printed: engine, accounts, writes/s (no decimals), p50
    /// and p99 in ms, jobs per commit with two decimals (`n/a` without stats),
    /// the filesystem, and a line with "warning" on a tmpfs run.
    pub fn render(&self) -> String {
        let mut sorted: Vec<f64> = self.latencies_ms.clone();
        sorted.sort_by(f64::total_cmp);
        let mut lines = vec![
            format!("engine:       {}", self.engine),
            format!(
                "accounts:     {} x {} writes",
                self.accounts, self.writes_per_account
            ),
            format!("writes/s:     {:.0}", self.writes_per_second()),
            format!("latency p50:  {:.2} ms", percentile(&sorted, 50.0)),
            format!("latency p99:  {:.2} ms", percentile(&sorted, 99.0)),
            format!("jobs/commit:  {}", self.jobs_per_commit()),
            format!(
                "filesystem:   {}",
                self.fs_type.as_deref().unwrap_or("unknown")
            ),
        ];
        if self.tmpfs_override {
            lines.push(format!(
                "warning: tmpfs makes fsync free, so these numbers hide the effect of group commit ({ALLOW_TMPFS_VAR}=1)"
            ));
        }
        lines.join("\n")
    }

    fn jobs_per_commit(&self) -> String {
        match self.stats {
            Some(stats) if stats.commits > 0 => {
                format!(
                    "{:.2} ({} commits)",
                    stats.jobs as f64 / stats.commits as f64,
                    stats.commits
                )
            }
            _ => "n/a".to_string(),
        }
    }
}

/// What the writers measured.
pub struct BenchRun {
    pub elapsed: Duration,
    pub latencies_ms: Vec<f64>,
    pub stats: Option<CommitStats>,
}

/// Create the accounts, then have every one write `writes` sessions at once,
/// each write waiting for its own commit. The commit counters are read as a
/// difference, so the account creation is not counted as batching.
pub async fn run_writers(
    engine: &Arc<dyn StorageEngine>,
    accounts: usize,
    writes: usize,
) -> anyhow::Result<BenchRun> {
    let mut backends: Vec<Arc<dyn Backend>> = Vec::with_capacity(accounts);
    for index in 0..accounts {
        let account = engine
            .create_account(Some(&format!("bench/{index}")))
            .await?;
        backends.push(engine.device_backend(account.device_id));
    }
    let before = engine.commit_stats();
    let started = Instant::now();
    let tasks: Vec<_> = backends
        .into_iter()
        .map(|backend| tokio::spawn(write_sessions(backend, writes)))
        .collect();
    let mut latencies_ms: Vec<f64> = Vec::with_capacity(accounts * writes);
    for task in tasks {
        latencies_ms.extend(task.await??);
    }
    let elapsed = started.elapsed();
    let stats = engine
        .commit_stats()
        .zip(before)
        .map(|(after, before)| CommitStats {
            commits: after.commits - before.commits,
            jobs: after.jobs - before.jobs,
        });
    Ok(BenchRun {
        elapsed,
        latencies_ms,
        stats,
    })
}

/// One account's writes, one after the other; the wait of each, in ms.
async fn write_sessions(backend: Arc<dyn Backend>, writes: usize) -> anyhow::Result<Vec<f64>> {
    let mut waits: Vec<f64> = Vec::with_capacity(writes);
    for write in 0..writes {
        let address = format!("peer{write}@s.whatsapp.net.0");
        let session: Vec<u8> = vec![write as u8; 256];
        let started = Instant::now();
        backend.put_session(&address, &session).await?;
        waits.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(waits)
}
