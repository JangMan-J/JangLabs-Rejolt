//! Conformance for the hook dispatch (WP-5 / P8; D12, D19, D6, A5, D18) — the
//! WP-5 gate: RB1(a), the engine-contract deny/allow fixtures, plus the quiet
//! pass path, the `.surface-disabled` kill-switch, the A5(b) malformed-payload
//! fail-open, post-op rebuild-refresh/read-signal, and session-start's marker +
//! routability/drift advisories. Drives the BUILT binary end-to-end: `hook` has
//! no `--store` flag (Appendix D), so every test sandboxes `XDG_CONFIG_HOME`
//! (the global config `rejolt::config::default_config_path` reads to learn
//! `storeRoots.boxRoot`) and `XDG_RUNTIME_DIR` (the dedup-mark dir) per-process
//! via `Command::env` — never a same-process `std::env::set_var`, which would
//! race other tests reading `HOME`/`XDG_*` under the harness.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::telemetry::{FireMem, FireRecord, Telemetry};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rejolt")
}

fn unique_dir(tag: &str) -> PathBuf {
    static C: AtomicU32 = AtomicU32::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("rejolt-hookcli-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
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

/// A sandboxed hook-mode fixture: a bootstrapped store + a GLOBAL config.toml
/// (under a sandboxed `XDG_CONFIG_HOME`) naming it as `storeRoots.boxRoot`, plus
/// a sandboxed `XDG_RUNTIME_DIR` for dedup marks. Returns (base, store, envs).
struct Fixture {
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
    std::fs::create_dir_all(xdg_config.join("rejolt")).unwrap();
    std::fs::create_dir_all(&xdg_runtime).unwrap();

    // Bootstrap the store first (no hook-mode env needed for this call — it
    // takes --store directly).
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

    // The GLOBAL config `resolve_store` reads: names this store as `boxRoot`.
    std::fs::write(
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

// A degenerate trigger set: only a generic command, no narrowing lever — the
// static degenerate gate denies (needs a loaded catalog, which a booted store
// already has from its first rebuild).
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

// A memory with no triggers and a tag unknown to the (empty-seed) grammar:
// zero routing rows -> unroutable (D18's routabilityReport).
const ORPHAN_MEMORY: &str = "\
---
name: orphan
description: no way to route
metadata:
  tags: [misc]
---
body
";

fn pre_op_write(target: &Path, content: &str, cwd: &Path) -> String {
    serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": target.to_str().unwrap(), "content": content},
        "cwd": cwd.to_str().unwrap(),
    })
    .to_string()
}

// =============================================================================
// RB1(a): the write-guard deny/allow engine-contract fixtures
// =============================================================================

#[test]
fn deny_contract_short_circuits_stderr_exit_2_no_stdout() {
    let f = boot("deny");
    let target = f.store.join("bad.md");
    let payload = pre_op_write(&target, DEGENERATE_MEMORY, &f.store);

    let out = f.hook("pre-op", &payload);
    assert_eq!(out.code, 2, "a degenerate memory write denies (exit 2)");
    assert!(
        out.stdout.is_empty(),
        "DENY short-circuits: no stdout at all, got: {}",
        out.stdout
    );
    assert!(
        !out.stderr.is_empty() && out.stderr.contains("bad.md"),
        "the deny line names the target: {}",
        out.stderr
    );
}

#[test]
fn allowed_memory_write_emits_write_context_exit_0() {
    let f = boot("allow");
    let target = f.store.join("gpu-notes.md");
    let payload = pre_op_write(&target, GOOD_MEMORY, &f.store);

    let out = f.hook("pre-op", &payload);
    assert_eq!(
        out.code, 0,
        "a well-formed memory write allows; {}",
        out.stderr
    );
    assert!(
        out.stderr.is_empty(),
        "allow is quiet on stderr: {}",
        out.stderr
    );
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout must be one JSON envelope ({e}): {}", out.stdout));
    let ctx = parsed["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext present");
    assert!(
        ctx.contains("OBSERVED BEHAVIOR"),
        "write-context schema hint present: {ctx}"
    );
    assert_eq!(parsed["hookSpecificOutput"]["suppressOutput"], true);
}

#[test]
fn never_exits_one_across_every_event_and_payload_shape() {
    let f = boot("neverone");
    let target = f.store.join("bad.md");
    let cases: &[(&str, String)] = &[
        ("pre-op", pre_op_write(&target, DEGENERATE_MEMORY, &f.store)),
        ("pre-op", pre_op_write(&target, GOOD_MEMORY, &f.store)),
        ("pre-op", "not json".to_string()),
        ("pre-op", "".to_string()),
        (
            "post-op",
            serde_json::json!({"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path": target.to_str().unwrap()}}).to_string(),
        ),
        (
            "session-start",
            serde_json::json!({"hook_event_name":"SessionStart","cwd": f.store.to_str().unwrap()}).to_string(),
        ),
    ];
    for (event, payload) in cases {
        let out = f.hook(event, payload);
        assert_ne!(
            out.code, 1,
            "hook `{event}` must never exit 1 (payload: {payload})"
        );
    }
}

// =============================================================================
// D12 quiet pass path
// =============================================================================

#[test]
fn quiet_allow_on_a_non_matching_bash_command() {
    let f = boot("quiet");
    let payload = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "totallyuniquenonroutablecommand12345"},
    })
    .to_string();

    let out = f.hook("pre-op", &payload);
    assert_eq!(out.code, 0);
    assert!(
        out.stdout.is_empty(),
        "quiet allow: no stdout: {}",
        out.stdout
    );
    assert!(
        out.stderr.is_empty(),
        "quiet allow: no stderr: {}",
        out.stderr
    );
}

// =============================================================================
// .surface-disabled kill-switch
// =============================================================================

#[test]
fn kill_switch_suppresses_every_event_including_a_deny() {
    let f = boot("killswitch");
    std::fs::write(f.store.join(".surface-disabled"), b"").unwrap();
    let target = f.store.join("bad.md");

    // Even a write that WOULD deny is suppressed entirely: exit 0, silent.
    let deny_payload = pre_op_write(&target, DEGENERATE_MEMORY, &f.store);
    let out = f.hook("pre-op", &deny_payload);
    assert_eq!(out.code, 0, "kill-switch overrides even a would-be deny");
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());

    // session-start and post-op are equally silenced.
    let ss = f.hook(
        "session-start",
        &serde_json::json!({"hook_event_name":"SessionStart","cwd": f.base.join("elsewhere").to_str().unwrap()}).to_string(),
    );
    assert_eq!(ss.code, 0);
    assert!(ss.stdout.is_empty() && ss.stderr.is_empty());

    let po = f.hook(
        "post-op",
        &serde_json::json!({"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path": target.to_str().unwrap()}}).to_string(),
    );
    assert_eq!(po.code, 0);
    assert!(po.stdout.is_empty() && po.stderr.is_empty());
}

// =============================================================================
// A5(b) malformed payload — fails open silently, never panics
// =============================================================================

#[test]
fn malformed_payload_fails_open_silently() {
    let f = boot("malformed");
    for garbage in ["not json at all", "{ broken", "", "\0\0\0", "[1,2,3"] {
        let out = f.hook("pre-op", garbage);
        assert_eq!(out.code, 0, "garbage stdin `{garbage:?}` must fail open");
        assert!(out.stdout.is_empty());
        assert!(out.stderr.is_empty());
    }
}

// =============================================================================
// post-op: rebuild-refresh (fail-open) + read-signal
// =============================================================================

#[test]
fn post_op_write_triggers_rebuild_refresh() {
    let f = boot("postop-rebuild");
    let report_path = f.store.join("_memory_catalog.json");
    let before = std::fs::read_to_string(&report_path).unwrap();

    // A new routable memory lands on disk directly (simulating the tool call
    // having already written it) — the hook does not write files, only reacts.
    let mem = "---\nname: newmem\ndescription: newly routable\nmetadata:\n  tags: [misc]\n  triggers:\n    commands: [uniquepostopcmd]\n---\nbody\n";
    let target = f.store.join("newmem.md");
    std::fs::write(&target, mem).unwrap();

    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": target.to_str().unwrap(), "content": mem},
    })
    .to_string();
    let out = f.hook("post-op", &payload);
    assert_eq!(out.code, 0, "post-op never exits nonzero; {}", out.stderr);
    assert!(
        out.stderr.is_empty(),
        "a successful refresh is quiet: {}",
        out.stderr
    );

    let after = std::fs::read_to_string(&report_path).unwrap();
    assert_ne!(
        before, after,
        "rebuild-refresh must have run (generation changed)"
    );
    assert!(
        after.contains("newmem"),
        "the new memory is now in the catalog"
    );
}

#[test]
fn post_op_read_of_live_marked_memory_logs_a_read() {
    let f = boot("postop-read");
    // `Telemetry::for_store` (what the subprocess uses) resolves its runtime dir
    // via `default_runtime_dir()`, which APPENDS the engine namespace
    // (`$XDG_RUNTIME_DIR/rejolt`) — match that exactly, or the mark this test
    // writes lands somewhere the subprocess never looks.
    let xdg_runtime = f
        .envs
        .iter()
        .find(|(k, _)| k == "XDG_RUNTIME_DIR")
        .map(|(_, v)| PathBuf::from(v))
        .unwrap();
    let tel = Telemetry::new(
        xdg_runtime.join("rejolt"),
        f.store.join("_recall_telemetry.jsonl"),
        rejolt::config::Config::default(),
    );
    // Manufacture a LIVE mark for "gpu-notes" via the one gated fire-logging path.
    let fire = FireRecord {
        ts: 1_000,
        qid: "q1".to_string(),
        mems: vec![FireMem {
            id: "gpu-notes".to_string(),
            tag: "gpu-tools".to_string(),
            trigger_type: "command".to_string(),
            val: "nvidia-smi".to_string(),
        }],
        conf: "high".to_string(),
    };
    let outcome = tel.log_fire(&fire);
    assert_eq!(
        outcome,
        rejolt::telemetry::FireOutcome::Logged,
        "mark must persist for this test to be meaningful"
    );

    let target = f.store.join("gpu-notes.md");
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Read",
        "tool_input": {"file_path": target.to_str().unwrap()},
    })
    .to_string();
    let out = f.hook("post-op", &payload);
    assert_eq!(out.code, 0);
    assert!(
        out.stdout.is_empty() && out.stderr.is_empty(),
        "post-op read-signal is quiet"
    );

    let tel_text = std::fs::read_to_string(f.store.join("_recall_telemetry.jsonl")).unwrap();
    assert!(
        tel_text
            .lines()
            .any(|l| l.contains("\"signal\":\"read\"") && l.contains("gpu-notes")),
        "a read record for gpu-notes must be appended: {tel_text}"
    );
}

// =============================================================================
// session-start: marker + maintenance_due + routability/drift advisories
// =============================================================================

#[test]
fn session_start_logs_the_session_marker() {
    let f = boot("sessionmarker");
    let tel_path = f.store.join("_recall_telemetry.jsonl");
    let before = std::fs::read_to_string(&tel_path).unwrap_or_default();

    let payload =
        serde_json::json!({"hook_event_name":"SessionStart","cwd": f.store.to_str().unwrap()})
            .to_string();
    let out = f.hook("session-start", &payload);
    assert_eq!(out.code, 0);

    let after = std::fs::read_to_string(&tel_path).unwrap();
    assert!(
        after.len() > before.len() && after.lines().any(|l| l.contains("\"signal\":\"session\"")),
        "the session marker must be appended: {after}"
    );
}

#[test]
fn session_start_emits_routability_and_drift_advisories_when_not_at_home() {
    let f = boot("sessionadvisory");
    // An unroutable memory (D18) + a degenerate trigger-bearing one (drift §11).
    std::fs::write(f.store.join("orphan.md"), ORPHAN_MEMORY).unwrap();
    std::fs::write(f.store.join("bad.md"), DEGENERATE_MEMORY).unwrap();
    let rebuild_out = run(&["rebuild", "--store", f.store.to_str().unwrap()], "", &[]);
    assert_eq!(rebuild_out.code, 0, "{}", rebuild_out.stderr);

    // cwd is NOT $HOME (some other tmp dir) -> advisories are NOT skipped.
    let not_home = f.base.join("some-other-project");
    std::fs::create_dir_all(&not_home).unwrap();
    let payload =
        serde_json::json!({"hook_event_name":"SessionStart","cwd": not_home.to_str().unwrap()})
            .to_string();

    let out = f.hook("session-start", &payload);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout must be one JSON envelope ({e}): {}", out.stdout));
    let ctx = parsed["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext present");
    assert!(
        ctx.contains("routability"),
        "routability delta present: {ctx}"
    );
    assert!(ctx.contains("orphan"), "the unroutable id is named: {ctx}");
    assert!(
        ctx.contains("drift"),
        "the §11 drift guardrail advisory present: {ctx}"
    );
    assert!(
        ctx.contains("bad"),
        "the degenerate memory id is named: {ctx}"
    );
}

#[test]
fn session_start_at_home_skips_advisories_but_still_logs_marker() {
    let f = boot("sessionathome");
    std::fs::write(f.store.join("orphan.md"), ORPHAN_MEMORY).unwrap();
    let rebuild_out = run(&["rebuild", "--store", f.store.to_str().unwrap()], "", &[]);
    assert_eq!(rebuild_out.code, 0, "{}", rebuild_out.stderr);

    let home = std::env::var("HOME").expect("HOME must be set for this test to be meaningful");
    let payload = serde_json::json!({"hook_event_name":"SessionStart","cwd": home}).to_string();
    let tel_path = f.store.join("_recall_telemetry.jsonl");
    let before = std::fs::read_to_string(&tel_path).unwrap_or_default();

    let out = f.hook("session-start", &payload);
    assert_eq!(out.code, 0);
    assert!(
        out.stdout.is_empty(),
        "at-home skips the advisory emission: {}",
        out.stdout
    );

    // Marker + maintenance-due check still ran (A7 ordering: BEFORE the skip).
    let after = std::fs::read_to_string(&tel_path).unwrap();
    assert!(
        after.len() > before.len(),
        "the session marker still logs even when at-home skips advisories"
    );
}
