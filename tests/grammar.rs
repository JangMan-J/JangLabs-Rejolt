//! Conformance for the `grammar.toml` loader (P3, D22, D23, D3, A6).
//!
//! Three proofs:
//!   1. `g2_grammar_valid` — the load+validate check counts through the WP-0 G2
//!      harness (valid multi-facet grammar + empty seed accepted; every
//!      known-bad rejected).
//!   2. `known_bads_distinct_classifiable_errors` — each RB5 known-bad (fourth
//!      table, duplicate-facet, synonyms-only, bad version, missing version, bad
//!      placement, unknown field) is rejected with a distinct, exit-2-classified
//!      error; the serde-enforced cases (A6a closed set) are distinguished by
//!      the deserialization message that names the offending key/value.
//!   3. `goods_parse_and_digest` — the empty seed is valid and renders to the
//!      header alone; the multi-facet grammar renders every tag into the digest.

use std::fs;
use std::path::{Path, PathBuf};

use rejolt::conformance::{Check, assert_counts, fixtures_root};
use rejolt::grammar::{GrammarError, parse_and_validate, render_digest};

fn grammar_area() -> PathBuf {
    fixtures_root().join("grammar")
}

fn read(rel: &str) -> String {
    fs::read_to_string(fixtures_root().join(rel)).expect("read fixture")
}

/// G2 predicate: accepted iff the grammar parses AND validates.
fn grammar_accepts(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .map(|s| parse_and_validate(&s).is_ok())
        .unwrap_or(false)
}

#[test]
fn g2_grammar_valid() {
    let check = Check::new("grammar-load-valid", "grammar", grammar_accepts);
    assert_counts(&check, &fixtures_root());
}

#[test]
fn known_bads_distinct_classifiable_errors() {
    let bad = grammar_area().join("bad");
    let mut seen = 0;
    for entry in fs::read_dir(&bad).expect("read grammar bad dir") {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
        let contents = fs::read_to_string(&path).unwrap();
        let err = parse_and_validate(&contents)
            .expect_err(&format!("grammar bad fixture {stem} must be rejected"));
        assert_eq!(
            err.exit_code(),
            2,
            "`{stem}` must be exit-2 (config/taxonomy)"
        );
        let ok = match stem.as_str() {
            // A6a closed set: serde deserialization errors that NAME the cause.
            "fourth-table" => matches!(&err, GrammarError::Parse(m) if m.contains("platform")),
            "missing-version" => {
                matches!(&err, GrammarError::Parse(m) if m.contains("grammar-version"))
            }
            "bad-placement" => matches!(&err, GrammarError::Parse(m) if m.contains("sidecar")),
            "unknown-field" => matches!(&err, GrammarError::Parse(m) if m.contains("weight")),
            // Engine-side / value checks: distinct typed variants.
            "bad-version" => matches!(&err, GrammarError::UnsupportedVersion(2)),
            "duplicate-facet" => matches!(&err, GrammarError::DuplicateFacet { .. }),
            "synonyms-only" => matches!(&err, GrammarError::NoEvidence { .. }),
            "blank-evidence" => matches!(&err, GrammarError::InvalidEvidence { .. }),
            "newline-in-gloss" => matches!(&err, GrammarError::MultilineGloss { .. }),
            other => panic!("unmapped grammar bad fixture `{other}`"),
        };
        assert!(ok, "grammar bad `{stem}`: unexpected error {err:?} ({err})");
        seen += 1;
    }
    assert!(
        seen >= 9,
        "expected the full RB5 grammar reject corpus, saw {seen}"
    );
}

#[test]
fn goods_parse_and_digest() {
    let multi = parse_and_validate(&read("grammar/good/multifacet.toml")).unwrap();
    let digest = render_digest(&multi);
    for tag in ["gpu-tools", "ripgrep", "atomic-write"] {
        assert!(
            digest.contains(tag),
            "digest missing tag `{tag}`:\n{digest}"
        );
    }
    // Digest is rendered from parsed data — placement + gloss surface.
    assert!(digest.contains("[box]") && digest.contains("GPU and VRAM diagnostics"));

    let empty = parse_and_validate(&read("grammar/good/empty-seed.toml")).unwrap();
    assert!(empty.domain.is_empty() && empty.tool.is_empty() && empty.pattern.is_empty());
    assert_eq!(
        render_digest(&empty).lines().count(),
        1,
        "empty seed digest is the header alone"
    );
}
