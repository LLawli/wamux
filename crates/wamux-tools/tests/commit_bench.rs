//! #172: `commit_bench`. The pure parts (the mount lookup, the tmpfs refusal,
//! the percentiles, the report) are checked on known inputs; one small run of
//! the real binary on SQLite checks that it measures and reports.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use wamux_tools::commit_bench::{
    ALLOW_TMPFS_VAR, BenchReport, fs_type_of, percentile, tmpfs_refusal,
};

const MOUNTS: &str = "\
/dev/mapper/root / btrfs rw,relatime 0 0
tmpfs /tmp tmpfs rw,nosuid,nodev 0 0
/dev/mapper/root /home btrfs rw,relatime,subvol=/@home 0 0
/dev/mapper/dados /mnt/dados btrfs rw,compress=zstd 0 0
proc /proc proc rw 0 0
";

#[test]
fn fs_type_is_read_from_the_longest_mount_prefix() {
    let of = |path: &str| fs_type_of(Path::new(path), MOUNTS);
    assert_eq!(of("/tmp/bench/wamux.db").as_deref(), Some("tmpfs"));
    assert_eq!(of("/home/luka/.cache/wamux.db").as_deref(), Some("btrfs"));
    assert_eq!(of("/mnt/dados/builds/wamux.db").as_deref(), Some("btrfs"));
    // A whole component, not a string prefix: /tmpfoo is not under /tmp.
    assert_eq!(of("/tmpfoo/wamux.db").as_deref(), Some("btrfs"));
    assert_eq!(of("/var/lib/wamux/wamux.db").as_deref(), Some("btrfs"));
    assert_eq!(fs_type_of(Path::new("/x/wamux.db"), ""), None);
}

#[test]
fn tmpfs_is_refused_without_the_override() {
    let refused = tmpfs_refusal(Some("tmpfs"), false).expect("tmpfs is refused");
    assert!(refused.contains("tmpfs"), "{refused}");
    assert!(
        refused.contains(ALLOW_TMPFS_VAR),
        "names the override: {refused}"
    );
    assert_eq!(tmpfs_refusal(Some("tmpfs"), true), None);
    assert_eq!(tmpfs_refusal(Some("btrfs"), false), None);
    assert_eq!(
        tmpfs_refusal(None, false),
        None,
        "an unknown filesystem runs"
    );
}

#[test]
fn percentiles_of_a_known_sample() {
    let sample: Vec<f64> = (1..=100).map(f64::from).collect();
    assert_eq!(percentile(&sample, 50.0), 50.0);
    assert_eq!(percentile(&sample, 99.0), 99.0);
    assert_eq!(percentile(&sample, 100.0), 100.0);
    assert_eq!(percentile(&sample, 0.0), 1.0);
    assert_eq!(percentile(&[7.0], 99.0), 7.0);
}

fn report(stats: Option<wamux::storage::CommitStats>, tmpfs_override: bool) -> BenchReport {
    BenchReport {
        engine: "turso".into(),
        accounts: 10,
        writes_per_account: 30,
        elapsed: Duration::from_millis(150),
        latencies_ms: (1..=300).map(|ms| f64::from(ms) / 10.0).collect(),
        stats,
        fs_type: Some(if tmpfs_override { "tmpfs" } else { "btrfs" }.into()),
        tmpfs_override,
    }
}

#[test]
fn the_report_shows_rate_latency_batching_and_the_tmpfs_warning() {
    let stats = wamux::storage::CommitStats {
        commits: 50,
        jobs: 300,
    };
    let disk = report(Some(stats), false);
    assert!((disk.writes_per_second() - 2000.0).abs() < 1e-6);
    let shown = disk.render();
    for expected in ["turso", "10", "2000", "p50", "p99", "6.00", "btrfs"] {
        assert!(
            shown.contains(expected),
            "{expected:?} missing in:\n{shown}"
        );
    }
    assert!(!shown.to_lowercase().contains("warning"), "{shown}");

    let tmpfs = report(None, true).render();
    assert!(tmpfs.contains("n/a"), "no stats, no ratio:\n{tmpfs}");
    assert!(
        tmpfs.to_lowercase().contains("warning") && tmpfs.contains("tmpfs"),
        "a tmpfs run is marked:\n{tmpfs}"
    );
}

/// Run the binary, killing it and failing after a minute.
fn run_bounded(command: &mut Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn commit_bench");
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("commit_bench did not finish in a minute");
        }
        // not a sync point: waiting for an external process to exit, polled with a deadline
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn a_small_run_reports_rate_and_latency() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("bench.db").display());
    let output = run_bounded(
        Command::new(env!("CARGO_BIN_EXE_commit_bench"))
            .args([url.as_str(), "3", "5"])
            .env(ALLOW_TMPFS_VAR, "1"),
    );
    let shown = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{shown}");
    for expected in ["sqlite", "writes/s", "p50", "p99", "n/a"] {
        assert!(
            shown.contains(expected),
            "{expected:?} missing in:\n{shown}"
        );
    }
}
