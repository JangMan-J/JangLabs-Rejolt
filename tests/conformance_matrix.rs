//! WP-8 part A (plan P16): the §14 conformance-matrix completion.
//!
//! `docs/reports/conformance-manifest-routed-memory-reseed.md` maps every
//! CORE-SPEC §14 row (+ the P16 new rows) to its covering test(s). Most rows
//! are already proven by an earlier packet's suite (`tests/recall.rs`,
//! `tests/write_guard.rs`, `tests/hook_dispatch.rs`, `tests/flat_index.rs`,
//! `tests/curation.rs`, …) — this file does NOT duplicate those; it closes the
//! specific residual gaps the manifest identifies, each with an inline
//! known-good/known-bad contrast (G2 discipline):
//!
//!  - §14 row 1 / row 4 / **row J** (recall fail-open on a catalog that is
//!    *present but corrupt* — the *missing/absent* case is already covered by
//!    `tests/recall.rs::missing_index_is_silent_and_does_not_rebuild` /
//!    `::deleted_index_stays_deleted_after_recall`): a malformed flat-index
//!    record, and a schema-mismatched-but-parseable catalog report, must each
//!    fail open — silence, no rebuild, the corrupt files left untouched.
//!  - §14 row 4 residual ("direct catalog edits are overwritten"): a hand-
//!    edited artifact pair is fully REPLACED (not merged) by the next rebuild.
//!  - **row K** (`.surface-disabled` kill-switch, "every adapter path" —
//!    `tests/hook_dispatch.rs::kill_switch_suppresses_every_event_including_a_deny`
//!    already proves the write-deny / session-start / post-op-read branches):
//!    the three remaining branches — a would-be recall fire, a would-be
//!    write-context emission, and a would-be rebuild-refresh — are ALL
//!    suppressed too.
//!  - §14 row 10 (path canonicalization divergence): engine-realpath
//!    containment is not fooled by a symlink-based escape that only *looks*
//!    lexically "inside" the box root; the adapter-lexical matcher recognizes
//!    the store's (symlinked, per D13) grammar file through a pure
//!    `..`-detour with no filesystem access, and does not false-positive on a
//!    look-alike path that resolves elsewhere.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::catalog::{
    ArtifactRead, CatalogReport, IndexHeader, RoutabilityReport, read_artifacts,
};
use rejolt::config::Config;
use rejolt::guard::{GuardConfig, GuardVerdict, StoreRoots, check_write};
use rejolt::index::{IndexRecord, emit_records};
use rejolt::normalize::{NormalizedOp, parse_host_event};
use rejolt::rebuild::{BuildConfig, index_path, rebuild, report_path};
use rejolt::recall::recall;
use rejolt::telemetry::Telemetry;
use rejolt::tier::{Axis, SCHEMA_VERSION, Source};

use serde_json::json;

// =============================================================================
// Shared helpers (engine-level; mirrors `tests/recall.rs`'s local helpers —
// each integration-test file is its own crate, so nothing is importable across
// them)
// =============================================================================

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rejolt-wp8-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn temp_telemetry() -> (PathBuf, Telemetry) {
    let base = unique_dir("tel");
    let tel = Telemetry::new(base.join("rt"), base.join("tel.jsonl"), Config::default());
    (base, tel)
}

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

/// Write a healthy, generation-consistent artifact pair — recall's normal path.
fn write_healthy_pair(store: &Path, records: &[IndexRecord]) {
    let header = IndexHeader {
        generation: "gen-wp8".into(),
        source_fingerprint: "fp-wp8".into(),
        schema_version: SCHEMA_VERSION,
    };
    let index_text = format!("{}\n{}", header.emit(), emit_records(records));
    fs::write(index_path(store), index_text).expect("write index");
    let report = CatalogReport {
        schema_version: SCHEMA_VERSION,
        generation: "gen-wp8".into(),
        source_fingerprint: "fp-wp8".into(),
        memories: vec![],
        routability_report: RoutabilityReport::default(),
        vocab_digest: String::new(),
        malformed_files: vec![],
    };
    fs::write(report_path(store), report.to_json()).expect("write report");
}

fn bash(cmd: &str) -> NormalizedOp {
    parse_host_event(&json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": cmd},
    }))
}

// =============================================================================
// §14 row 1 / row 4 / row J — recall fail-open on a PRESENT-but-CORRUPT catalog
// =============================================================================

#[test]
fn malformed_index_record_fails_open_and_leaves_files_untouched() {
    let store = unique_dir("corrupt-record");

    // GOOD: a healthy pair with a strong command tuple fires — proves the query
    // used below WOULD surface if the index were not corrupt.
    write_healthy_pair(
        &store,
        &[rec(
            Axis::Command,
            "nvidia-smi",
            Source::Tag,
            "gpu",
            "gpu-mem",
        )],
    );
    let (_b1, tel_good) = temp_telemetry();
    assert!(
        recall(&bash("nvidia-smi -q"), &store, &tel_good)
            .advisory()
            .is_some(),
        "sanity: the healthy pair must fire on this query"
    );

    // BAD: corrupt the index in place — a valid header, then ONE record line
    // with the wrong column count (a malformed record, not merely absent).
    let header = IndexHeader {
        generation: "gen-wp8".into(),
        source_fingerprint: "fp-wp8".into(),
        schema_version: SCHEMA_VERSION,
    };
    let corrupt_index = format!("{}\ngarbage\ttoo\tfew\tcolumns\n", header.emit());
    fs::write(index_path(&store), &corrupt_index).expect("write corrupt index");
    let report_before = fs::read_to_string(report_path(&store)).expect("report still present");

    match read_artifacts(&index_path(&store), &report_path(&store)) {
        ArtifactRead::Malformed(msg) => {
            assert!(msg.contains("malformed record"), "message: {msg}")
        }
        other => panic!("a bad record line must read as Malformed, got {other:?}"),
    }

    // Recall over the SAME query must now go silent — fail open, not a panic,
    // not a stale hit.
    let (_b2, tel_bad) = temp_telemetry();
    assert!(
        recall(&bash("nvidia-smi -q"), &store, &tel_bad).is_silent(),
        "a corrupt-but-present index must fail open to silence"
    );

    // Fail-open means untouched, not "helpfully" rebuilt/repaired.
    assert_eq!(
        fs::read_to_string(index_path(&store)).unwrap(),
        corrupt_index,
        "recall must not rewrite the corrupt index"
    );
    assert_eq!(
        fs::read_to_string(report_path(&store)).unwrap(),
        report_before,
        "recall must not rewrite the report"
    );
}

#[test]
fn malformed_report_json_fails_open_and_leaves_files_untouched() {
    let store = unique_dir("corrupt-report");

    write_healthy_pair(
        &store,
        &[rec(Axis::Command, "rg", Source::Tag, "ripgrep", "rg-mem")],
    );
    let (_b1, tel_good) = temp_telemetry();
    assert!(
        recall(&bash("rg pattern"), &store, &tel_good)
            .advisory()
            .is_some(),
        "sanity: the healthy pair must fire on this query"
    );
    let index_before = fs::read_to_string(index_path(&store)).unwrap();

    // BAD: the report is syntactically valid JSON but does not match the
    // `CatalogReport` schema at all — "malformed-but-parseable" (§14 row 4).
    let corrupt_report = "{\"totallyWrongShape\":true}\n".to_string();
    fs::write(report_path(&store), &corrupt_report).expect("write corrupt report");

    match read_artifacts(&index_path(&store), &report_path(&store)) {
        ArtifactRead::Malformed(msg) => assert!(msg.contains("catalog report"), "message: {msg}"),
        other => panic!("a schema-mismatched report must read as Malformed, got {other:?}"),
    }

    let (_b2, tel_bad) = temp_telemetry();
    assert!(
        recall(&bash("rg pattern"), &store, &tel_bad).is_silent(),
        "a corrupt-but-present report must fail open to silence"
    );

    assert_eq!(
        fs::read_to_string(index_path(&store)).unwrap(),
        index_before,
        "recall must not rewrite the index"
    );
    assert_eq!(
        fs::read_to_string(report_path(&store)).unwrap(),
        corrupt_report,
        "recall must not rewrite the corrupt report"
    );
}

// =============================================================================
// §14 row 4 residual — direct catalog edits are OVERWRITTEN by rebuild, never
// merged
// =============================================================================

#[test]
fn rebuild_overwrites_direct_catalog_edits_not_merges() {
    let store = unique_dir("direct-edit");
    fs::write(
        store.join("m.md"),
        "---\nmetadata:\n  tags: [ripgrep]\n---\nbody\n",
    )
    .unwrap();
    let grammar_text = "grammar-version = 1\n\n[tool.ripgrep]\ngloss = \"ripgrep\"\nplacement = \"either\"\ncommands = [\"rg\"]\n";
    fs::write(store.join("_grammar.toml"), grammar_text).unwrap();
    rebuild(
        &store,
        &store.join("_grammar.toml"),
        &BuildConfig::default(),
    )
    .expect("initial rebuild");

    const MARKER: &str = "DIRECT-EDIT-MARKER-WP8";
    // Hand-edit both artifacts directly (out-of-band, never through `rebuild`).
    let mut tampered_index = fs::read_to_string(index_path(&store)).unwrap();
    tampered_index.push_str(&format!("# {MARKER}\n"));
    fs::write(index_path(&store), &tampered_index).unwrap();
    let tampered_report = format!("{{\"{MARKER}\":true}}\n");
    fs::write(report_path(&store), &tampered_report).unwrap();
    assert!(
        fs::read_to_string(index_path(&store))
            .unwrap()
            .contains(MARKER)
    );
    assert!(
        fs::read_to_string(report_path(&store))
            .unwrap()
            .contains(MARKER)
    );

    // Re-run rebuild over the SAME (unchanged) store + grammar.
    rebuild(
        &store,
        &store.join("_grammar.toml"),
        &BuildConfig::default(),
    )
    .expect("second rebuild");

    let index_after = fs::read_to_string(index_path(&store)).unwrap();
    let report_after = fs::read_to_string(report_path(&store)).unwrap();
    assert!(
        !index_after.contains(MARKER),
        "rebuild must fully replace the index, not merge the direct edit: {index_after}"
    );
    assert!(
        !report_after.contains(MARKER),
        "rebuild must fully replace the report, not merge the direct edit: {report_after}"
    );
    assert!(
        matches!(
            read_artifacts(&index_path(&store), &report_path(&store)),
            ArtifactRead::Consistent(_)
        ),
        "a fresh rebuild must read back Consistent, replacing the tampered pair"
    );
}

// =============================================================================
// Hook-mode helpers (subprocess; mirrors `tests/hook_dispatch.rs`'s local
// helpers — a separate integration-test crate, nothing importable across them)
// =============================================================================

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rejolt")
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], stdin: &str, envs: &[(&str, &str)]) -> Out {
    let mut cmd = Command::new(bin());
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn rejolt");
    {
        let mut si = child.stdin.take().unwrap();
        let _ = si.write_all(stdin.as_bytes());
    }
    let o = child.wait_with_output().expect("wait");
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

struct Fixture {
    #[allow(dead_code)]
    base: PathBuf,
    store: PathBuf,
    envs: Vec<(String, String)>,
}

impl Fixture {
    fn env_refs(&self) -> Vec<(&str, &str)> {
        self.envs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    fn hook(&self, event: &str, stdin: &str) -> Out {
        run(&["hook", event], stdin, &self.env_refs())
    }
}

fn boot(tag: &str) -> Fixture {
    let base = unique_dir(tag);
    let store = base.join("store");
    let grammar = base.join("_grammar.toml");
    let xdg_config = base.join("xdg-config");
    let xdg_runtime = base.join("xdg-runtime");
    fs::create_dir_all(xdg_config.join("rejolt")).unwrap();
    fs::create_dir_all(&xdg_runtime).unwrap();

    let boot_out = run(
        &[
            "bootstrap",
            "--store",
            store.to_str().unwrap(),
            "--grammar",
            grammar.to_str().unwrap(),
        ],
        "",
        &[],
    );
    assert_eq!(boot_out.code, 0, "bootstrap failed: {}", boot_out.stderr);

    fs::write(
        xdg_config.join("rejolt").join("config.toml"),
        format!("[storeRoots]\nboxRoot = \"{}\"\n", store.display()),
    )
    .unwrap();

    let envs = vec![
        (
            "XDG_CONFIG_HOME".to_string(),
            xdg_config.to_str().unwrap().to_string(),
        ),
        (
            "XDG_RUNTIME_DIR".to_string(),
            xdg_runtime.to_str().unwrap().to_string(),
        ),
    ];
    Fixture { base, store, envs }
}

const GOOD_MEMORY: &str = "\
---
name: gpu-notes
description: gpu tips
metadata:
  tags: [gpu-tools]
  triggers:
    commands: [nvidia-smi]
---
body
";

fn pre_op_write(target: &Path, content: &str) -> String {
    json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": target.to_str().unwrap(), "content": content},
    })
    .to_string()
}

// =============================================================================
// §14 row K — `.surface-disabled` kill-switch across the remaining adapter
// branches (deny / session-start / post-op-read already proven in
// `tests/hook_dispatch.rs`)
// =============================================================================

#[test]
fn kill_switch_suppresses_a_would_be_recall_fire() {
    let f = boot("ks-recall");
    fs::write(f.store.join("gpu-notes.md"), GOOD_MEMORY).unwrap();
    let rebuild_out = run(&["rebuild", "--store", f.store.to_str().unwrap()], "", &[]);
    assert_eq!(rebuild_out.code, 0, "{}", rebuild_out.stderr);

    let payload = json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": "nvidia-smi -q"},
    })
    .to_string();

    // GOOD: without the kill-switch, this command DOES fire an advisory.
    let fired = f.hook("pre-op", &payload);
    assert_eq!(fired.code, 0);
    assert!(
        !fired.stdout.is_empty(),
        "sanity: this command must normally surface an advisory"
    );

    // BAD: with the kill-switch, the SAME would-be fire is fully suppressed.
    fs::write(f.store.join(".surface-disabled"), b"").unwrap();
    let killed = f.hook("pre-op", &payload);
    assert_eq!(killed.code, 0);
    assert!(
        killed.stdout.is_empty() && killed.stderr.is_empty(),
        "kill-switch must suppress a would-be recall fire: stdout={:?} stderr={:?}",
        killed.stdout,
        killed.stderr
    );
}

#[test]
fn kill_switch_suppresses_a_would_be_write_context_emission() {
    let f = boot("ks-writectx");

    // GOOD: without the kill-switch, a well-formed candidate memory write emits
    // a write-context envelope.
    let target_a = f.store.join("gpu-notes.md");
    let allowed = f.hook("pre-op", &pre_op_write(&target_a, GOOD_MEMORY));
    assert_eq!(allowed.code, 0, "{}", allowed.stderr);
    assert!(
        !allowed.stdout.is_empty(),
        "sanity: an allowed candidate memory write must normally emit write-context"
    );

    // BAD: with the kill-switch, an equivalent candidate write is fully silent.
    fs::write(f.store.join(".surface-disabled"), b"").unwrap();
    let target_b = f.store.join("gpu-notes-2.md");
    let killed = f.hook("pre-op", &pre_op_write(&target_b, GOOD_MEMORY));
    assert_eq!(killed.code, 0);
    assert!(
        killed.stdout.is_empty() && killed.stderr.is_empty(),
        "kill-switch must suppress a would-be write-context emission: stdout={:?} stderr={:?}",
        killed.stdout,
        killed.stderr
    );
}

#[test]
fn kill_switch_suppresses_post_op_rebuild_refresh() {
    let f = boot("ks-rebuild");
    let report = report_path(&f.store);
    let index = index_path(&f.store);
    let before_report = fs::read_to_string(&report).unwrap();
    let before_index = fs::read_to_string(&index).unwrap();

    // Enable the kill-switch FIRST, then land a new routable memory that WOULD
    // trigger a rebuild-refresh — this is the "bad" (suppressed) side.
    fs::write(f.store.join(".surface-disabled"), b"").unwrap();
    let mem = "---\nname: newmem\ndescription: newly routable\nmetadata:\n  tags: [misc]\n  triggers:\n    commands: [uniquewp8postopcmd]\n---\nbody\n";
    let target = f.store.join("newmem.md");
    fs::write(&target, mem).unwrap();
    let payload = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": target.to_str().unwrap(), "content": mem},
    })
    .to_string();

    let out = f.hook("post-op", &payload);
    assert_eq!(out.code, 0);
    assert!(out.stdout.is_empty() && out.stderr.is_empty());

    assert_eq!(
        fs::read_to_string(&report).unwrap(),
        before_report,
        "kill-switch must suppress rebuild-refresh: the report must be untouched"
    );
    assert_eq!(
        fs::read_to_string(&index).unwrap(),
        before_index,
        "kill-switch must suppress rebuild-refresh: the index must be untouched"
    );
    assert!(
        !fs::read_to_string(&report).unwrap().contains("newmem"),
        "the new memory must NOT have been picked up under the kill-switch"
    );
}

// =============================================================================
// §14 row 10 — engine-realpath containment blocks a symlink-based escape
// =============================================================================

const BOX_PLACEMENT_GRAMMAR: &str = "grammar-version = 1\n\n[domain.boxonly]\ngloss = \"box general fact\"\nplacement = \"box\"\ncommands = [\"boxcmd\"]\n";

#[test]
fn engine_realpath_containment_blocks_symlink_based_escape() {
    let base = unique_dir("realpath-escape");
    let box_root = base.join("box");
    let outside = base.join("outside");
    fs::create_dir_all(&box_root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(box_root.join("_grammar.toml"), BOX_PLACEMENT_GRAMMAR).unwrap();

    // A symlink INSIDE the box root pointing OUTSIDE it. Its lexical/string form
    // (`box_root/escape/...`) looks like it is under the box; only realpath
    // resolution reveals it is not.
    symlink(&outside, box_root.join("escape")).unwrap();

    let cfg = GuardConfig {
        grammar_path: box_root.join("_grammar.toml"),
        roots: StoreRoots {
            box_root: Some(fs::canonicalize(&box_root).unwrap()),
        },
    };
    let boxmem = "---\ndescription: a box general fact\nmetadata:\n  tags: [boxonly]\n---\nbody\n";

    // GOOD: a target genuinely inside the box root is correctly placed → allow.
    let inside_target = box_root.join("inside.md");
    assert!(
        check_write(&box_root, &inside_target, boxmem, true, &cfg).is_allow(),
        "a target genuinely under the box root must be allowed"
    );

    // BAD: a target reached THROUGH the escaping symlink is lexically prefixed
    // by the box root's path, but realpath resolves it OUTSIDE — misplacement
    // must fire, proving containment is not fooled by the lexical prefix.
    let escape_target = box_root.join("escape").join("leak.md");
    match check_write(&box_root, &escape_target, boxmem, true, &cfg) {
        GuardVerdict::Deny(reason) => assert_eq!(reason.code(), "misplacement"),
        other => panic!(
            "a target that escapes the box root via a symlink must be denied as \
             misplaced (realpath containment), got {other:?}"
        ),
    }
}

// =============================================================================
// §14 row 10 — adapter-lexical canonicalization matches a symlinked infra path
// through a pure `..` detour (no filesystem access), and does not
// false-positive on a look-alike path that resolves elsewhere
// =============================================================================

#[test]
fn post_op_grammar_symlink_write_matched_via_pure_lexical_dotdot_collapse() {
    // GOOD: a `..` detour that lexically collapses to the REAL (symlinked)
    // grammar path must be recognized and trigger a rebuild-refresh.
    let f_good = boot("lex-good");
    assert!(
        fs::symlink_metadata(f_good.store.join("_grammar.toml"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "bootstrap must have created the store-side grammar symlink (D13)"
    );
    let new_mem = "---\nname: newmem\ndescription: newly routable\nmetadata:\n  tags: [misc]\n  triggers:\n    commands: [uniquewp8lexcmd]\n---\nbody\n";
    fs::write(f_good.store.join("newmem.md"), new_mem).unwrap();
    let detour_to_grammar = f_good
        .store
        .join("some-subdir")
        .join("..")
        .join("_grammar.toml");
    let payload = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": detour_to_grammar.to_str().unwrap()},
    })
    .to_string();
    let out = f_good.hook("post-op", &payload);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let report_after = fs::read_to_string(report_path(&f_good.store)).unwrap();
    assert!(
        report_after.contains("newmem"),
        "a `..`-detour path lexically equal to the symlinked grammar file must \
         still trigger rebuild-refresh: {report_after}"
    );

    // BAD: a `..` detour that looks similar (still ends in `_grammar.toml`) but
    // lexically collapses to somewhere OUTSIDE the store must NOT match — the
    // adapter compares the full canonical path, not just the filename.
    let f_bad = boot("lex-bad");
    fs::write(f_bad.store.join("newmem.md"), new_mem).unwrap();
    let report_before = fs::read_to_string(report_path(&f_bad.store)).unwrap();
    let detour_outside = f_bad
        .store
        .join("sub")
        .join("..")
        .join("..")
        .join("_grammar.toml");
    let payload_bad = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": detour_outside.to_str().unwrap()},
    })
    .to_string();
    let out_bad = f_bad.hook("post-op", &payload_bad);
    assert_eq!(out_bad.code, 0, "{}", out_bad.stderr);
    let report_after_bad = fs::read_to_string(report_path(&f_bad.store)).unwrap();
    assert_eq!(
        report_after_bad, report_before,
        "a look-alike path that resolves OUTSIDE the store must not trigger a refresh"
    );
    assert!(!report_after_bad.contains("newmem"));
}
