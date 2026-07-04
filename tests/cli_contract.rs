//! Appendix D CLI contract conformance (WP-7 / P15; D20, D12, A5). Drives the
//! BUILT binary and asserts each wired subcommand's flags / output / exit codes
//! match Appendix D exactly (WP-8 consumes this surface verbatim). Also proves the
//! D13/N7 property that `bootstrap --print-hooks` EMITS the settings block to
//! stdout and writes NOTHING to host settings.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::Value;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rejolt")
}

fn unique_dir(tag: &str) -> PathBuf {
    static C: AtomicU32 = AtomicU32::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("rejolt-cli-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], stdin: &str) -> Out {
    run_env(args, stdin, &[])
}

fn run_env(args: &[&str], stdin: &str, envs: &[(&str, &str)]) -> Out {
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
        let _ = si.write_all(stdin.as_bytes()); // ignore broken pipe if the child does not read
    }
    let o = child.wait_with_output().expect("wait");
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// Bootstrap a fresh store via the binary. Returns (base, store, grammar).
fn boot(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let base = unique_dir(tag);
    let store = base.join("store");
    let grammar = base.join("_grammar.toml");
    let out = run(
        &[
            "bootstrap",
            "--store",
            store.to_str().unwrap(),
            "--grammar",
            grammar.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(
        out.code, 0,
        "bootstrap should exit 0; stderr:\n{}",
        out.stderr
    );
    (base, store, grammar)
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

// A degenerate trigger set: only a generic command, no narrowing lever → the
// static degenerate gate denies (needs a loaded catalog, which a booted store has).
const DEGENERATE_MEMORY: &str = "\
---
name: bad
description: bad
metadata:
  tags: [misc]
  triggers:
    commands: [restart]
---
body
";

// =============================================================================
// bootstrap
// =============================================================================

#[test]
fn bootstrap_exits_zero_and_creates_files() {
    let (_base, store, _grammar) = boot("boot");
    assert!(store.join("MEMORY.md").exists());
    assert!(store.join("_grammar.toml").exists());
    assert!(store.join("_flat_index.tsv").exists());
    assert!(store.join("_memory_catalog.json").exists());
}

#[test]
fn print_hooks_emits_json_and_writes_no_host_settings() {
    // D13/N7: --print-hooks EMITS the settings block to stdout; the engine writes
    // NOTHING to host settings. We pin HOME + XDG_RUNTIME_DIR at a sandbox so any
    // host-settings write would land under our sandbox HOME/.claude — then assert
    // it never appears.
    let base = unique_dir("printhooks");
    let home = base.join("home");
    let xdg = base.join("xdg");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&xdg).unwrap();
    let store = base.join("store");
    let grammar = base.join("_grammar.toml");

    let out = run_env(
        &[
            "bootstrap",
            "--store",
            store.to_str().unwrap(),
            "--grammar",
            grammar.to_str().unwrap(),
            "--print-hooks",
        ],
        "",
        &[
            ("HOME", home.to_str().unwrap()),
            ("XDG_RUNTIME_DIR", xdg.to_str().unwrap()),
        ],
    );
    assert_eq!(
        out.code, 0,
        "bootstrap --print-hooks should exit 0; {}",
        out.stderr
    );

    // stdout is PURE, valid JSON (the report went to stderr) with a `hooks` block.
    let parsed: Value = serde_json::from_str(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "--print-hooks stdout must be valid JSON ({e}):\n{}",
            out.stdout
        )
    });
    assert!(
        parsed["hooks"]["PreToolUse"].is_array(),
        "hooks block present"
    );

    // D13: NO host settings were written anywhere the engine could reach.
    assert!(
        !home.join(".claude").exists(),
        "the engine must NEVER write host settings (~/.claude)"
    );
    assert!(
        !store.join("settings.json").exists(),
        "no settings.json in the store"
    );
}

// =============================================================================
// rebuild
// =============================================================================

#[test]
fn rebuild_human_and_json() {
    let (_base, store, _g) = boot("rebuild");
    let s = store.to_str().unwrap();
    let human = run(&["rebuild", "--store", s], "");
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(
        human.stdout.contains("rebuild: OK"),
        "loud on success (D12): {}",
        human.stdout
    );

    let json = run(&["rebuild", "--store", s, "--json"], "");
    assert_eq!(json.code, 0);
    let v: Value = serde_json::from_str(&json.stdout).expect("--json emits JSON");
    assert!(v["generation"].is_string(), "generation present");
    assert_eq!(v["unroutableCount"], 0, "empty store → 0 unroutable");
}

// =============================================================================
// validate
// =============================================================================

#[test]
fn validate_clean_findings_and_taxonomy() {
    let (base, store, _g) = boot("validate");
    let s = store.to_str().unwrap();

    // Clean booted store → 0.
    let clean = run(&["validate", "--store", s], "");
    assert_eq!(clean.code, 0, "clean store validates 0; {}", clean.stderr);

    // A malformed memory file → findings → exit 1.
    std::fs::write(store.join("broken.md"), "not frontmatter, no fences\n").unwrap();
    let findings = run(&["validate", "--store", s], "");
    assert_eq!(findings.code, 1, "malformed memory → findings (exit 1)");
    assert!(
        findings.stdout.contains("broken.md"),
        "the finding names the file"
    );

    // An invalid grammar (synonyms-only tag) → config/taxonomy → exit 2.
    let bad_grammar = base.join("bad_grammar.toml");
    std::fs::write(
        &bad_grammar,
        "grammar-version = 1\n\n[domain.weak]\ngloss = \"w\"\nplacement = \"either\"\nsynonyms = [\"x\"]\n",
    )
    .unwrap();
    let taxonomy = run(
        &[
            "validate",
            "--store",
            s,
            "--grammar",
            bad_grammar.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(
        taxonomy.code, 2,
        "grammar validation error → config/taxonomy (exit 2)"
    );
}

// =============================================================================
// check-write
// =============================================================================

#[test]
fn check_write_allow_and_deny() {
    let (_base, store, _g) = boot("checkwrite");
    let s = store.to_str().unwrap();

    let allow = run(
        &[
            "check-write",
            "--store",
            s,
            "--target",
            store.join("gpu-notes.md").to_str().unwrap(),
        ],
        GOOD_MEMORY,
    );
    assert_eq!(
        allow.code, 0,
        "a well-formed memory allows; {}",
        allow.stdout
    );
    assert!(
        allow.stdout.contains("ALLOW"),
        "loud allow: {}",
        allow.stdout
    );

    let deny = run(
        &[
            "check-write",
            "--store",
            s,
            "--target",
            store.join("bad.md").to_str().unwrap(),
        ],
        DEGENERATE_MEMORY,
    );
    assert_eq!(deny.code, 1, "a degenerate trigger set denies (exit 1)");
    assert!(deny.stdout.contains("DENY"), "loud deny: {}", deny.stdout);
}

// =============================================================================
// project
// =============================================================================

#[test]
fn project_json_and_bad_input() {
    let (_base, store, _g) = boot("project");
    let s = store.to_str().unwrap();

    let ok = run(
        &["project", "--store", s],
        r#"{"commands":["restart"],"paths":[],"args":[],"synonyms":[]}"#,
    );
    assert_eq!(ok.code, 0, "{}", ok.stderr);
    let v: Value = serde_json::from_str(&ok.stdout).expect("projection JSON");
    assert_eq!(v["distinctCount"], 0, "empty store → 0 co-fires");
    assert_eq!(v["verdict"], "PASS");
    assert!(v["liveLevers"].is_object() && v["perTrigger"].is_object());

    let bad = run(&["project", "--store", s], "not json at all");
    assert_eq!(bad.code, 2, "bad stdin JSON → usage/config (exit 2)");
}

// =============================================================================
// search
// =============================================================================

#[test]
fn search_silence_expect_absent_and_present() {
    let (_base, store, _g) = boot("search");
    let s = store.to_str().unwrap();
    let event = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"uniquebenchcmd"}}"#;

    // Empty store: silence, exit 0.
    let silence = run(&["search", "--store", s], event);
    assert_eq!(silence.code, 0, "silence exits 0; {}", silence.stderr);

    // --expect an absent id → exit 1 (the seat-probe form).
    let absent = run(
        &["search", "--store", s, "--expect", "no-such-memory"],
        event,
    );
    assert_eq!(absent.code, 1, "--expect an absent id → exit 1");

    // Bad JSON → exit 2.
    let bad = run(&["search", "--store", s], "}{ not json");
    assert_eq!(bad.code, 2, "bad event JSON → exit 2");

    // Route a memory, rebuild, then --expect it → exit 0.
    let mem = "---\nname: synth-mem\ndescription: routable\nmetadata:\n  tags: [synth-bench]\n  triggers:\n    commands: [uniquebenchcmd]\n---\nbody\n";
    std::fs::write(store.join("synth-mem.md"), mem).unwrap();
    let rebuilt = run(&["rebuild", "--store", s], "");
    assert_eq!(rebuilt.code, 0, "{}", rebuilt.stderr);
    let present = run(&["search", "--store", s, "--expect", "synth-mem"], event);
    assert_eq!(
        present.code, 0,
        "a surfaced memory → --expect exits 0; {}",
        present.stderr
    );
    assert!(present.stdout.contains("synth-mem") || present.stdout.contains("routable"));
}

// =============================================================================
// bench
// =============================================================================

#[test]
fn bench_nobaseline_exits_zero() {
    let (_base, store, _g) = boot("bench");
    let s = store.to_str().unwrap();
    let out = run(&["bench", "--store", s, "--samples", "16"], "");
    assert_eq!(
        out.code, 0,
        "NOBASELINE measure-only exits 0; {}",
        out.stderr
    );
    assert!(
        out.stdout.contains("NOBASELINE"),
        "verdict line: {}",
        out.stdout
    );
}

// =============================================================================
// not-yet-wired stubs (surface defined; bodies in WP-5 / WP-6)
// =============================================================================

#[test]
fn maintain_and_seats_are_wired_and_exit_zero_with_no_evidence() {
    // WP-6 wires the curation engine; a freshly-bootstrapped store has no
    // telemetry at all, so every one of these is a NORMAL (exit 0) no-op, never
    // the old "NOT YET WIRED" exit-1 stub.
    let (_base, store, _g) = boot("curation-wired");
    let s = store.to_str().unwrap();

    // No telemetry file at all -> below the ≥50-record trigger -> exit 0.
    let maintain = run(&["maintain", "--store", s], "");
    assert_eq!(maintain.code, 0, "below-trigger maintain is a normal no-op");
    assert!(maintain.stdout.contains("below the maintenance trigger"));

    // `--force` bypasses the record-count trigger but not the minimum-evidence
    // floor — a fresh store has zero telemetry, so evidence is insufficient.
    let maintain_forced = run(&["maintain", "--store", s, "--force"], "");
    assert_eq!(
        maintain_forced.code, 0,
        "insufficient-evidence maintain is a normal no-op, not a failure"
    );
    assert!(maintain_forced.stdout.contains("insufficient evidence"));

    // Seats: the bootstrap MEMORY.md seed carries no seats and no telemetry —
    // insufficient evidence, exit 0, never a failure.
    let seats = run(&["seats", "--store", s], "");
    assert_eq!(
        seats.code, 0,
        "insufficient-evidence seats is a normal no-op"
    );
    assert!(seats.stdout.contains("insufficient evidence"));

    let seats_propose = run(&["seats", "--store", s, "--propose"], "");
    assert_eq!(seats_propose.code, 0);
    assert!(seats_propose.stdout.contains("insufficient evidence"));
}

#[test]
fn hook_with_no_store_configured_is_fail_open_never_exit_one() {
    // A5/D20: the hook path NEVER exits 1. `hook` has no `--store` flag (WP-5:
    // it resolves the store from the GLOBAL config, `rejolt::config::
    // default_config_path`); with no such config on this test's ambient
    // environment, `box_root` resolves to `None` — a quiet fail-open allow
    // (exit 0), never a block, never exit 1. The full wired dispatch (deny,
    // recall, write-context, rebuild-refresh, read-signal, session-start
    // advisories) is covered end-to-end, sandboxed, in `tests/hook_dispatch.rs`.
    for event in ["session-start", "pre-op", "post-op"] {
        let out = run(&["hook", event], "");
        assert_eq!(
            out.code, 0,
            "hook `{event}` with no store configured must fail-open (exit 0), never 1"
        );
    }
}

// =============================================================================
// usage (exit 2) — clap-driven
// =============================================================================

#[test]
fn usage_errors_exit_two() {
    // Missing required flags are usage errors → exit 2 (A5/D20 taxonomy).
    assert_eq!(
        run(&["rebuild"], "").code,
        2,
        "rebuild without --store → usage 2"
    );
    let (_base, store, _g) = boot("usage");
    let s = store.to_str().unwrap();
    assert_eq!(
        run(&["check-write", "--store", s], "").code,
        2,
        "check-write without --target → usage 2"
    );
}
