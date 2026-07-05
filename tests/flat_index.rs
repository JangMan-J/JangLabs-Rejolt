//! Conformance for the flat recall index, the one walk, and rebuild
//! (WP-2 / P4 + P5; D4, D14, D24, A2, A3, §4, §11). Covers the risk register:
//! RB2 (one-record-per-line under hostile content), RB4 (torn-pair generation),
//! RB9 (recall ≡ projection through one walk), RB11 (byPath glob survives build),
//! plus key normalization, atomic ordering, and the drift guardrail.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::catalog::{ArtifactRead, CatalogReport, IndexHeader, read_artifacts};
use rejolt::conformance::{Check, assert_counts, fixtures_root};
use rejolt::grammar::parse_and_validate;
use rejolt::index::{Index, IndexRecord, WalkQuery};
use rejolt::rebuild::{
    Artifacts, BuildConfig, build_artifacts, drift_guardrail, index_path, rebuild, report_path,
    scan_store,
};

// =============================================================================
// Helpers
// =============================================================================

fn unique_store(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rejolt-wp2-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp store");
    dir
}

fn put(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).expect("write store file");
}

/// A minimal grammar with one `tool.ripgrep` tag (command evidence `rg`).
const RG_GRAMMAR: &str = "grammar-version = 1\n\n[tool.ripgrep]\ngloss = \"ripgrep\"\nplacement = \"either\"\ncommands = [\"rg\"]\nsynonyms = [\"grep\"]\n";

/// Scan + parse grammar + build in one step (no writes).
fn build(store: &Path, grammar_text: &str) -> Artifacts {
    let grammar = parse_and_validate(grammar_text).expect("valid grammar");
    let (memories, malformed) = scan_store(store).expect("scan store");
    build_artifacts(
        &memories,
        &malformed,
        &grammar,
        grammar_text,
        &BuildConfig::default(),
    )
}

// =============================================================================
// G2: the flat-index loader accepts well-formed index files, rejects malformed
// =============================================================================

/// Predicate: the file loads as a flat index (valid metadata header + every
/// non-comment line a valid 13-column record) — exactly the reader's index-side
/// parse in [`read_artifacts`].
fn index_file_loads(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let mut lines = text.lines();
    match lines.next() {
        Some(first) if IndexHeader::parse(first).is_some() => {}
        _ => return false,
    }
    for line in lines {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if IndexRecord::parse(line).is_err() {
            return false;
        }
    }
    true
}

#[test]
fn g2_flat_index_parse_valid() {
    let check = Check::new("index-record-parse-valid", "flat-index", index_file_loads);
    assert_counts(&check, &fixtures_root());
}

// =============================================================================
// RB2 — one record per physical line under hostile content
// =============================================================================

#[test]
fn rb2_one_record_per_line_under_hostile_content() {
    let store = unique_store("rb2");
    // Clean memory: routes via the grammar tag and its own trigger.
    put(
        &store,
        "clean.md",
        "---\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [clean-cmd]\n---\nbody\n",
    );
    // Hostile memory: a tab in a command, a newline in a path glob, a CR in an arg.
    put(
        &store,
        "hostile.md",
        "---\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [\"ok-cmd\", \"bad\\tcmd\"]\n    paths: [\"/good/**\", \"/bad\\nglob/**\"]\n    args: [\"war\\rn\"]\n---\nbody\n",
    );
    let artifacts = build(&store, RG_GRAMMAR);

    // Bad-fails: the three hostile fields are EXCLUDED and reported (A2e).
    let excluded = &artifacts.report.routability_report.excluded_entries;
    assert_eq!(
        excluded.len(),
        3,
        "expected 3 control-char exclusions, got {excluded:?}"
    );
    assert!(excluded.iter().all(|e| e.memory_id == "hostile"));

    // Good-passes: the clean fields survived and route.
    let patterns: Vec<&str> = artifacts
        .records
        .iter()
        .map(|r| r.pattern.as_str())
        .collect();
    assert!(patterns.contains(&"ok-cmd"));
    assert!(patterns.contains(&"/good/**"));
    assert!(patterns.contains(&"clean-cmd"));
    // No hostile bytes leaked into any pattern.
    assert!(
        artifacts
            .records
            .iter()
            .all(|r| !r.pattern.contains(['\t', '\n', '\r']))
    );

    // One record = one physical line: every emitted line has exactly 12 tabs and
    // no embedded newline, and the body line count equals the record count.
    for r in &artifacts.records {
        let line = r.emit();
        assert_eq!(
            line.matches('\t').count(),
            12,
            "record must be 13 columns: {line:?}"
        );
        assert!(!line.contains('\n'), "record must not embed a newline");
    }
    let body_lines = artifacts
        .index_text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .count();
    assert_eq!(
        body_lines,
        artifacts.records.len(),
        "one physical line per record"
    );
}

// =============================================================================
// RB11 — byPath glob preserved verbatim and matched by scan
// =============================================================================

#[test]
fn rb11_bypath_glob_survives_build_and_fires_correctly() {
    let store = unique_store("rb11");
    // Two patterns that MUST route: a trailing `/**` (brace-bearing) and a plain
    // fnmatch. Three that MUST NOT (they are broad, §3.x): mid, bare, and leading
    // `**`. All five are preserved VERBATIM in the index (byPath is exempt).
    put(
        &store,
        "cfg.md",
        "---\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    paths: [\"~/.config/{nvim,vim}/**\", \"/etc/*.conf\", \"**/*.md\", \"**\", \"~/**/settings.json\"]\n---\nbody\n",
    );
    let artifacts = build(&store, RG_GRAMMAR);

    // Preserved VERBATIM (case-, slash-, brace-, glob-bearing; not normalized).
    let path_patterns: Vec<&str> = artifacts
        .records
        .iter()
        .filter(|r| r.table_str() == "byPath")
        .map(|r| r.pattern.as_str())
        .collect();
    for pat in [
        "~/.config/{nvim,vim}/**",
        "/etc/*.conf",
        "**/*.md",
        "**",
        "~/**/settings.json",
    ] {
        assert!(
            path_patterns.contains(&pat),
            "byPath must preserve `{pat}` verbatim"
        );
    }

    // End-to-end: rebuild → walk(byPath) fires ONLY for the trailing-`/**` and the
    // plain fnmatch; the broad `**` forms never fire (RB11 correctness lock).
    let idx = Index::from_records(artifacts.records.clone());
    let home = std::env::var("HOME").unwrap();
    let q = WalkQuery {
        paths: vec![
            format!("{home}/.config/{{nvim,vim}}/init.lua"), // trailing `/**` literal prefix → fires
            "/etc/nginx.conf".to_string(),                   // plain fnmatch → fires
            "/repo/docs/readme.md".to_string(), // would match **/*.md IF it weren't broad
            format!("{home}/.config/settings.json"), // would match ~/**/settings.json IF not broad
        ],
        ..Default::default()
    };
    let fired: Vec<&str> = idx
        .walk(&q)
        .iter()
        .map(|h| h.record.pattern.as_str())
        .collect();
    assert!(
        fired.contains(&"~/.config/{nvim,vim}/**"),
        "trailing /** must fire"
    );
    assert!(fired.contains(&"/etc/*.conf"), "plain fnmatch must fire");
    assert!(
        !fired.contains(&"**/*.md"),
        "**/*.md is broad — must NOT fire"
    );
    assert!(!fired.contains(&"**"), "bare ** is broad — must NOT fire");
    assert!(
        !fired.contains(&"~/**/settings.json"),
        "leading ** is broad — must NOT fire"
    );
}

// =============================================================================
// RB4 — torn-pair generation mismatch → stale advisory, fail open
// =============================================================================

#[test]
fn rb4_torn_pair_detected_fail_open() {
    let store = unique_store("rb4");
    put(
        &store,
        "m.md",
        "---\nmetadata:\n  tags: [ripgrep]\n---\nbody\n",
    );
    put(&store, "_grammar.toml", RG_GRAMMAR);
    rebuild(
        &store,
        &store.join("_grammar.toml"),
        &BuildConfig::default(),
    )
    .expect("rebuild");

    // A consistent pair reads back Consistent, no advisory.
    match read_artifacts(&index_path(&store), &report_path(&store)) {
        ArtifactRead::Consistent(l) => {
            assert_eq!(l.header.generation, l.report.generation);
            assert!(
                read_artifacts(&index_path(&store), &report_path(&store))
                    .advisory()
                    .is_none()
            );
        }
        other => panic!("fresh rebuild should be Consistent, got {other:?}"),
    }

    // Simulate a torn rebuild: rewrite the report with a different generation id
    // (as if the index was rewritten with new inputs and the report never caught up).
    let report_text = fs::read_to_string(report_path(&store)).unwrap();
    let mut report = CatalogReport::from_json(&report_text).unwrap();
    report.generation = "0000000000000000".to_string();
    fs::write(report_path(&store), report.to_json()).unwrap();

    match read_artifacts(&index_path(&store), &report_path(&store)) {
        ArtifactRead::Stale(advisory) => {
            assert!(
                advisory.contains("stale artifact pair"),
                "advisory: {advisory}"
            );
        }
        other => panic!("a generation mismatch must be Stale, got {other:?}"),
    }
    // Fail-open: the reader yields no usable index (recall would go silent).
    assert!(
        read_artifacts(&index_path(&store), &report_path(&store))
            .loaded()
            .is_none()
    );
}

// =============================================================================
// RB9 — recall ≡ projection through the ONE walk
// =============================================================================

#[test]
fn rb9_recall_equals_projection_single_walk() {
    let store = unique_store("rb9");
    put(
        &store,
        "m.md",
        "---\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [cargo]\n    synonyms: [grep]\n---\nbody\n",
    );
    let artifacts = build(&store, RG_GRAMMAR);
    let idx = Index::from_records(artifacts.records.clone());

    // The SAME proposed trigger set, reached two ways: "as recall would" extract
    // tokens, and "as projection would" from the proposed triggers. Both build
    // the same WalkQuery and call the one walk — identical hit set (D4/RB9).
    let recall_query = WalkQuery {
        commands: vec!["cargo".into()],
        synonyms: vec!["grep".into()],
        ..Default::default()
    };
    let projection_query = WalkQuery {
        commands: vec!["cargo".into()],
        synonyms: vec!["grep".into()],
        ..Default::default()
    };
    let recall_hits = idx.walk(&recall_query);
    let projection_hits = idx.walk(&projection_query);
    assert!(!recall_hits.is_empty(), "the walk must fire for this set");
    assert_eq!(
        recall_hits, projection_hits,
        "recall ≡ projection through the one walk"
    );
}

// =============================================================================
// Key normalization: exact tables normalize; byPath does not
// =============================================================================

#[test]
fn key_normalization_at_build_and_walk() {
    let store = unique_store("norm");
    put(
        &store,
        "m.md",
        "---\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [\"  RipGrep  \"]\n    paths: [\"~/.Config/Nvim/**\"]\n---\nbody\n",
    );
    let artifacts = build(&store, RG_GRAMMAR);

    // byCommand key normalized at build: mixed-case + whitespace → `ripgrep`.
    assert!(
        artifacts
            .records
            .iter()
            .any(|r| r.table_str() == "byCommand" && r.pattern == "ripgrep"),
        "mixed-case/whitespace command must normalize to `ripgrep` at build"
    );
    // The `type` column is reserved-empty in the reseed (no frontmatter `type`).
    assert!(artifacts.records.iter().all(|r| r.mem_type.is_empty()));

    // byPath NOT normalized: case preserved verbatim.
    assert!(
        artifacts
            .records
            .iter()
            .any(|r| r.table_str() == "byPath" && r.pattern == "~/.Config/Nvim/**"),
        "byPath must preserve case (no normalization)"
    );

    // A normalized query matches the normalized command key.
    let idx = Index::from_records(artifacts.records.clone());
    let q = WalkQuery {
        commands: vec!["RIPGREP".into()],
        ..Default::default()
    };
    assert!(
        idx.walk(&q).iter().any(|h| h.record.pattern == "ripgrep"),
        "a normalized query must match the normalized key"
    );
}

// =============================================================================
// Report carries NO routing tables (A2b)
// =============================================================================

#[test]
fn report_carries_no_routing_tables() {
    let store = unique_store("report");
    put(
        &store,
        "m.md",
        "---\nname: rg-notes\ndescription: ripgrep tips\nmetadata:\n  tags: [ripgrep]\n---\nbody\n",
    );
    let artifacts = build(&store, RG_GRAMMAR);
    let json = &artifacts.report_text;
    for table in ["byCommand", "byPath", "byArg", "bySynonym", "triggerIndex"] {
        assert!(
            !json.contains(table),
            "A2b: report must carry no routing table `{table}`"
        );
    }
    // It DOES carry the write-side metadata A2b names.
    for field in [
        "\"memories\"",
        "\"routabilityReport\"",
        "\"sourceFingerprint\"",
        "\"vocabDigest\"",
        "\"generation\"",
    ] {
        assert!(json.contains(field), "report must carry {field}");
    }
    // Dedup input present: tags + description per memory.
    assert!(json.contains("ripgrep tips"));
}

// =============================================================================
// Idempotence + consistent pair (D2/P14, A2d)
// =============================================================================

#[test]
fn rebuild_is_idempotent_and_writes_consistent_pair() {
    let store = unique_store("idem");
    put(
        &store,
        "m.md",
        "---\nmetadata:\n  tags: [ripgrep]\n---\nbody\n",
    );
    put(&store, "_grammar.toml", RG_GRAMMAR);
    let gpath = store.join("_grammar.toml");

    let out1 = rebuild(&store, &gpath, &BuildConfig::default()).expect("rebuild 1");
    let index1 = fs::read_to_string(index_path(&store)).unwrap();
    let report1 = fs::read_to_string(report_path(&store)).unwrap();

    let out2 = rebuild(&store, &gpath, &BuildConfig::default()).expect("rebuild 2");
    let index2 = fs::read_to_string(index_path(&store)).unwrap();
    let report2 = fs::read_to_string(report_path(&store)).unwrap();

    assert_eq!(
        out1.generation, out2.generation,
        "same inputs → same generation"
    );
    assert_eq!(
        index1, index2,
        "flat index is byte-identical on re-run (idempotent)"
    );
    assert_eq!(
        report1, report2,
        "catalog report is byte-identical on re-run"
    );

    // The written pair is generation-consistent (index-first / report-last, A2d).
    match read_artifacts(&index_path(&store), &report_path(&store)) {
        ArtifactRead::Consistent(l) => assert_eq!(l.header.generation, l.report.generation),
        other => panic!("written pair must be Consistent, got {other:?}"),
    }
}

// =============================================================================
// Drift guardrail: fires on a degenerate trigger-bearing memory; else silent
// =============================================================================

#[test]
fn drift_guardrail_fires_on_degenerate_and_is_silent_otherwise() {
    // Degenerate: trigger-bearing but every declared trigger normalizes away
    // (an unknown tag + a `--bare` arg) → no live route → assertion 1 fires.
    let store = unique_store("drift-bad");
    put(
        &store,
        "degen.md",
        "---\nmetadata:\n  tags: [orphan-topic]\n  triggers:\n    args: [\"--bare\"]\n---\nbody\n",
    );
    let artifacts = build(&store, RG_GRAMMAR);
    assert!(
        artifacts
            .drift
            .advisories
            .iter()
            .any(|a| a.contains("degen") && a.contains("static-gate")),
        "drift must flag the degenerate memory: {:?}",
        artifacts.drift.advisories
    );

    // Clean: a routable trigger-bearing memory → guardrail silent, fail-open.
    let store2 = unique_store("drift-good");
    put(
        &store2,
        "good.md",
        "---\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [rg]\n---\nbody\n",
    );
    let clean = build(&store2, RG_GRAMMAR);
    assert!(
        clean.drift.is_clean(),
        "no drift expected: {:?}",
        clean.drift.advisories
    );

    // The guardrail is reusable from (memories, Index) at session-start (WP-5).
    let (mems, malformed) = scan_store(&store2).unwrap();
    let grammar = parse_and_validate(RG_GRAMMAR).unwrap();
    let arts = build_artifacts(
        &mems,
        &malformed,
        &grammar,
        RG_GRAMMAR,
        &BuildConfig::default(),
    );
    let idx = Index::from_records(arts.records.clone());
    assert!(drift_guardrail(&mems, &idx).is_clean());
}

// =============================================================================
// FIX 2 — control chars in NON-pattern columns cannot split the line (A2e)
// =============================================================================

/// A committed, ordinary memory whose `lastReviewed` decodes to a real tab must
/// index normally, with the tab sanitized to a space — it must not split the
/// line or disable the index.
#[test]
fn lastreviewed_tab_is_sanitized_not_line_splitting() {
    let store = unique_store("lr-tab");
    let fixture = fixtures_root().join("flat-index/memory-lastreviewed-tab.md");
    fs::copy(&fixture, store.join("rg.md")).expect("copy fixture memory");
    let artifacts = build(&store, RG_GRAMMAR);

    // The memory routes (it is not excluded) and its lastReviewed is sanitized.
    let rows: Vec<&IndexRecord> = artifacts
        .records
        .iter()
        .filter(|r| r.memory_id == "rg")
        .collect();
    assert!(!rows.is_empty(), "the memory must still route");
    assert!(
        rows.iter().all(|r| r.last_reviewed == "a b"),
        "lastReviewed tab must be sanitized to a space, got {:?}",
        rows.iter().map(|r| &r.last_reviewed).collect::<Vec<_>>()
    );
    // No exclusion, and every emitted line is exactly 13 columns / one line.
    assert!(
        artifacts
            .report
            .routability_report
            .excluded_entries
            .is_empty()
    );
    for r in &artifacts.records {
        let line = r.emit();
        assert_eq!(line.matches('\t').count(), 12);
        assert!(!line.contains(['\n', '\r']));
    }
}

/// A store file whose NAME carries control chars (Linux-legal) would inject a
/// tab/newline into the `memory_id` / `route_tag` / `path` columns. The whole
/// memory must be excluded + reported, and the surviving index must stay
/// well-formed (13 columns per line) — one hostile filename must not disable
/// recall store-wide.
#[test]
fn hostile_filename_excludes_whole_memory_and_keeps_index_wellformed() {
    let store = unique_store("hostile-name");
    // A clean memory that must survive and route.
    put(
        &store,
        "clean.md",
        "---\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [clean-cmd]\n---\nbody\n",
    );
    // A memory whose filename stem contains a tab AND a newline.
    let hostile_name = "we\tird\nname.md";
    fs::write(
        store.join(hostile_name),
        "---\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [would-route]\n---\nbody\n",
    )
    .expect("write control-char-named file");

    let artifacts = build(&store, RG_GRAMMAR);

    // The hostile memory is excluded wholesale and reported.
    let hostile_id = "we\tird\nname";
    assert!(
        artifacts
            .report
            .routability_report
            .excluded_entries
            .iter()
            .any(|e| e.memory_id == hostile_id && e.table == "(all)"),
        "hostile-named memory must be excluded + reported: {:?}",
        artifacts.report.routability_report.excluded_entries
    );
    // Its would-be route never made it into the index.
    assert!(
        artifacts.records.iter().all(|r| r.memory_id != hostile_id),
        "hostile-named memory must emit no rows"
    );
    // The clean memory still routes.
    assert!(artifacts.records.iter().any(|r| r.memory_id == "clean"));

    // The surviving index is well-formed: every physical line is exactly 13
    // columns and carries no embedded control char (so the reader will NOT
    // discard the index).
    for line in artifacts
        .index_text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        assert_eq!(
            line.matches('\t').count(),
            12,
            "line must be 13 columns: {line:?}"
        );
    }
    // And it round-trips through the single reader without Malformed.
    put(&store, "_grammar.toml", RG_GRAMMAR);
    rebuild(
        &store,
        &store.join("_grammar.toml"),
        &BuildConfig::default(),
    )
    .expect("rebuild");
    assert!(
        matches!(
            read_artifacts(&index_path(&store), &report_path(&store)),
            ArtifactRead::Consistent(_)
        ),
        "the index must not be discarded as Malformed by one hostile filename"
    );
}
