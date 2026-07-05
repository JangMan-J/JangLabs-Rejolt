//! Conformance for the bespoke frontmatter dialect parser (P2, D21, A3).
//!
//! Four proofs, one per oracle discipline in plan Appendix B2 plus the G2 gate:
//!   1. `g2_frontmatter_parse_valid` — the parse/validate check counts through
//!      the WP-0 G2 harness (every good accepted, every bad rejected).
//!   2. `vector_corpus_bad_named_errors` — the committed expected-value vector
//!      corpus (portable fallback oracle): each known-bad fixture is rejected
//!      with the SPECIFIC named error whose B2/D21 rule it violates (RB3).
//!   3. `round_trip_all_goods` — generate→parse→regenerate is stable on every
//!      in-subset vector.
//!   4. `differential_pyyaml_over_goods` — differential agreement with pinned
//!      PyYAML `safe_load`, out-of-process, over the in-subset corpus; gated on
//!      python3+PyYAML, skipped-with-notice when absent (N10 untouched: nothing
//!      but the static binary ever runs on an engine path).

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use rejolt::conformance::{Check, assert_counts, fixtures_root};
use rejolt::frontmatter::{FrontmatterError, canonical_json, frontmatter_block, generate, parse};

fn fm_area() -> PathBuf {
    fixtures_root().join("frontmatter")
}

fn read(rel: &str) -> String {
    fs::read_to_string(fixtures_root().join(rel)).expect("read fixture")
}

fn good_fixtures() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(fm_area().join("good"))
        .expect("read good dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    v
}

/// G2 predicate: a fixture is accepted iff the full parse + D21 schema validates.
fn fm_accepts(path: &std::path::Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .map(|s| parse(&s).is_ok())
        .unwrap_or(false)
}

#[test]
fn g2_frontmatter_parse_valid() {
    let check = Check::new("frontmatter-parse-valid", "frontmatter", fm_accepts);
    assert_counts(&check, &fixtures_root());
}

#[test]
fn vector_corpus_bad_named_errors() {
    let bad_dir = fm_area().join("bad");
    let mut seen = 0;
    for entry in fs::read_dir(&bad_dir).expect("read bad dir") {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
        let contents = fs::read_to_string(&path).unwrap();
        let err = parse(&contents).expect_err(&format!("bad fixture {stem} must be rejected"));
        let ok = match stem.as_str() {
            "no-opening-fence" => matches!(err, FrontmatterError::MissingOpeningFence),
            "no-closing-fence" => matches!(err, FrontmatterError::MissingClosingFence),
            "anchor" => matches!(err, FrontmatterError::Anchor { .. }),
            "alias" => matches!(err, FrontmatterError::Alias { .. }),
            "type-tag" => matches!(err, FrontmatterError::TypeTag { .. }),
            "multi-document" => matches!(err, FrontmatterError::MultiDocument { .. }),
            "block-scalar" => matches!(err, FrontmatterError::BlockScalar { .. }),
            "flow-mapping" => matches!(err, FrontmatterError::FlowMapping { .. }),
            "flow-brace" => matches!(err, FrontmatterError::FlowMapping { .. }),
            "multiline-string" => matches!(err, FrontmatterError::MultilineString { .. }),
            "inline-comment" => matches!(err, FrontmatterError::InlineComment { .. }),
            "colon-space-value" => matches!(err, FrontmatterError::ColonInScalar { .. }),
            "bad-escape" => matches!(err, FrontmatterError::InvalidEscape { .. }),
            "tab-indentation" => matches!(err, FrontmatterError::TabIndentation { .. }),
            "duplicate-key" => matches!(err, FrontmatterError::DuplicateKey { .. }),
            "top-level-triggers" => matches!(err, FrontmatterError::TopLevelTriggers { .. }),
            "unknown-top-level-key" => matches!(err, FrontmatterError::UnknownTopLevelKey { .. }),
            "unknown-metadata-key" => matches!(err, FrontmatterError::UnknownMetadataKey { .. }),
            "unknown-trigger-key" => matches!(err, FrontmatterError::UnknownTriggerKey { .. }),
            "missing-tags" => matches!(err, FrontmatterError::MissingTags),
            "empty-tags" => matches!(err, FrontmatterError::EmptyTags),
            "invalid-tag" => matches!(err, FrontmatterError::InvalidTag { .. }),
            other => panic!("unmapped bad fixture `{other}` — add it to the vector corpus"),
        };
        assert!(ok, "fixture `{stem}`: unexpected error {err:?} ({err})");
        seen += 1;
    }
    assert!(
        seen >= 22,
        "expected the full B2 rejection corpus, saw {seen}"
    );
}

#[test]
fn vector_corpus_good_values() {
    let fm = parse(&read("frontmatter/good/worked-example.md")).unwrap();
    assert_eq!(fm.name.as_deref(), Some("gpu-notes"));
    assert_eq!(
        fm.description.as_deref(),
        Some("GPU memory and VRAM diagnostics")
    );
    assert_eq!(
        fm.metadata.tags,
        vec!["gpu".to_string(), "vram".to_string()]
    );
    let tr = fm.metadata.triggers.expect("triggers present");
    assert_eq!(tr.commands, vec!["nvidia-smi".to_string()]);
    assert_eq!(
        tr.paths,
        vec!["~/.config/gpu/**".to_string(), "**/*.md".to_string()]
    );
    assert_eq!(tr.args, vec!["--no-cache".to_string()]);

    let ranked = parse(&read("frontmatter/good/ranking-fields.md")).unwrap();
    assert_eq!(ranked.metadata.decline_count, Some(2));
    assert_eq!(ranked.metadata.last_reviewed.as_deref(), Some("2026-07-04"));

    let minimal = parse(&read("frontmatter/good/minimal.md")).unwrap();
    assert!(minimal.name.is_none());
    assert!(minimal.metadata.triggers.is_none());
    assert_eq!(minimal.metadata.tags, vec!["solo".to_string()]);

    // FIX 1/2 literals: mid-scalar braces are kept, and a `#` not preceded by
    // whitespace is a literal (not an inline comment).
    let braced = parse(&read("frontmatter/good/braced-plain.md")).unwrap();
    assert_eq!(
        braced.description.as_deref(),
        Some("expands {HOME} at runtime")
    );
    let btr = braced.metadata.triggers.expect("triggers present");
    assert_eq!(btr.args, vec!["a{b}".to_string()]);
    assert_eq!(btr.paths, vec!["/foo#bar".to_string()]);
    assert_eq!(btr.synonyms, vec!["c#".to_string(), "f#".to_string()]);
}

#[test]
fn round_trip_all_goods() {
    for path in good_fixtures() {
        let contents = fs::read_to_string(&path).unwrap();
        let p1 = parse(&contents).unwrap_or_else(|e| panic!("parse {path:?}: {e}"));
        let g1 = generate(&p1);
        let p2 = parse(&g1).unwrap_or_else(|e| panic!("reparse generated {path:?}: {e}\n{g1}"));
        assert_eq!(p1, p2, "round-trip parse mismatch for {path:?}");
        assert_eq!(g1, generate(&p2), "round-trip regen mismatch for {path:?}");
    }
}

#[test]
fn differential_pyyaml_over_goods() {
    if !pyyaml_available() {
        eprintln!(
            "SKIP differential_pyyaml_over_goods: python3 + PyYAML unavailable; \
             the committed vector corpus (vector_corpus_* tests) is the fallback oracle"
        );
        return;
    }
    for path in good_fixtures() {
        let contents = fs::read_to_string(&path).unwrap();
        let block = frontmatter_block(&contents).unwrap();
        let py = pyyaml_canonical(&block);
        let mine = canonical_json(&contents).unwrap();
        let py_val: serde_json::Value = serde_json::from_str(&py).expect("parse pyyaml json");
        let my_val: serde_json::Value = serde_json::from_str(&mine).expect("parse rejolt json");
        assert_eq!(my_val, py_val, "differential mismatch for {path:?}");
    }
}

fn pyyaml_available() -> bool {
    Command::new("python3")
        .args(["-c", "import yaml"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Load the frontmatter block through PyYAML `safe_load`, stringify every scalar
/// (sidestepping YAML implicit typing), and emit sorted-key JSON — the reference
/// side of the differential oracle.
fn pyyaml_canonical(block: &str) -> String {
    const SCRIPT: &str = r#"
import sys, json, yaml
def s(x):
    if isinstance(x, dict): return {str(k): s(v) for k, v in x.items()}
    if isinstance(x, list): return [s(v) for v in x]
    if x is None: return None
    return str(x)
print(json.dumps(s(yaml.safe_load(sys.stdin.read())), sort_keys=True))
"#;
    let mut child = Command::new("python3")
        .args(["-c", SCRIPT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python3");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(block.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("python3 output");
    assert!(
        out.status.success(),
        "pyyaml failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
