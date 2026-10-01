//! Check results and the exit code they add up to (#64).
//!
//! Three outcomes, so a check that verified nothing never reads as a pass:
//! `Pass` asserted a returned value, `Accepted` means an RPC with nothing to
//! assert answered Ok, `Fail` is anything else. A report is green only with
//! zero failures AND at least one pass: an empty report, or one made only of
//! accepted calls, proved nothing.

use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Accepted,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub outcome: Outcome,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub pass: usize,
    pub accepted: usize,
    pub fail: usize,
}

#[derive(Debug, Default)]
pub struct Report {
    checks: Vec<Check>,
}

impl Report {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a check whose returned value was asserted.
    pub fn pass(&mut self, name: &str, detail: impl Into<String>) {
        self.record(name, Outcome::Pass, detail.into());
    }

    /// Record an RPC that answered Ok with nothing to assert.
    pub fn accepted(&mut self, name: &str, detail: impl Into<String>) {
        self.record(name, Outcome::Accepted, detail.into());
    }

    pub fn fail(&mut self, name: &str, detail: impl Into<String>) {
        self.record(name, Outcome::Fail, detail.into());
    }

    /// `pass` when `ok`, `fail` otherwise.
    pub fn verify(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        let outcome = if ok { Outcome::Pass } else { Outcome::Fail };
        self.record(name, outcome, detail.into());
    }

    /// For an RPC that returns nothing worth asserting: Ok is `accepted` and
    /// hands the response back, an error is `fail`.
    pub fn accepted_rpc<T>(&mut self, name: &str, result: Result<T, tonic::Status>) -> Option<T> {
        match result {
            Ok(value) => {
                self.accepted(name, "ok");
                Some(value)
            }
            Err(status) => {
                self.fail(name, format!("{}: {}", status.code(), status.message()));
                None
            }
        }
    }

    /// Printed as it is recorded, so a run that hangs still shows how far it got.
    fn record(&mut self, name: &str, outcome: Outcome, detail: String) {
        let check = Check {
            name: name.to_string(),
            outcome,
            detail,
        };
        println!("{}", check_line(&check));
        self.checks.push(check);
    }

    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for check in &self.checks {
            match check.outcome {
                Outcome::Pass => counts.pass += 1,
                Outcome::Accepted => counts.accepted += 1,
                Outcome::Fail => counts.fail += 1,
            }
        }
        counts
    }

    pub fn is_green(&self) -> bool {
        let counts = self.counts();
        counts.fail == 0 && counts.pass > 0
    }

    /// `summary: N pass, M accepted, K fail`.
    pub fn summary_line(&self) -> String {
        let Counts {
            pass,
            accepted,
            fail,
        } = self.counts();
        format!("summary: {pass} pass, {accepted} accepted, {fail} fail")
    }

    pub fn exit_code(&self) -> ExitCode {
        if self.is_green() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }

    /// Print the summary line (every check was already printed as it was
    /// recorded), then return the exit code.
    pub fn finish(self) -> ExitCode {
        println!("{}", self.summary_line());
        self.exit_code()
    }
}

/// One printed line per check: `[PASS] name :: detail`, `[ACCEPTED] ...`,
/// `[FAIL] ...`.
pub fn check_line(check: &Check) -> String {
    let tag = match check.outcome {
        Outcome::Pass => "PASS",
        Outcome::Accepted => "ACCEPTED",
        Outcome::Fail => "FAIL",
    };
    format!("[{tag}] {} :: {}", check.name, check.detail)
}
