//! §3.x path-specificity conformance (WP-4; D5, §3.x, §6, §7).
//!
//! Every §3.x example is a G2 row: one fixture file per example under
//! `fixtures/path-specificity/{good,bad}/`, `good/` = the "must be specific" list,
//! `bad/` = the "must be broad" list. The check `path-specificity` accepts iff the
//! fixture's path is SPECIFIC (`is_broad_path == false`) — so it accepts every
//! specific example and rejects every broad one, and its verdict counts (G2).
//!
//! The classifier is the SAME `is_broad_path` the write-guard static gate (§6) and
//! the collision liveness (§7) call — §3.x's whole point.

use std::fs;
use std::path::Path;

use rejolt::conformance::{Check, assert_counts, fixtures_root};
use rejolt::path_class::is_broad_path;

/// G2 predicate: read the fixture's path (first non-empty line) and accept iff it
/// classifies as SPECIFIC. `good/` fixtures (specific paths) are accepted; `bad/`
/// fixtures (broad paths) are rejected.
fn path_is_specific(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let pat = text.lines().next().unwrap_or("").trim();
    !is_broad_path(pat)
}

#[test]
fn g2_path_specificity() {
    let check = Check::new("path-specificity", "path-specificity", path_is_specific);
    assert_counts(&check, &fixtures_root());
}

/// Belt-and-suspenders: every fixture in `bad/` is broad and every fixture in
/// `good/` is specific, asserted per-file so a single wrong example is named.
#[test]
fn every_fixture_example_classified_exactly() {
    let root = fixtures_root().join("path-specificity");
    let mut broad = 0;
    for entry in fs::read_dir(root.join("bad")).expect("read bad dir") {
        let p = entry.unwrap().path();
        let pat = fs::read_to_string(&p).unwrap();
        let pat = pat.lines().next().unwrap_or("").trim();
        assert!(is_broad_path(pat), "§3.x: `{pat}` ({p:?}) MUST be broad");
        broad += 1;
    }
    let mut specific = 0;
    for entry in fs::read_dir(root.join("good")).expect("read good dir") {
        let p = entry.unwrap().path();
        let pat = fs::read_to_string(&p).unwrap();
        let pat = pat.lines().next().unwrap_or("").trim();
        assert!(
            !is_broad_path(pat),
            "§3.x: `{pat}` ({p:?}) MUST be specific"
        );
        specific += 1;
    }
    assert_eq!(broad, 17, "the full §3.x broad list");
    assert_eq!(specific, 8, "the full §3.x specific list");
}
