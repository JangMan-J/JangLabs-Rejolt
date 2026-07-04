//! The N-sweep (plan P17; WP-8 part B): N1–N14 from the frozen "Negative
//! contract" (`docs/frozen/routed-memory-reseed-decisions-20260703.md`), each
//! mapped to a mechanical check and pinned here so a future regression trips
//! this file. The narrative version of this table — check, expected result,
//! PASS/needs-vet disposition — lives in
//! `docs/reports/negative-contract-sweep-routed-memory-reseed.md`; this file
//! is its executable half.
//!
//! Scope note ("engine path"): `src/` outside any `#[cfg(test)]` block. The
//! one deliberate exception anywhere in this crate is the PyYAML differential
//! oracle in `tests/frontmatter.rs` (A3/B2) — test-only, and N10 explicitly
//! says "untouched" by it ("no python … on any ENGINE path").
//!
//! G4 note (N14): this sweep found a live contradiction between committed
//! code (`src/bench.rs::regression_ceiling`) and D9/D26/A4's calibration
//! protocol. It is NOT fixed here — `src/bench.rs` is outside this packet's
//! disjoint file scope (this packet writes only this file and the sweep-sheet
//! report). See the `n14_flagged_g4_…` test below and the sweep sheet's "N14
//! finding" section for the full writeup handed to `/vet`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use serde_json::json;

use rejolt::bench::{
    self, Baseline, CEILING_MIN_SLACK_MS, CEILING_REL_SLACK, EnvFingerprint,
    Verdict as BenchVerdict, regression_ceiling,
};
use rejolt::bootstrap;
use rejolt::catalog::{CatalogReport, IndexHeader, RoutabilityReport};
use rejolt::cli::Cli;
use rejolt::config::Config;
use rejolt::conformance::fixtures_root;
use rejolt::curation::{self, MaintainOutcome};
use rejolt::frontmatter::{self, FrontmatterError, Triggers};
use rejolt::grammar::{self, GrammarError};
use rejolt::hooks::{hooks_settings_block, render_print_hooks};
use rejolt::index::{Index, IndexRecord, emit_records};
use rejolt::normalize::parse_host_event;
use rejolt::projection::{self, COLLISION_GUIDE_FLOOR};
use rejolt::rebuild::{index_path, report_path};
use rejolt::recall::recall;
use rejolt::telemetry::{TELEMETRY_FILENAME, Telemetry};
use rejolt::tier::{Axis, SCHEMA_VERSION, Source};

// =============================================================================
// Shared helpers (each `tests/*.rs` file in this crate keeps its own small copy
// of these — this file follows the same convention rather than reaching into
// another test file).
// =============================================================================

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rejolt-nsweep-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn temp_telemetry() -> (PathBuf, Telemetry) {
    let base = unique_dir("tel");
    let tel = Telemetry::new(base.join("rt"), base.join("tel.jsonl"), Config::default());
    (base, tel)
}

/// A PreToolUse Bash op from a command string (mirrors `tests/recall.rs`).
fn bash(cmd: &str) -> rejolt::normalize::NormalizedOp {
    parse_host_event(&json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": cmd},
    }))
}

/// A flat-index record whose `path` deliberately points at a file that has
/// never existed — any test that surfaces this record proves the reader never
/// opened a memory body (N11).
fn rec(axis: Axis, pattern: &str, source: Source, route_tag: &str, memory_id: &str) -> IndexRecord {
    IndexRecord {
        axis,
        pattern: pattern.to_string(),
        route_tag: route_tag.to_string(),
        source,
        memory_id: memory_id.to_string(),
        mem_type: String::new(),
        last_reviewed: String::new(),
        decline_count: 0,
        tags: vec!["t".into()],
        path: format!("/nonexistent-store/{memory_id}.md"),
        snippet: format!("snippet for {memory_id}"),
    }
}

fn write_index(store: &Path, records: &[IndexRecord]) {
    let header = IndexHeader {
        generation: "gen-nsweep".into(),
        source_fingerprint: "fp-nsweep".into(),
        schema_version: SCHEMA_VERSION,
    };
    let index_text = format!("{}\n{}", header.emit(), emit_records(records));
    fs::write(index_path(store), index_text).expect("write index");
    let report = CatalogReport {
        schema_version: SCHEMA_VERSION,
        generation: "gen-nsweep".into(),
        source_fingerprint: "fp-nsweep".into(),
        memories: vec![],
        routability_report: RoutabilityReport::default(),
        vocab_digest: String::new(),
        malformed_files: vec![],
    };
    fs::write(report_path(store), report.to_json()).expect("write report");
}

/// Recursively collect every file under `dir` as `(path, contents-if-utf8-rs)`.
/// Used by the source-grep style checks (N1/N2/N3/N5/N9/N10/N11/N13).
fn engine_src_files() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    collect_rs(&root, &mut out);
    out
}

fn collect_rs(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let text = fs::read_to_string(&path).expect("read src file");
            out.push((path, text));
        }
    }
}

/// Read one `src/<name>` file as text.
fn read_src_file(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name))
        .unwrap_or_else(|e| panic!("read src/{name}: {e}"))
}

/// Recursively collect every file path under `dir` (any extension). Used by
/// the N12 vendoring sweep.
fn collect_all_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_all_paths(&path, out);
        } else {
            out.push(path);
        }
    }
}

// =============================================================================
// N1 — no second matcher: recall and projection share ONE index walk (D4).
// =============================================================================

#[test]
fn n1_exactly_one_walk_matcher_in_the_engine() {
    let total: usize = engine_src_files()
        .iter()
        .map(|(_, text)| text.matches("fn walk(").count())
        .sum();
    assert_eq!(
        total, 1,
        "N1/D4: exactly one `walk` matcher must exist under src/; found {total} \
         definitions — a second matcher would violate D4/N1"
    );
}

#[test]
fn n1_recall_and_projection_see_the_same_hit_through_the_one_walk() {
    // One index, one record. Recall (via a NormalizedOp) and projection (via a
    // proposed Triggers set) must both reach it — proving both consume
    // `Index::walk` (RB9), never a second matcher.
    let store = unique_dir("n1");
    let (_base, tel) = temp_telemetry();
    write_index(
        &store,
        &[rec(
            Axis::Command,
            "nvidia-smi",
            Source::Tag,
            "gpu",
            "gpu-mem",
        )],
    );
    let out = recall(&bash("nvidia-smi -q"), &store, &tel);
    let via_recall = out.advisory().expect("a strong command tuple fires");
    assert_eq!(via_recall.memories[0].memory_id, "gpu-mem");

    let index = Index::from_records(vec![rec(
        Axis::Command,
        "nvidia-smi",
        Source::Tag,
        "gpu",
        "gpu-mem",
    )]);
    let triggers = Triggers {
        commands: vec!["nvidia-smi".into()],
        ..Default::default()
    };
    let via_projection = projection::project(&triggers, &index);
    assert!(
        via_projection.collisions.contains(&"gpu-mem".to_string()),
        "N1/D4: projection must see the SAME record recall does — one walk, no second matcher"
    );
}

// =============================================================================
// N2 — no SQLite/FTS5 on the routing path; no embeddings/LLM on the read path.
// =============================================================================

#[test]
fn n2_no_sqlite_fts5_or_embeddings_anywhere_in_the_crate() {
    let cargo_toml = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read Cargo.toml");
    let forbidden = ["sqlite", "fts5", "rusqlite", "embedding"];
    for needle in forbidden {
        assert!(
            !cargo_toml.to_lowercase().contains(needle),
            "N2: Cargo.toml must not depend on `{needle}`"
        );
    }
    for (path, text) in engine_src_files() {
        let lower = text.to_lowercase();
        for needle in forbidden {
            assert!(
                !lower.contains(needle),
                "N2: {path:?} must not reference `{needle}` — no SQLite/FTS5/embeddings on the routing path"
            );
        }
    }
}

// =============================================================================
// N3 — no prompt-keyword routing (D3): routing keys on behavior, never prompt
// text.
// =============================================================================

#[test]
fn n3_no_prompt_text_field_feeds_routing() {
    // The two files that actually assemble routing tokens: the host-event
    // normalizer and the recall query extractor. Neither may read a "prompt"
    // JSON key or field. (The substring "prompt" alone is NOT forbidden: the
    // host event kind `UserPromptSubmit` legitimately appears — cited only to
    // be classified as a non-routing event, exactly N3's point.)
    for rel in ["normalize.rs", "recall.rs"] {
        let text = read_src_file(rel);
        for needle in ["\"prompt\"", ".prompt"] {
            assert!(
                !text.contains(needle),
                "N3/D3: src/{rel} must not read a `prompt` field (found `{needle}`) — routing keys \
                 on behavior (commands/paths/args), never prompt text"
            );
        }
    }
}

#[test]
fn n3_normalized_op_carries_no_prompt_field() {
    // An extraneous host `"prompt"` field must be silently ignored: the
    // normalized op is identical with or without it.
    let with_prompt = parse_host_event(&json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": "rg foo"}, "prompt": "please search for foo",
    }));
    let without_prompt = parse_host_event(&json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": "rg foo"},
    }));
    assert_eq!(
        format!("{with_prompt:?}"),
        format!("{without_prompt:?}"),
        "N3/D3: an extraneous host `prompt` field must not change the normalized op"
    );
}

// =============================================================================
// N4 — no standing review ritual; curation never deletes or rewrites memory
// content (D7): frontmatter-only mutation, bodies byte-identical, files never
// removed.
// =============================================================================

fn write_memory_n4(store: &Path, id: &str, decline_count: i64) {
    let s = format!(
        "---\nmetadata:\n  tags: [seat]\n  declineCount: {decline_count}\n---\nBODY for {id} — must never change.\n"
    );
    fs::write(store.join(format!("{id}.md")), s).expect("write memory");
}

fn body_of(content: &str) -> &str {
    let close = content
        .match_indices("---")
        .nth(1)
        .expect("closing fence")
        .0;
    content[close + 3..].trim_start_matches('\n')
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const DAY: i64 = 86_400;

fn fire_line(ts: i64, id: &str) -> String {
    format!(
        "{{\"ts\":{ts},\"qid\":\"q{ts}-{id}\",\"mems\":[{{\"id\":\"{id}\",\"tag\":\"{id}\",\"type\":\"command\",\"val\":\"x\"}}],\"conf\":\"high\"}}\n"
    )
}

fn session_line(ts: i64) -> String {
    format!("{{\"ts\":{ts},\"signal\":\"session\"}}\n")
}

fn append_telemetry(store: &Path, lines: &[String]) {
    let path = store.join(TELEMETRY_FILENAME);
    let mut existing = fs::read_to_string(&path).unwrap_or_default();
    for l in lines {
        existing.push_str(l);
    }
    fs::write(path, existing).unwrap();
}

#[test]
fn n4_curation_demotes_by_frontmatter_only_body_and_zero_fire_files_never_touched() {
    let store = unique_dir("n4");
    let cfg = Config::default();

    write_memory_n4(&store, "zero-fire", 0);
    let zero_fire_before = fs::read_to_string(store.join("zero-fire.md")).unwrap();

    write_memory_n4(&store, "low-read", 0);
    let now = now_unix();
    let mut lines: Vec<String> = (0..4).map(|i| fire_line(now - i, "low-read")).collect();
    lines.extend((0..10).map(|i| session_line(now - i * DAY)));
    append_telemetry(&store, &lines);

    let outcome = curation::maintain(&store, &cfg, true);
    let demoted = match outcome {
        MaintainOutcome::Ran { demoted, .. } => demoted,
        other => panic!("N4: expected a Ran outcome with sufficient evidence, got {other:?}"),
    };
    assert!(
        demoted.contains(&"low-read".to_string()),
        "a real fired-but-unread memory must demote: {demoted:?}"
    );
    assert!(
        !demoted.contains(&"zero-fire".to_string()),
        "N4/D7: a zero-fire memory is NEVER demoted (never mutated, let alone deleted)"
    );

    // N4/D7: the untouched memory's file is byte-identical (curation never even
    // opened it to rewrite).
    let zero_fire_after = fs::read_to_string(store.join("zero-fire.md")).unwrap();
    assert_eq!(
        zero_fire_after, zero_fire_before,
        "N4/D7: curation must never rewrite a memory it did not demote"
    );

    // N4/D7: the demoted memory's declineCount changed, but its BODY did not.
    let mutated = fs::read_to_string(store.join("low-read.md")).unwrap();
    assert!(
        mutated.contains("declineCount: 1"),
        "declineCount must actually increment: {mutated}"
    );
    assert_eq!(
        body_of(&mutated),
        "BODY for low-read — must never change.\n",
        "N4/D7: curation never rewrites memory BODIES, only the frontmatter block"
    );

    // N4: curation never DELETES a memory file.
    assert!(store.join("zero-fire.md").exists());
    assert!(store.join("low-read.md").exists());
}

// =============================================================================
// N5 — no bulk-LLM trigger derivation; per D18 no mechanical body-token
// derivation either: no inferred routes at all.
// =============================================================================

#[test]
fn n5_no_fallback_derivation_or_memory_derived_route_source() {
    let forbidden = [
        "derive_fallback",
        "memory-derived",
        "memory_derived",
        "byMemoryId",
    ];
    for (path, text) in engine_src_files() {
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "N5/D18: {path:?} must not reference `{needle}` — every route is DECLARED, never derived"
            );
        }
    }
}

#[test]
fn n5_route_source_is_closed_to_tag_or_declared_memory_trigger() {
    // Structural proof: `Source` has EXACTLY two provenances. If a third
    // ("derived"/"fallback") variant were ever added, this exhaustive match
    // fails to COMPILE — the strongest possible gate (the whole test binary
    // fails to build, not just this assertion).
    fn exhaustive(s: Source) -> &'static str {
        match s {
            Source::Tag => "t",
            Source::Memory => "m",
        }
    }
    assert_eq!(exhaustive(Source::Tag), "t");
    assert_eq!(exhaustive(Source::Memory), "m");
}

// =============================================================================
// N6 — no per-corpus block cutoff beyond the single collision floor (D8).
// =============================================================================

fn wide_command_index(command: &str, n: usize) -> Index {
    let records: Vec<IndexRecord> = (0..n)
        .map(|i| {
            rec(
                Axis::Command,
                command,
                Source::Tag,
                "wide",
                &format!("mem-{i}"),
            )
        })
        .collect();
    Index::from_records(records)
}

#[test]
fn n6_single_collision_floor_not_scaled_by_corpus_size() {
    let small = wide_command_index("restart", COLLISION_GUIDE_FLOOR + 1);
    let large = wide_command_index("restart", COLLISION_GUIDE_FLOOR + 1 + 500);
    let triggers = Triggers {
        commands: vec!["restart".into()],
        ..Default::default()
    };
    let p_small = projection::project(&triggers, &small);
    let p_large = projection::project(&triggers, &large);
    assert!(p_small.distinct_count > COLLISION_GUIDE_FLOOR);
    assert!(p_large.distinct_count > COLLISION_GUIDE_FLOOR);
    assert_eq!(
        p_small.verdict, p_large.verdict,
        "N6/D8: the SAME fixed floor must govern regardless of corpus scale — no per-corpus cutoff exists"
    );
}

#[test]
fn n6_no_second_breadth_cutoff_constant_defined_outside_projection() {
    // Scan the WHOLE engine for any const whose name suggests a corpus-breadth
    // cutoff (as opposed to unrelated thresholds like guard.rs's
    // `DEDUP_CANDIDATE_FLOOR`, which is a dedup *display* floor, not a breadth
    // cutoff). Exactly one such constant may exist: `COLLISION_GUIDE_FLOOR`.
    let mut breadth_consts = Vec::new();
    for (path, text) in engine_src_files() {
        for line in text.lines() {
            let l = line.trim();
            if (l.starts_with("pub const") || l.starts_with("const"))
                && (l.contains("COLLISION") || l.contains("BREADTH") || l.contains("CUTOFF"))
            {
                breadth_consts.push(format!("{path:?}: {l}"));
            }
        }
    }
    assert_eq!(
        breadth_consts.len(),
        1,
        "N6/D8: exactly one corpus-breadth-cutoff constant may exist (COLLISION_GUIDE_FLOOR); \
         found {}: {breadth_consts:?}",
        breadth_consts.len()
    );
    assert!(
        breadth_consts[0].contains("COLLISION_GUIDE_FLOOR"),
        "N6/D8: the sole breadth constant must be COLLISION_GUIDE_FLOOR; found {:?}",
        breadth_consts[0]
    );

    for consumer in ["guard.rs", "rebuild.rs", "config.rs"] {
        let text = read_src_file(consumer);
        assert!(
            text.contains("COLLISION_GUIDE_FLOOR"),
            "N6/D8: {consumer} must CONSUME projection::COLLISION_GUIDE_FLOOR, not define its own floor"
        );
    }
}

// =============================================================================
// N7 — no host permission-policy writes, including bootstrap (D13).
// =============================================================================

#[test]
fn n7_print_hooks_output_carries_no_permission_policy_keys() {
    let block = hooks_settings_block("rejolt");
    let rendered = render_print_hooks("rejolt");
    let as_text = block.to_string();
    for key in ["permissions", "allow", "deny", "defaultMode"] {
        let quoted = format!("\"{key}\"");
        assert!(
            !as_text.contains(&quoted),
            "N7/D13: the emitted hooks block must not carry the permission-policy key `{key}`"
        );
        assert!(
            !rendered.contains(&quoted),
            "N7/D13: --print-hooks output must not carry the permission-policy key `{key}`"
        );
    }
}

#[test]
fn n7_hooks_module_performs_no_filesystem_writes() {
    let text = read_src_file("hooks.rs");
    for needle in [
        "fs::write",
        "fs::create_dir",
        "File::create",
        "write_atomic",
        "std::fs::",
    ] {
        assert!(
            !text.contains(needle),
            "N7/D13: src/hooks.rs must perform NO filesystem I/O (found `{needle}`) — it only \
             builds+returns the hooks JSON for the user to place; the engine never writes it"
        );
    }
}

#[test]
fn n7_bootstrap_writes_only_inside_the_caller_provided_store_and_grammar_paths() {
    let store = unique_dir("n7-store");
    let grammar = unique_dir("n7-grammar-parent").join("grammar.toml");
    let cfg = Config::default();
    // Structural: bootstrap's signature is exactly (store, grammar, config) — no
    // host-settings-path parameter exists for it to write through.
    bootstrap::bootstrap(&store, &grammar, &cfg).expect("bootstrap should succeed");
    assert!(store.exists());
    assert!(grammar.exists());
}

// =============================================================================
// N8 — closed facet axis set + facet-less/tag-less denial (D21, D22).
// =============================================================================

#[test]
fn n8_a_fourth_top_level_facet_table_is_a_hard_deserialization_error() {
    let text = fs::read_to_string(fixtures_root().join("grammar/bad/fourth-table.toml"))
        .expect("read fixture");
    assert!(
        grammar::parse_grammar(&text).is_err(),
        "N8/D22: a 4th facet table must be REJECTED (deny_unknown_fields), not silently accepted"
    );
}

#[test]
fn n8_duplicate_facet_tag_is_denied() {
    let text = fs::read_to_string(fixtures_root().join("grammar/bad/duplicate-facet.toml"))
        .expect("read fixture");
    let g = grammar::parse_grammar(&text).expect("parses as a valid TOML shape");
    assert!(
        matches!(
            grammar::validate_grammar(&g),
            Err(GrammarError::DuplicateFacet { .. })
        ),
        "N8/D22: a tag declared under two facet tables must be denied — every tag has EXACTLY one facet"
    );
}

#[test]
fn n8_tagless_memory_is_denied_missing_and_empty() {
    let missing = fs::read_to_string(fixtures_root().join("frontmatter/bad/missing-tags.md"))
        .expect("read fixture");
    let empty = fs::read_to_string(fixtures_root().join("frontmatter/bad/empty-tags.md"))
        .expect("read fixture");
    assert!(
        matches!(
            frontmatter::parse(&missing),
            Err(FrontmatterError::MissingTags)
        ),
        "N8/D21: a memory with no metadata.tags key must be denied"
    );
    assert!(
        matches!(frontmatter::parse(&empty), Err(FrontmatterError::EmptyTags)),
        "N8/D21: a memory with metadata.tags: [] must be denied"
    );
}

// =============================================================================
// N9 — no legacy-format parsing code; no import flag (D17).
// =============================================================================

#[test]
fn n9_no_import_legacy_flag_on_the_cli() {
    let result = Cli::try_parse_from([
        "rejolt",
        "bootstrap",
        "--store",
        "/tmp/rejolt-n9-store",
        "--grammar",
        "/tmp/rejolt-n9-grammar.toml",
        "--import-legacy",
    ]);
    let err = result.expect_err(
        "N9/D17: --import-legacy must not be a recognized flag — it was dropped entirely",
    );
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("unexpected")
            || msg.to_lowercase().contains("unrecognized")
            || msg.contains("--import-legacy"),
        "unexpected clap error shape for an unknown flag: {msg}"
    );
}

#[test]
fn n9_no_legacy_parsing_code_in_the_engine() {
    // Every mention of "legacy" anywhere in the engine (case-insensitive — this
    // subsumes `import_legacy`/`import-legacy`/`ImportLegacy`) must be confined
    // to a `//`-comment line. `bootstrap.rs` legitimately CITES the absent flag
    // in its module doc ("There is no `--import-legacy`"); that is a comment,
    // never live code, and is exactly what this check requires.
    for (path, text) in engine_src_files() {
        for line in text.lines() {
            if line.to_lowercase().contains("legacy") {
                assert!(
                    line.trim_start().starts_with("//"),
                    "N9/D17: {path:?} has a non-comment `legacy` reference: {line}"
                );
            }
        }
    }
}

// =============================================================================
// N10 — no interpreter on any engine path; no runtime deps beyond the static
// binary (D16).
// =============================================================================

#[test]
fn n10_cargo_toml_dependencies_are_the_known_pure_rust_set() {
    let cargo_toml = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read Cargo.toml");
    let doc: toml::Value = toml::from_str(&cargo_toml).expect("parse Cargo.toml");
    let deps = doc
        .get("dependencies")
        .and_then(|d| d.as_table())
        .expect("[dependencies] table must exist");
    let known: BTreeSet<&str> = ["clap", "libc", "serde", "serde_json", "toml"]
        .into_iter()
        .collect();
    let names: BTreeSet<&str> = deps.keys().map(|s| s.as_str()).collect();
    assert_eq!(
        names, known,
        "N10: an unreviewed dependency was added or removed — confirm it is not an \
         interpreter/runtime/process-spawning crate before widening this set"
    );
    assert!(
        doc.get("dev-dependencies").is_none(),
        "N10: no dev-dependency crate is expected either (the PyYAML oracle shells out to the \
         SYSTEM python3 as a test-only tool, never a crate dependency)"
    );
}

#[test]
fn n10_no_process_spawn_anywhere_in_the_engine() {
    for (path, text) in engine_src_files() {
        assert!(
            !text.contains("Command::new"),
            "N10: {path:?} spawns a process (`Command::new`) — the engine may run NO \
             interpreter/subprocess on any engine path; only `std::process::id`/`std::process::exit` \
             (self pid / own exit) are permitted"
        );
    }
}

// =============================================================================
// N11 — recall never rebuilds, never loads memory bodies, never emits output
// on silence (D1, D19).
// =============================================================================

#[test]
fn n11_recall_never_rebuilds_a_missing_index() {
    let store = unique_dir("n11-missing");
    let (_base, tel) = temp_telemetry();
    assert!(recall(&bash("rg foo"), &store, &tel).is_silent());
    assert!(
        !index_path(&store).exists(),
        "N11/D1: recall must NEVER rebuild — the index stays absent"
    );
    assert!(
        !report_path(&store).exists(),
        "N11/D1: recall must NEVER create the report"
    );
}

#[test]
fn n11_recall_never_opens_the_memory_body_file() {
    let store = unique_dir("n11-nobody");
    let (_base, tel) = temp_telemetry();
    write_index(
        &store,
        &[rec(
            Axis::Command,
            "nvidia-smi",
            Source::Tag,
            "gpu",
            "gpu-mem",
        )],
    );
    let out = recall(&bash("nvidia-smi -q"), &store, &tel);
    let adv = out.advisory().expect("a strong command tuple fires");
    assert!(
        !Path::new(&adv.memories[0].path).exists(),
        "N11: the body file at `path` never existed — its content came only from the \
         pre-baked index snippet, never a body read"
    );
}

#[test]
fn n11_no_body_read_call_on_the_recall_hot_path() {
    let text = read_src_file("recall.rs");
    for needle in ["fs::read_to_string", "fs::read(", "File::open"] {
        assert!(
            !text.contains(needle),
            "N11: src/recall.rs must never open a file directly (found `{needle}`) — it consumes \
             only `catalog::read_artifacts`'s already-parsed result"
        );
    }
}

// =============================================================================
// N12 — no vendoring of synapse files into rejolt; reference by path only.
// =============================================================================

#[test]
fn n12_no_synapse_files_or_python_sources_vendored_into_this_repo() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for sub in ["src", "tests", "fixtures", "docs"] {
        let dir = root.join(sub);
        if !dir.exists() {
            continue;
        }
        let mut out = Vec::new();
        collect_all_paths(&dir, &mut out);
        for p in out {
            let s = p.to_string_lossy();
            assert!(
                !s.to_lowercase().contains("synapse"),
                "N12: {p:?} looks vendored from the synapse lab — reference by path only, never copy files in"
            );
            assert_ne!(
                p.extension().and_then(|e| e.to_str()),
                Some("py"),
                "N12: {p:?} is a vendored Python file — synapse is reference-by-path only"
            );
        }
    }
}

// =============================================================================
// N13 — adapter handlers never block a host operation on engine/store/index
// failure (D6).
// =============================================================================

#[test]
fn n13_hook_dispatch_defines_exactly_two_exit_codes_ok_and_deny() {
    let text = read_src_file("hook.rs");
    assert!(
        text.contains("const EXIT_OK: i32 = 0;"),
        "N13/A5: hook.rs must define the quiet-allow exit constant as 0"
    );
    assert!(
        text.contains("const EXIT_DENY: i32 = 2;"),
        "N13/A5: hook.rs must define the write-guard-deny exit constant as 2"
    );
    assert!(
        !text.contains("EXIT_FAIL"),
        "N13/A5: hook.rs must not reuse the direct-CLI EXIT_FAIL(1) constant — hook mode NEVER exits 1"
    );
    for needle in ["return 1;", "return 1,", "-> 1 "] {
        assert!(
            !text.contains(needle),
            "N13/D6: hook.rs must never literally return exit code 1 (found `{needle}`)"
        );
    }
}

// =============================================================================
// N14 — no performance magnitude asserted without the D26 calibration
// protocol behind it.
// =============================================================================

#[test]
fn n14_gate_is_measure_only_until_a_baseline_is_committed() {
    let (verdict, lines) = bench::verdict_of(3.0, None, &EnvFingerprint::detect());
    assert_eq!(
        verdict,
        BenchVerdict::NoBaseline,
        "N14/D26: with no committed baseline, the gate must be measure-only (NOBASELINE) — \
         no magnitude is asserted before the calibration commit exists"
    );
    assert!(
        !lines.is_empty(),
        "N14/A4: the NOBASELINE/measure-only state must be LOUD, never silent"
    );
}

#[test]
fn n14_flagged_g4_the_ceiling_still_bakes_in_the_superseded_static_slack() {
    // D9 states the CORE-SPEC §9 formula `max(25%, 15 ms)` is SUPERSEDED by the
    // D26/A4 calibration protocol, and A4(c) defines the REPLACEMENT slack
    // floor as EXACTLY `max(3σ, min→max band)` over ≥5 runs of ≥100 samples —
    // no static minimum anywhere in A4's wording. But
    // `bench::regression_ceiling` still folds `CEILING_REL_SLACK`(0.25) and
    // `CEILING_MIN_SLACK_MS`(15.0) into the ceiling as an UNCONDITIONAL floor
    // UNDER the calibrated slack, so a real, tiny, correctly-calibrated slack
    // can never narrow the ceiling below `baseline + 15 ms`. At the
    // D16-measured recall scale (0.7–2.4 ms!) this static 15 ms floor swallows
    // any realistic structural regression — the exact failure D26's own
    // rationale (the old ~100ms-scale regime does not transplant to <10 ms)
    // was written to prevent.
    //
    // This test PINS today's actual (contradiction-bearing) behavior — it is
    // NOT an endorsement of it. Flagged N14/G4 in the sweep sheet for /vet:
    // either (a) `regression_ceiling` should drop the static floor so the
    // ceiling is purely calibration-derived per A4(c), or (b) D9/A4 need an
    // amendment explicitly re-admitting a static defense-in-depth floor. Not
    // fixed here — `src/bench.rs` is outside this packet's file scope (this
    // packet writes only `tests/negative_contract.rs` and the sweep-sheet
    // report).
    let baseline = Baseline {
        p95_ms: 1.0,             // D16-scale real recall latency
        design_budget_ms: 100.0, // wide enough that WARN cannot mask this either
        ceiling_slack_ms: 0.3,   // a real, tiny, correctly-calibrated A4(c) slack
        env: EnvFingerprint::detect(),
    };
    // A 10x structural slowdown over baseline (1.0 ms -> 10.0 ms) — exactly the
    // shape of regression the REGRESSED verdict exists to catch.
    let (verdict, _lines) = bench::verdict_of(10.0, Some(&baseline), &baseline.env);
    assert_eq!(
        verdict,
        BenchVerdict::Pass,
        "G4/N14 CONTRADICTION (see sweep sheet): a 10x recall slowdown (1.0ms -> 10.0ms baseline, \
         calibrated slack 0.3ms) is swallowed by the static 15ms/25% floor (ceiling = 1.0 + \
         max(0.25, 15.0, 0.3) = 16.0ms > 10.0ms), so REGRESSED never fires and it isn't even WARN. \
         If this assertion ever starts failing with a DIFFERENT verdict, the static floor was \
         removed from `regression_ceiling` — that is PROGRESS: update this test (and the sweep \
         sheet's N14 row) to confirm N14 now holds cleanly; it is not a regression to chase down."
    );
    // Pin the arithmetic itself so the mechanism (not just the outcome) is legible:
    // the ceiling equals baseline + the STATIC 15ms floor, not baseline + the
    // calibrated 0.3ms slack — confirming CEILING_MIN_SLACK_MS (not
    // CEILING_REL_SLACK, since 0.25*1.0 << 15.0 here) is what set it.
    let ceiling = regression_ceiling(baseline.p95_ms, baseline.ceiling_slack_ms);
    assert!(
        (ceiling - (baseline.p95_ms + CEILING_MIN_SLACK_MS)).abs() < 1e-9,
        "the static CEILING_MIN_SLACK_MS({CEILING_MIN_SLACK_MS}ms) floor — not the calibrated \
         slack — is what set the ceiling here: ceiling={ceiling}"
    );
    assert!(
        CEILING_REL_SLACK * baseline.p95_ms < CEILING_MIN_SLACK_MS,
        "sanity: at this baseline scale the relative slack is dwarfed by the static ms floor"
    );
}
