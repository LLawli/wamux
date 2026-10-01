//! #64: the env contract and the exit code, proved on the built binaries.
//!
//! Each binary runs with a cleared environment and no daemon behind its
//! socket path, so a binary that validated its configuration late would fail
//! on the socket instead, and the assertion on the message catches it.

use std::process::{Output, Stdio};
use std::time::Duration;

mod support;
use support::TestDaemon;

const NO_DAEMON: &str = "/tmp/wmx64-no-daemon/wamux.sock";
// A port nothing listens on: the in-process binaries must refuse before this.
const NO_DATABASE: &str = "postgres://nobody:nobody@127.0.0.1:1/none";
const DEST: &str = "5500000000000@s.whatsapp.net";

/// Every binary that acts as one account (all but bench_client; stress_live
/// is feature-gated and checked by the grep gate instead).
const ACCOUNT_BINS: &[(&str, &str)] = &[
    ("backfill", env!("CARGO_BIN_EXE_backfill")),
    ("chat_state_live", env!("CARGO_BIN_EXE_chat_state_live")),
    ("e2e_all", env!("CARGO_BIN_EXE_e2e_all")),
    ("e2e_destructive", env!("CARGO_BIN_EXE_e2e_destructive")),
    (
        "interactive_reply_live",
        env!("CARGO_BIN_EXE_interactive_reply_live"),
    ),
    ("logout_e2e", env!("CARGO_BIN_EXE_logout_e2e")),
    (
        "newsletter_history_live",
        env!("CARGO_BIN_EXE_newsletter_history_live"),
    ),
    ("pair_backfill", env!("CARGO_BIN_EXE_pair_backfill")),
    ("pair_cli", env!("CARGO_BIN_EXE_pair_cli")),
    ("pair_socket", env!("CARGO_BIN_EXE_pair_socket")),
    ("poll_live", env!("CARGO_BIN_EXE_poll_live")),
    ("read_receipt_live", env!("CARGO_BIN_EXE_read_receipt_live")),
    ("recv_media", env!("CARGO_BIN_EXE_recv_media")),
    ("send_echo_live", env!("CARGO_BIN_EXE_send_echo_live")),
    ("send_types", env!("CARGO_BIN_EXE_send_types")),
    ("set_pfp", env!("CARGO_BIN_EXE_set_pfp")),
];

/// Every binary that writes to someone else's chat.
const SENDING_BINS: &[(&str, &str)] = &[
    ("e2e_all", env!("CARGO_BIN_EXE_e2e_all")),
    ("e2e_destructive", env!("CARGO_BIN_EXE_e2e_destructive")),
    ("poll_live", env!("CARGO_BIN_EXE_poll_live")),
    ("read_receipt_live", env!("CARGO_BIN_EXE_read_receipt_live")),
    ("send_echo_live", env!("CARGO_BIN_EXE_send_echo_live")),
    ("send_types", env!("CARGO_BIN_EXE_send_types")),
];

/// What every run gets unless the case removes it: a full, valid config for
/// any binary, so the one missing piece is the only thing that can fail.
fn base_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("WAMUX_SOCKET_PATH", NO_DAEMON),
        ("DATABASE_URL", NO_DATABASE),
        ("WAMUX_REF", "tools-64"),
        ("WAMUX_PEER_REF", "tools-64-peer"),
        ("WAMUX_LIVE_DEST", DEST),
        ("WAMUX_E2E_DESTRUCTIVE", "yes"),
    ]
}

fn without(
    vars: Vec<(&'static str, &'static str)>,
    drop: &str,
) -> Vec<(&'static str, &'static str)> {
    vars.into_iter().filter(|(k, _)| *k != drop).collect()
}

fn with(
    vars: Vec<(&'static str, &'static str)>,
    key: &'static str,
    value: &'static str,
) -> Vec<(&'static str, &'static str)> {
    let mut vars = without(vars, key);
    vars.push((key, value));
    vars
}

async fn run(exe: &str, vars: &[(&str, &str)]) -> Output {
    let child = tokio::process::Command::new(exe)
        .env_clear()
        .envs(vars.iter().copied())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(Duration::from_secs(60), child)
        .await
        .unwrap_or_else(|_| panic!("{exe} did not exit within 60s"))
        .unwrap_or_else(|e| panic!("cannot run {exe}: {e}"))
}

fn assert_refused(name: &str, out: &Output, must_mention: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "{name} must exit non-zero; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains(must_mention),
        "{name} must say {must_mention} on stderr, got:\n{stderr}"
    );
}

#[tokio::test]
async fn every_account_binary_refuses_to_start_without_wamux_ref() {
    let vars = without(base_env(), "WAMUX_REF");
    for (name, exe) in ACCOUNT_BINS {
        assert_refused(name, &run(exe, &vars).await, "WAMUX_REF");
    }
}

#[tokio::test]
async fn every_sending_binary_refuses_to_start_without_a_destination() {
    let vars = without(base_env(), "WAMUX_LIVE_DEST");
    for (name, exe) in SENDING_BINS {
        assert_refused(name, &run(exe, &vars).await, "WAMUX_LIVE_DEST");
    }
}

#[tokio::test]
async fn every_sending_binary_refuses_the_legacy_c_us_server() {
    let vars = with(base_env(), "WAMUX_LIVE_DEST", "5500000000000@c.us");
    for (name, exe) in SENDING_BINS {
        assert_refused(name, &run(exe, &vars).await, "@c.us");
    }
}

#[tokio::test]
async fn interactive_reply_needs_a_destination_only_to_answer() {
    let exe = env!("CARGO_BIN_EXE_interactive_reply_live");
    let vars = with(without(base_env(), "WAMUX_LIVE_DEST"), "WAMUX_REPLY", "1");
    assert_refused(
        "interactive_reply_live",
        &run(exe, &vars).await,
        "WAMUX_LIVE_DEST",
    );
}

#[tokio::test]
async fn chat_state_live_refuses_to_start_without_its_peer_account() {
    let exe = env!("CARGO_BIN_EXE_chat_state_live");
    let vars = without(base_env(), "WAMUX_PEER_REF");
    assert_refused("chat_state_live", &run(exe, &vars).await, "WAMUX_PEER_REF");
}

#[tokio::test]
async fn e2e_destructive_refuses_without_the_explicit_yes() {
    let exe = env!("CARGO_BIN_EXE_e2e_destructive");
    for vars in [
        without(base_env(), "WAMUX_E2E_DESTRUCTIVE"),
        with(base_env(), "WAMUX_E2E_DESTRUCTIVE", "1"),
    ] {
        assert_refused(
            "e2e_destructive",
            &run(exe, &vars).await,
            "WAMUX_E2E_DESTRUCTIVE",
        );
    }
}

#[tokio::test]
async fn e2e_all_exits_non_zero_when_a_check_fails() {
    let daemon = TestDaemon::start().await;
    let socket = daemon.socket_str();
    let mut vars = with(base_env(), "WAMUX_REF", "tools-64-nobody");
    vars.retain(|(k, _)| *k != "WAMUX_SOCKET_PATH");
    let mut vars: Vec<(&str, &str)> = vars;
    vars.push(("WAMUX_SOCKET_PATH", socket.as_str()));

    let out = run(env!("CARGO_BIN_EXE_e2e_all"), &vars).await;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "a failed check must fail the run; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Checks really ran against the daemon before the failing one...
    assert!(stdout.contains("[PASS] Account.CreateAccount"), "{stdout}");
    assert!(stdout.contains("[PASS] Account.DeleteAccount"), "{stdout}");
    // ...the unknown account is the recorded failure, and nothing was sent.
    assert!(stdout.contains("[FAIL] Account.ConnectAccount"), "{stdout}");
    assert!(
        !stdout.contains("Messaging."),
        "nothing may be sent: {stdout}"
    );
    assert!(stdout.contains("summary: "), "{stdout}");
    assert!(stdout.contains(" fail"), "{stdout}");
    // The throwaway account of the lifecycle checks is gone again.
    let leftovers: Vec<String> = daemon
        .registry
        .list()
        .iter()
        .filter_map(|h| h.external_ref.clone())
        .filter(|r| r.starts_with("e2e-tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "leaked: {leftovers:?}");
}
