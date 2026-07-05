//! G2 conformance self-test harness (`WORKFLOW.md` §6, gate **G2**).
//!
//! > A verifier or harness's verdicts do not count until it has **failed a
//! > known-bad fixture and passed a known-good one.**
//!
//! This module is the reusable scaffold every later packet plugs its
//! conformance checks into. A [`Check`] pairs a named predicate with a fixture
//! *area* under `fixtures/<area>/{good,bad}`. [`run_check`] enforces the G2
//! discipline: a check whose verdict is allowed to *count* must have proven it
//! accepts at least one known-good fixture **and** rejects at least one
//! known-bad fixture. A check that declares no known-bad (or no known-good)
//! fixture is an [`Verdict::Undisciplined`] violation — its verdict does not
//! count. A check that *accepts* a known-bad fixture (a rubber-stamp that would
//! pass anything) is caught as a [`Verdict::WrongAnswer`].
//!
//! The predicate convention is: **`true` = the check ACCEPTS the fixture as
//! conformant, `false` = the check REJECTS it.** Good fixtures must be
//! accepted; bad fixtures must be rejected.

use std::fs;
use std::path::{Path, PathBuf};

/// Which side of a check a fixture is expected to land on. Maps directly to the
/// `good/` and `bad/` subdirectories of a fixture area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// A conformant fixture the check MUST accept.
    Good,
    /// A non-conformant fixture the check MUST reject.
    Bad,
}

impl Polarity {
    fn subdir(self) -> &'static str {
        match self {
            Polarity::Good => "good",
            Polarity::Bad => "bad",
        }
    }
}

/// A single conformance check: a named predicate over a fixture, drawing its
/// known-good and known-bad fixtures from `fixtures/<area>/{good,bad}`.
///
/// The predicate returns `true` when the check ACCEPTS a fixture as conformant
/// and `false` when it REJECTS it.
pub struct Check {
    name: String,
    area: String,
    predicate: Box<dyn Fn(&Path) -> bool>,
}

impl Check {
    /// Declare a check named `name`, drawing fixtures from area `area`, with
    /// acceptance decided by `predicate`.
    pub fn new(
        name: impl Into<String>,
        area: impl Into<String>,
        predicate: impl Fn(&Path) -> bool + 'static,
    ) -> Self {
        Check {
            name: name.into(),
            area: area.into(),
            predicate: Box::new(predicate),
        }
    }

    /// The check's name (for diagnostics).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The fixture area this check draws from.
    pub fn area(&self) -> &str {
        &self.area
    }
}

/// The outcome of running a [`Check`] against its fixtures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The check's verdict **counts**: it accepted at least one known-good
    /// fixture and rejected at least one known-bad fixture, with no wrong
    /// answers.
    Counts,
    /// The verdict does **not** count: the check lacks a known-good or a
    /// known-bad fixture, so G2 has not been satisfied. The string names the
    /// missing side.
    Undisciplined(String),
    /// The check ran against a disciplined fixture set but gave a wrong answer
    /// on one or more fixtures: it rejected a known-good, or accepted a
    /// known-bad (a rubber-stamp). The vector lists every offending fixture.
    WrongAnswer(Vec<String>),
}

impl Verdict {
    /// `true` iff this verdict is [`Verdict::Counts`].
    pub fn counts(&self) -> bool {
        matches!(self, Verdict::Counts)
    }
}

/// The repo `fixtures/` directory, resolved from the crate manifest dir at
/// compile time so tests and packets need not compute it.
pub fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Run `check` against the fixtures rooted at `fixtures_root`, returning the G2
/// [`Verdict`]. See the module docs for the discipline this enforces.
pub fn run_check(check: &Check, fixtures_root: &Path) -> Verdict {
    let good = match list_fixtures(check.area(), fixtures_root, Polarity::Good) {
        Ok(paths) => paths,
        Err(err) => return Verdict::Undisciplined(err),
    };
    let bad = match list_fixtures(check.area(), fixtures_root, Polarity::Bad) {
        Ok(paths) => paths,
        Err(err) => return Verdict::Undisciplined(err),
    };

    if good.is_empty() {
        return Verdict::Undisciplined(format!(
            "check `{}` has no known-good fixture under {}/good/",
            check.name(),
            check.area()
        ));
    }
    if bad.is_empty() {
        return Verdict::Undisciplined(format!(
            "check `{}` has no known-bad fixture under {}/bad/",
            check.name(),
            check.area()
        ));
    }

    let mut failures = Vec::new();
    for path in &good {
        if !(check.predicate)(path) {
            failures.push(format!("known-good fixture rejected: {}", path.display()));
        }
    }
    for path in &bad {
        if (check.predicate)(path) {
            failures.push(format!(
                "known-bad fixture accepted (rubber-stamp): {}",
                path.display()
            ));
        }
    }

    if failures.is_empty() {
        Verdict::Counts
    } else {
        Verdict::WrongAnswer(failures)
    }
}

/// Assert that `check`'s verdict counts under `fixtures_root`, panicking with
/// the offending [`Verdict`] otherwise. The one-line helper conformance rows
/// in later packets call.
pub fn assert_counts(check: &Check, fixtures_root: &Path) {
    let verdict = run_check(check, fixtures_root);
    assert!(
        verdict.counts(),
        "G2: check `{}` (area `{}`) verdict does not count: {verdict:?}",
        check.name(),
        check.area()
    );
}

/// List the regular files in `fixtures_root/<area>/<good|bad>/`, sorted for
/// determinism. A missing directory is treated as "no fixtures" (empty), which
/// [`run_check`] then reports as an `Undisciplined` violation; genuine I/O
/// errors on an existing directory are surfaced as the `Err` string.
fn list_fixtures(
    area: &str,
    fixtures_root: &Path,
    polarity: Polarity,
) -> Result<Vec<PathBuf>, String> {
    let dir = fixtures_root.join(area).join(polarity.subdir());
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read entry in {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}
