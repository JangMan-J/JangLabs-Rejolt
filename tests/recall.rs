//! Conformance for the recall path + host-event parser (WP-3 / P6 + P7; D1, D3,
//! D5, D19, D25, A7; CORE-SPEC §5, §2.6, §14). End-to-end over an on-disk artifact
//! pair with the REAL WP-2b telemetry primitive (a stubbed telemetry path cannot
//! pass the fire-append row). Covers the §14 surface-gate/scoring row, the
//! diagnosable-fire citation row, and the index-only read-path invariant (N11).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::catalog::{CatalogReport, IndexHeader, RoutabilityReport};
use rejolt::config::Config;
use rejolt::index::{IndexRecord, emit_records};
use rejolt::normalize::{NormalizedOp, ToolOp, parse_host_event};
use rejolt::rebuild::{index_path, report_path};
use rejolt::recall::recall;
use rejolt::telemetry::{FireOutcome, Telemetry};
use rejolt::tier::{Axis, SCHEMA_VERSION, Source};

use serde_json::json;

// =============================================================================
// Helpers — a temp store, an on-disk artifact pair, and an injected Telemetry
// =============================================================================

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rejolt-wp3-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A telemetry primitive pointed at temp dirs. The runtime (mark) dir is left
/// UNcreated so the primitive creates it 0o700 (owned by us) and passes its own
/// safety gate; the telemetry file lives under the created base dir.
fn temp_telemetry() -> (PathBuf, Telemetry) {
    let base = unique_dir("tel");
    let tel = Telemetry::new(base.join("rt"), base.join("tel.jsonl"), Config::default());
    (base, tel)
}

/// A flat-index record with the routing fields set and benign display fields; the
/// `path` deliberately points at a file that does NOT exist, so any test that
/// surfaces this record proves recall never opened a memory body (N11).
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

/// Write a generation-consistent artifact pair (flat index + catalog report) into
/// `store`, so `read_artifacts` loads it as `Consistent`. Recall reads only the
/// index; the report carries no memories (recall never consults it).
fn write_index(store: &Path, records: &[IndexRecord]) {
    let header = IndexHeader {
        generation: "gen-test".into(),
        source_fingerprint: "fp-test".into(),
        schema_version: SCHEMA_VERSION,
    };
    let index_text = format!("{}\n{}", header.emit(), emit_records(records));
    fs::write(index_path(store), index_text).expect("write index");
    let report = CatalogReport {
        schema_version: SCHEMA_VERSION,
        generation: "gen-test".into(),
        source_fingerprint: "fp-test".into(),
        memories: vec![],
        routability_report: RoutabilityReport::default(),
        vocab_digest: String::new(),
        malformed_files: vec![],
    };
    fs::write(report_path(store), report.to_json()).expect("write report");
}

/// A PreToolUse Bash op from a command string.
fn bash(cmd: &str) -> NormalizedOp {
    parse_host_event(&json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": cmd},
    }))
}

// =============================================================================
// Index-only invariant (N11): no rebuild, no body load, silence, fail-open
// =============================================================================

#[test]
fn missing_index_is_silent_and_does_not_rebuild() {
    let store = unique_dir("noidx");
    let (_base, tel) = temp_telemetry();
    // No index/report written at all.
    assert!(recall(&bash("rg foo"), &store, &tel).is_silent());
    // Fail-open: recall must NOT have created (rebuilt) the artifacts on the read
    // path — the files stay absent.
    assert!(
        !index_path(&store).exists(),
        "recall must not rebuild the index"
    );
    assert!(
        !report_path(&store).exists(),
        "recall must not create the report"
    );
}

#[test]
fn deleted_index_stays_deleted_after_recall() {
    let store = unique_dir("delidx");
    let (_base, tel) = temp_telemetry();
    write_index(
        &store,
        &[rec(Axis::Command, "rg", Source::Tag, "ripgrep", "rg-mem")],
    );
    // Delete the index (leaving the report) → stale/missing → fail open.
    fs::remove_file(index_path(&store)).unwrap();
    assert!(recall(&bash("rg foo"), &store, &tel).is_silent());
    assert!(
        !index_path(&store).exists(),
        "recall must not recreate the deleted index"
    );
}

#[test]
fn surfaces_without_opening_the_memory_body() {
    // The record's `path` points at a nonexistent file. A fire here proves recall
    // reads only the index, never the body.
    let store = unique_dir("nobody");
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
    assert_eq!(adv.memories.len(), 1);
    assert!(
        !Path::new(&adv.memories[0].path).exists(),
        "the body file never existed"
    );
}

// =============================================================================
// Surface-gate matrix (§14): no/weak → silent; strong/two → fire; generic verb
// =============================================================================

#[test]
fn one_strong_tuple_fires() {
    let store = unique_dir("strong");
    let (_base, tel) = temp_telemetry();
    write_index(
        &store,
        &[rec(Axis::Command, "rg", Source::Tag, "ripgrep", "rg-mem")],
    );
    assert!(
        recall(&bash("rg pattern"), &store, &tel)
            .advisory()
            .is_some()
    );
}

#[test]
fn one_weak_tuple_is_silent() {
    let store = unique_dir("weak");
    let (_base, tel) = temp_telemetry();
    // Only a bySynonym (weak) entry; a lone weak tuple must NOT fire.
    write_index(
        &store,
        &[rec(
            Axis::Synonym,
            "vram",
            Source::Memory,
            "gpu-mem",
            "gpu-mem",
        )],
    );
    // "echo" is not indexed; "vram" is a content token → bySynonym weak hit only.
    assert!(recall(&bash("echo vram"), &store, &tel).is_silent());
}

#[test]
fn two_tuples_fire() {
    let store = unique_dir("two");
    let (_base, tel) = temp_telemetry();
    write_index(
        &store,
        &[
            rec(
                Axis::Arg,
                "release",
                Source::Memory,
                "cargo-mem",
                "cargo-mem",
            ),
            rec(
                Axis::Synonym,
                "grep",
                Source::Memory,
                "cargo-mem",
                "cargo-mem",
            ),
        ],
    );
    // "release" → byArg (medium); "grep" → bySynonym (weak). Two distinct tuples.
    let out = recall(&bash("echo release grep"), &store, &tel);
    let adv = out.advisory().expect("two tuples fire");
    assert_eq!(adv.memories[0].citations.len(), 2);
}

#[test]
fn generic_verb_does_not_count_as_strong() {
    let (_base, tel) = temp_telemetry();
    // BAD: the ONLY command tuple's basename is a generic verb → dropped → silent.
    let store_bad = unique_dir("gv-bad");
    write_index(
        &store_bad,
        &[rec(
            Axis::Command,
            "restart",
            Source::Memory,
            "svc-mem",
            "svc-mem",
        )],
    );
    assert!(
        recall(&bash("restart"), &store_bad, &tel).is_silent(),
        "a lone generic-verb command must not fire"
    );
    // GOOD: a non-generic command basename is strong → fires.
    let store_good = unique_dir("gv-good");
    write_index(
        &store_good,
        &[rec(
            Axis::Command,
            "systemctl",
            Source::Memory,
            "svc-mem",
            "svc-mem",
        )],
    );
    assert!(
        recall(&bash("systemctl"), &store_good, &tel)
            .advisory()
            .is_some(),
        "a non-generic command fires"
    );
}

// =============================================================================
// Dedup window (D5/D25): a live mark suppresses; without one, it fires
// =============================================================================

#[test]
fn dedup_window_suppresses_a_live_mark() {
    let store = unique_dir("dedup");
    let (_base, tel) = temp_telemetry();
    write_index(
        &store,
        &[rec(Axis::Command, "rg", Source::Tag, "ripgrep", "rg-mem")],
    );
    // GOOD (no mark yet): first recall fires and logs, writing the dedup mark.
    let first = recall(&bash("rg foo"), &store, &tel);
    assert!(first.advisory().is_some(), "first fire surfaces");
    assert_eq!(first.advisory().unwrap().fire, FireOutcome::Logged);
    // BAD (mark now live): the immediate re-fire is deduped → silence.
    assert!(
        recall(&bash("rg bar"), &store, &tel).is_silent(),
        "a live per-memory mark suppresses the re-fire"
    );
    // A separate, mark-free telemetry primitive fires the same query again.
    let (_base2, fresh) = temp_telemetry();
    assert!(recall(&bash("rg baz"), &store, &fresh).advisory().is_some());
}

// =============================================================================
// Fire-append (§14): the REAL primitive logs; the qid lands in telemetry
// =============================================================================

#[test]
fn fire_appends_a_record_through_the_real_primitive() {
    let store = unique_dir("fire");
    let (_base, tel) = temp_telemetry();
    write_index(
        &store,
        &[rec(Axis::Command, "rg", Source::Tag, "ripgrep", "rg-mem")],
    );
    let out = recall(&bash("rg foo"), &store, &tel);
    let adv = out.advisory().expect("fires");
    assert_eq!(adv.fire, FireOutcome::Logged);
    // The real telemetry file exists and carries a fire record with the qid.
    let logged = fs::read_to_string(tel.telemetry_file()).expect("telemetry file written");
    assert!(
        logged.contains(&adv.query_id),
        "the fire record carries the query id"
    );
    assert!(
        logged.contains("\"qid\""),
        "a fire record was appended, not a stub"
    );
}

// =============================================================================
// Diagnosable-fire citation (§2.6): `{route_tag} <- {trigger_type}:{matched_value}`
// =============================================================================

#[test]
fn advisory_renders_the_grammar_route_tag_citation() {
    let store = unique_dir("cite");
    let (_base, tel) = temp_telemetry();
    // A grammar-tag route: route_tag is the tag name `gpu` (source `t`).
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
    let adv = out.advisory().expect("fires");
    assert!(
        adv.text.contains("gpu <- command:nvidia-smi"),
        "advisory must cite the grammar route_tag: {}",
        adv.text
    );
    // The citation uses the populated trigger_type axis, never the empty `type`.
    let cite = &adv.memories[0].citations[0];
    assert_eq!(cite.route_tag, "gpu");
    assert_eq!(cite.trigger_type, "command");
    assert_eq!(cite.matched_value, "nvidia-smi");
}

// =============================================================================
// Fail-open deserialization (A5b): malformed / unknown-tool payload → no-op
// =============================================================================

#[test]
fn malformed_and_unknown_payloads_are_silent_no_ops() {
    let store = unique_dir("failopen");
    let (_base, tel) = temp_telemetry();
    write_index(
        &store,
        &[rec(Axis::Command, "rg", Source::Tag, "ripgrep", "rg-mem")],
    );

    // A malformed payload parses to Unclassifiable → recall no-op (no panic).
    let malformed = parse_host_event(&json!("not an object"));
    assert_eq!(malformed, NormalizedOp::Unclassifiable);
    assert!(recall(&malformed, &store, &tel).is_silent());

    // An unknown tool normalizes (fail-open) but extracts no routable token → silent.
    let unknown = parse_host_event(&json!({
        "hook_event_name": "PreToolUse", "tool_name": "SomeFutureTool",
        "tool_input": {"weird": 1},
    }));
    assert!(matches!(unknown, NormalizedOp::PreOp(_)));
    assert!(recall(&unknown, &store, &tel).is_silent());

    // SessionStart carries no operation → silent.
    let ss = NormalizedOp::SessionStart {
        cwd: Some(PathBuf::from("/w")),
    };
    assert!(recall(&ss, &store, &tel).is_silent());
}

// =============================================================================
// The full parse→recall path over a realistic Bash payload (path routing too)
// =============================================================================

#[test]
fn bash_path_routing_end_to_end() {
    let store = unique_dir("bashpath");
    let (_base, tel) = temp_telemetry();
    // A byPath trailing-/** prefix route + a byCommand route on the same memory.
    write_index(
        &store,
        &[
            rec(Axis::Path, "/etc/nvidia/**", Source::Tag, "gpu", "gpu-mem"),
            rec(Axis::Command, "nvidia-smi", Source::Tag, "gpu", "gpu-mem"),
        ],
    );
    // Bash touches a file under /etc/nvidia and runs nvidia-smi → two strong tuples.
    let op = parse_host_event(&json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": "cat /etc/nvidia/../nvidia/gpu.conf && nvidia-smi"},
        "cwd": "/home/u",
    }));
    let out = recall(&op, &store, &tel);
    let adv = out.advisory().expect("path + command both fire");
    assert_eq!(adv.memories.len(), 1);
    assert!(adv.text.contains("gpu <- path:"));
    assert!(adv.text.contains("gpu <- command:nvidia-smi"));
    let _ = ToolOp::default(); // ToolOp is part of the consumed public surface
}

// =============================================================================
// FIX 1 lock: a generic command never shadows a specific one (order-independent)
// =============================================================================

#[test]
fn generic_command_never_shadows_a_specific_one_order_independent() {
    // A memory routing on BOTH a generic (`install`) and a specific (`rsync`)
    // command under the SAME route_tag `svc`.
    let records = [
        rec(Axis::Command, "install", Source::Tag, "svc", "svc-mem"),
        rec(Axis::Command, "rsync", Source::Tag, "svc", "svc-mem"),
    ];
    // Both orders must fire and cite the SPECIFIC rsync — the FIX-1 lock. The
    // generic is filtered at the hit level (pre-dedup), so it can never become the
    // (svc, command) tuple representative and drop it.
    for cmd in [
        "install -m755 a b && rsync -a x y",
        "rsync -a x y && install -m755 a b",
    ] {
        let store = unique_dir("fix1");
        write_index(&store, &records);
        let (_b, fresh) = temp_telemetry();
        let out = recall(&bash(cmd), &store, &fresh);
        let adv = out
            .advisory()
            .unwrap_or_else(|| panic!("`{cmd}` must fire on the specific command"));
        assert!(
            adv.text.contains("svc <- command:rsync"),
            "`{cmd}` must cite the specific rsync: {}",
            adv.text
        );
        assert!(
            !adv.text.contains("command:install"),
            "the generic install must never surface: {}",
            adv.text
        );
    }

    // A memory whose ONLY command evidence is generic stays silent.
    let store = unique_dir("fix1-genonly");
    write_index(
        &store,
        &[rec(
            Axis::Command,
            "install",
            Source::Tag,
            "gen-only",
            "gen-only",
        )],
    );
    let (_b2, fresh2) = temp_telemetry();
    assert!(
        recall(&bash("install foo"), &store, &fresh2).is_silent(),
        "a generic-only command memory must not fire"
    );
}

// =============================================================================
// FIX 2 lock: web keywords are bySynonym-only (weak), never byArg (medium)
// =============================================================================

#[test]
fn web_keywords_are_synonym_only_never_byarg() {
    // A memory with a byArg `release` (medium) + bySynonym `grep` (weak).
    let records = [
        rec(
            Axis::Arg,
            "release",
            Source::Memory,
            "cargo-mem",
            "cargo-mem",
        ),
        rec(
            Axis::Synonym,
            "grep",
            Source::Memory,
            "cargo-mem",
            "cargo-mem",
        ),
    ];

    // WEB: web keywords reach bySynonym ONLY, so `release` never touches byArg and
    // `grep` alone is 1 weak tuple → below the ≥2/strong gate → SILENT.
    let store_web = unique_dir("fix2-web");
    write_index(&store_web, &records);
    let (_bw, telw) = temp_telemetry();
    let web = parse_host_event(&json!({
        "hook_event_name": "PreToolUse", "tool_name": "WebSearch",
        "tool_input": {"query": "release grep"},
    }));
    assert!(
        recall(&web, &store_web, &telw).is_silent(),
        "a web query must not reach byArg (would false-fire arg+synonym)"
    );

    // BASH: the SAME tokens as Bash arguments DO reach byArg — `echo release grep`
    // fires (arg `release` medium + synonym `grep` weak = 2 tuples) and cites
    // `arg:release`, proving the split is provenance-based, not token-based.
    let store_bash = unique_dir("fix2-bash");
    write_index(&store_bash, &records);
    let (_bb, telb) = temp_telemetry();
    let out = recall(&bash("echo release grep"), &store_bash, &telb);
    let adv = out.advisory().expect("Bash args reach byArg → fires");
    assert!(
        adv.text.contains("cargo-mem <- arg:release"),
        "a Bash arg must still match byArg (medium): {}",
        adv.text
    );
}
