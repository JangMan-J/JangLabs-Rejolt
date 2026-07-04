//! The legacy fence (WP-8c / P18; D15, D17, D18; risk RB7).
//!
//! D17 dropped legacy import entirely; D18 removed fallback-trigger derivation;
//! D15 makes every wire format clean-slate (synapse is cited evidence, never a
//! constraint). This file is the mechanical proof those decisions hold, and the
//! regression gate if anyone ever tries to grow an import path back in: every
//! test here reads the actual `src/` tree (not a fixture, not a description of
//! it) and asserts a specific forbidden surface is absent. A future PR that
//! reintroduces any of these trips a test in this file, not a review comment.
//!
//! Only **full-line** comments (`//`, `///`, `//!` after trimming leading
//! whitespace) are stripped before scanning. `bootstrap.rs`'s module doc
//! legitimately *names* `--import-legacy` to document that D17 voided it; that
//! mention lives entirely on such a line and must not count as a hit. Nothing
//! else in the tree is exempted — inline trailing comments are scanned as-is,
//! which is intentionally strict (an identifier reintroduced even inside a
//! comment is worth a human look).
//!
//! The one deliberately narrow exception is `_grammar.md`: it is a legitimate
//! *filename* the infra-file classifier (`is_infra`, `src/rebuild.rs`) must
//! recognize and skip during a store scan, so the literal string is allowed —
//! but only as an argument to `is_infra(`. Any other appearance (something
//! that would open, read, or parse it as a legacy markdown grammar) fails the
//! check.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The engine `src/` tree, resolved from the crate manifest dir at compile
/// time (mirrors `rejolt::conformance::fixtures_root`'s pattern for `fixtures/`).
fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src/`, recursively (the tree is flat today, but the
/// walk does not assume that).
fn src_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("read src dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&src_root(), &mut out);
    out.sort();
    assert!(!out.is_empty(), "expected to find .rs files under src/");
    out
}

/// Lines of `path`'s contents with full-line comments (`//`, `///`, `//!`,
/// after trimming leading whitespace) dropped. Trailing inline comments on a
/// code line are NOT stripped — see the module doc for why that's the
/// intentionally strict choice here.
fn code_lines(path: &Path) -> Vec<String> {
    let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    src.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .map(str::to_owned)
        .collect()
}

/// The full `src/` tree as one blob of code (full-line comments dropped),
/// for whole-tree substring checks that don't need per-line context.
fn all_code() -> String {
    let mut blob = String::new();
    for path in src_files() {
        for line in code_lines(&path) {
            blob.push_str(&line);
            blob.push('\n');
        }
    }
    blob
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rejolt")
}

// ---------------------------------------------------------------------------
// D17: no legacy import.
// ---------------------------------------------------------------------------

#[test]
fn no_import_legacy_flag_or_handling_in_source() {
    let code = all_code();
    assert!(
        !code.contains("import-legacy") && !code.contains("import_legacy"),
        "found `--import-legacy` surface in non-comment source (D17: void)"
    );
    // Broader net: no CLI flag/field named anything import-shaped should exist
    // on the bootstrap surface (the only D17-relevant subcommand).
    assert!(
        !code.contains("ImportLegacy"),
        "found an `ImportLegacy`-named type/variant (D17: void)"
    );
}

#[test]
fn bootstrap_cli_help_has_no_import_legacy_flag() {
    // Confirms the *built binary*, not just the struct literal, never
    // advertises the flag (task instruction: check via `--help`).
    let output = Command::new(bin())
        .args(["bootstrap", "--help"])
        .output()
        .expect("run `rejolt bootstrap --help`");
    assert!(output.status.success(), "`bootstrap --help` should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.to_lowercase().contains("legacy"),
        "`bootstrap --help` mentions legacy import; D17 dropped it entirely:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// D18: no fallback trigger derivation.
// ---------------------------------------------------------------------------

#[test]
fn no_derive_fallback_triggers_producer() {
    let code = all_code();
    assert!(
        !code.contains("derive_fallback_triggers") && !code.contains("deriveFallbackTriggers"),
        "found a `derive_fallback_triggers` producer (D18: removed, every route is declared)"
    );
}

#[test]
fn no_memory_derived_source_producer() {
    let code = all_code();
    assert!(
        !code.contains("memory-derived") && !code.contains("memory_derived"),
        "found a `source = memory-derived` route producer (D18: removed)"
    );
}

#[test]
fn no_by_memory_id_producer() {
    let code = all_code();
    assert!(
        !code.contains("byMemoryId"),
        "found a `byMemoryId` table/producer (D18's fallback lifecycle; P5 dropped the table)"
    );
}

// ---------------------------------------------------------------------------
// D15: wire formats are clean-slate; no legacy-format parsing anywhere.
// ---------------------------------------------------------------------------

#[test]
fn no_tags_md_parser() {
    let code = all_code();
    assert!(
        !code.contains("_tags.md"),
        "found a reference to the legacy `_tags.md` file (D15: clean-slate wire formats)"
    );
    assert!(
        !code.contains("parse_tags_md"),
        "found a `parse_tags_md` function (D15: no legacy-format parsing)"
    );
}

#[test]
fn no_tag_links_md_parser() {
    let code = all_code();
    assert!(
        !code.contains("_tag_links.md"),
        "found a reference to the legacy `_tag_links.md` file (D15: clean-slate wire formats)"
    );
    assert!(
        !code.contains("parse_tag_links"),
        "found a `parse_tag_links` function (D15: no legacy-format parsing)"
    );
}

#[test]
fn no_legacy_grammar_md_reader() {
    // `_grammar.md` may appear ONLY as the literal argument to `is_infra(` —
    // the store-scan filename classifier that skips underscore-prefixed infra
    // files (see `src/rebuild.rs`). Any other appearance would mean something
    // is opening/parsing it as a legacy markdown grammar (the reseed grammar
    // is `grammar.toml`/`_grammar.toml`, never markdown).
    let mut hits = 0;
    for path in src_files() {
        for line in code_lines(&path) {
            if line.contains("_grammar.md") {
                hits += 1;
                assert!(
                    line.contains("is_infra("),
                    "`_grammar.md` referenced outside the infra-filename classifier \
                     in {}: `{}` (D15: no legacy markdown-grammar reader)",
                    path.display(),
                    line.trim()
                );
            }
        }
    }
    assert!(
        hits > 0,
        "expected at least the known `is_infra(\"_grammar.md\")` classification hit; \
         if this now legitimately reads 0, the allowance above is dead and should be \
         reconsidered rather than silently trusted"
    );
}
