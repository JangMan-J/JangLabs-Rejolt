//! G2 self-test: prove the conformance harness itself works (G2 applied to the
//! harness). Drives `rejolt::conformance` exactly as a later packet would.
//!
//! Three proofs:
//!   1. a real check (`nonempty-file`) passes its known-good and rejects its
//!      known-bad → verdict COUNTS;
//!   2. a rubber-stamp check (accepts everything) is CAUGHT — it accepts the
//!      known-bad, so the harness returns `WrongAnswer` rather than `Counts`;
//!   3. a check that declares no known-bad fixture is flagged as a discipline
//!      violation (`Undisciplined`) — its verdict does not count.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::conformance::{Check, Verdict, assert_counts, fixtures_root, run_check};

/// Reference check predicate: a fixture file is conformant iff it is non-empty.
fn nonempty(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false)
}

#[test]
fn harness_passes_known_good_and_rejects_known_bad() {
    // good/nonempty.txt (31 bytes) -> accepted; bad/empty.txt (0 bytes) -> rejected.
    let check = Check::new("nonempty-file", "selftest", nonempty);
    assert_counts(&check, &fixtures_root());
}

#[test]
fn harness_catches_rubber_stamp_check() {
    // A check that accepts EVERYTHING would rubber-stamp the known-bad empty
    // fixture. The harness must catch that as a WrongAnswer, never Counts —
    // this is the proof the scaffold catches a check that fails to reject its
    // known-bad.
    let rubber_stamp = Check::new("always-accept", "selftest", |_path| true);
    let verdict = run_check(&rubber_stamp, &fixtures_root());

    assert!(
        !verdict.counts(),
        "a rubber-stamp check must not be allowed to count, got {verdict:?}"
    );
    match verdict {
        Verdict::WrongAnswer(failures) => assert!(
            failures.iter().any(|f| f.contains("rubber-stamp")),
            "expected a rubber-stamp failure listed, got {failures:?}"
        ),
        other => panic!("expected WrongAnswer for a rubber-stamp check, got {other:?}"),
    }
}

#[test]
fn harness_flags_check_missing_known_bad() {
    // A declared check whose area has a known-good but NO known-bad fixture is a
    // G2 discipline violation: its verdict does not count.
    let root = make_temp_fixtures_root();
    let good_dir = root.join("orphan").join("good");
    fs::create_dir_all(&good_dir).expect("create temp good dir");
    fs::write(good_dir.join("present.txt"), b"content\n").expect("write temp fixture");
    // Deliberately no orphan/bad/ directory.

    let check = Check::new("nonempty-file", "orphan", nonempty);
    let verdict = run_check(&check, &root);

    match &verdict {
        Verdict::Undisciplined(reason) => assert!(
            reason.contains("known-bad"),
            "expected a missing-known-bad reason, got {reason:?}"
        ),
        other => panic!("expected Undisciplined (missing known-bad), got {other:?}"),
    }
    assert!(!verdict.counts(), "an undisciplined check must not count");

    let _ = fs::remove_dir_all(&root);
}

/// A unique temp dir for a discipline test, without pulling in a temp-file
/// crate (WP-0 keeps the dep tree minimal).
fn make_temp_fixtures_root() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rejolt-g2-selftest-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp fixtures root");
    dir
}
