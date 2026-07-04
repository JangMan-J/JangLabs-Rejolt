//! The hook dispatch (plan P8 / WP-5; D12, D19, D6, A5, D18): `rejolt hook
//! <session-start|pre-op|post-op>` reads the host payload JSON from stdin,
//! resolves the store, parses via [`crate::normalize::parse_host_event`], and
//! dispatches internally to recall (P6), the write guard (P9), rebuild-refresh
//! (P4), and the read-signal / session-marker primitive (P11).
//!
//! ## Exit taxonomy (A5 — CRITICAL, this is RB1(a))
//!
//! - **Quiet allow** — exit **0**, no stdout, no stderr.
//! - **Write-guard DENY** (the write-guard branch ONLY) — exit **2** + the deny
//!   line on **stderr**, and SHORT-CIRCUITS: nothing else is emitted (recall /
//!   write-context are suppressed).
//! - **NEVER exit 1** on any hook path (A5(a): D20's 0/1/2 direct-CLI taxonomy
//!   governs direct CLIs only; hook modes obey host semantics).
//! - A host-payload parse failure, an unclassifiable event, or a missing store
//!   all fail **open, silently** (A5(b), D6).
//!
//! ## Store resolution (no `--store` flag — Appendix D)
//!
//! Every other subcommand takes `--store`; `hook` does not. [`resolve_store`]
//! loads `config.toml` from the GLOBAL default path
//! ([`crate::config::default_config_path`], `${XDG_CONFIG_HOME:-~/.config}/
//! rejolt/config.toml`) via the fail-open hook loader, and uses
//! `config.store_roots.box_root` as BOTH the store directory to operate on and
//! the tunable source for the rest of the dispatch (tier weights, TTLs, dedup
//! thresholds, …) — one config load, no second per-store `config.toml` read. An
//! absent/unreadable global config, or an unconfigured `box_root`, resolves to
//! `None`: the caller then fails open (there is no store to act on). The
//! event's `cwd` is NOT used to pick a store (v1 has exactly one, global,
//! store) — it is used only for path canonicalization (recall's query, §5.x)
//! and the at-home floor-skip check (session-start, below).
//!
//! ## The `.surface-disabled` kill-switch (§5) — FIRST, before any dispatch
//!
//! If `<store>/.surface-disabled` exists, EVERY event (session-start, pre-op,
//! post-op) returns allow, exit 0, with NO stdout/stderr — the whole pipeline
//! is suppressed. Checked before the host payload is even parsed.
//!
//! ## pre-op dispatch (order matters — the short-circuit)
//!
//! 1. **Write-guard branch** (Write/Edit/MultiEdit with a `target_path`):
//!    [`crate::guard::check_write`]. `Deny` → the deny line to stderr, exit 2,
//!    NOTHING else emitted. `Allow` (or not a guardable write) → continue.
//! 2. **Recall** ([`crate::recall::recall`]): `Advisory` → its text joins the
//!    `additionalContext`; `Silence` → nothing (D19).
//! 3. **Write-context** ([`crate::guard::write_context`]), iff the op is a
//!    candidate memory write ([`crate::guard::is_candidate_memory_write`]).
//! 4. Exit 0 — quiet if nothing was collected (D12).
//!
//! ## post-op dispatch (PostToolUse cannot block — NEVER exit nonzero)
//!
//! 1. **Rebuild-refresh**: a `Write`/`Edit`/`MultiEdit` whose target is a store
//!    memory `.md` file or the grammar file → [`crate::rebuild::rebuild`],
//!    fail-open (a rebuild error keeps the stale catalog + ONE loud stderr
//!    correction line — the only post-op stderr).
//! 2. **Read-signal**: a `Read` of a store memory file →
//!    [`crate::telemetry::Telemetry::log_read`] (which itself gates on mark
//!    liveness, D25 — this call site does not need to re-check).
//! 3. Exit 0, otherwise quiet.
//!
//! ## session-start dispatch (A7 ordering: marker + maintenance check BEFORE the
//! at-home floor skip)
//!
//! 1. [`crate::telemetry::Telemetry::log_session`] FIRST (the session marker).
//! 2. [`maintenance_due`] — computed and exposed; the maintenance PASS itself
//!    (`curation::maintain`, WP-6) is **not wired here** (WP-6 does not exist in
//!    this worktree). See the `// INTEGRATOR:` marker below.
//! 3. The **at-home floor skip** ([`is_at_home`]), then — if not at home — the
//!    session-start advisories: (a) the routability delta (the D18
//!    `routabilityReport` reader) and (b) the §11 drift guardrail
//!    ([`crate::rebuild::drift_guardrail`], reused, not rebuilt).
//! 4. Exit 0.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::bootstrap;
use crate::catalog::read_artifacts;
use crate::cli::HookEvent;
use crate::config::{self, Config};
use crate::guard::{
    GuardConfig, GuardVerdict, StoreRoots, check_write, is_candidate_memory_write, write_context,
};
use crate::normalize::{NormalizedOp, canonicalize_lexical, parse_host_event};
use crate::rebuild::{BuildConfig, drift_guardrail, index_path, is_infra, rebuild, report_path};
use crate::recall::{RecallOutcome, recall};
use crate::telemetry::Telemetry;

/// Exit code: quiet allow (A5).
const EXIT_OK: i32 = 0;
/// Exit code: a write-guard DENY (the ONLY nonzero hook exit; A5).
const EXIT_DENY: i32 = 2;

/// The write-capable tool set the hook guards / refreshes on (A5(c), closed).
fn is_write_tool(tool_name: &str) -> bool {
    matches!(tool_name, "Write" | "Edit" | "MultiEdit")
}

// =============================================================================
// Entry point (`cli.rs::cmd_hook` calls this)
// =============================================================================

/// The hook entry: resolve the store, apply the kill-switch, parse stdin, and
/// dispatch per `event`. Returns the process exit code (A5 taxonomy: 0 / 2
/// only, never 1).
pub fn dispatch(event: HookEvent) -> i32 {
    let Some((store, config)) = resolve_store() else {
        return EXIT_OK; // no configured store -> nothing to act on, fail-open
    };
    if bootstrap::is_surface_disabled(&store) {
        return EXIT_OK; // §5 kill-switch: allow, silent, for EVERY event
    }

    let stdin = read_stdin();
    // A5(b): an unparseable payload is Unclassifiable (parse_host_event already
    // treats a non-object / Null value that way) — never an error, never a panic.
    let value: Value = serde_json::from_str(&stdin).unwrap_or(Value::Null);
    let op = parse_host_event(&value);

    match event {
        HookEvent::SessionStart => dispatch_session_start(&op, &store, &config),
        HookEvent::PreOp => dispatch_pre_op(&op, &store, &config),
        HookEvent::PostOp => dispatch_post_op(&op, &store, &config),
    }
}

/// Read all of stdin as a UTF-8 string (best-effort; a read fault yields `""`,
/// which fails the JSON parse above and lands on `Unclassifiable` — fail-open).
fn read_stdin() -> String {
    std::io::read_to_string(std::io::stdin()).unwrap_or_default()
}

// =============================================================================
// Store resolution (no `--store` flag — see module docs)
// =============================================================================

/// Resolve the store + its config for hook mode: load `config.toml` from the
/// GLOBAL default path ([`config::default_config_path`]) and read
/// `store_roots.box_root` as the store. `None` when the global config is
/// absent/unreadable (fail-open [`config::load_for_hook`] still returns a usable
/// default [`Config`], just with `box_root: None`) or `box_root` is unset —
/// there is then no store to act on, and every dispatch fails open.
fn resolve_store() -> Option<(PathBuf, Config)> {
    let cfg = config::load_for_hook(&config::default_config_path());
    let store = cfg.store_roots.box_root.clone()?;
    Some((store, cfg))
}

/// The [`GuardConfig`] for a resolved store: the grammar path (config override,
/// else the store-side `_grammar.toml`) + the store roots (`box_root` IS
/// `store` for this single-store v1 — see module docs).
fn guard_config(store: &Path, config: &Config) -> GuardConfig {
    GuardConfig {
        grammar_path: grammar_path_for(store, config),
        roots: StoreRoots {
            box_root: config.store_roots.box_root.clone(),
        },
    }
}

/// The grammar path: an explicit `config.grammarPath` override, else the
/// store-side `_grammar.toml` (mirrors `cli.rs::resolve_grammar`'s no-explicit-
/// flag branch — hook mode has no `--grammar` flag either).
fn grammar_path_for(store: &Path, config: &Config) -> PathBuf {
    config
        .grammar_path
        .clone()
        .unwrap_or_else(|| bootstrap::store_grammar_path(store))
}

// =============================================================================
// additionalContext emission (Appendix C: hookSpecificOutput + suppressOutput)
// =============================================================================

/// Emit one `hookSpecificOutput.additionalContext` JSON envelope to stdout
/// (Appendix C: `+ suppressOutput: true`). The caller only invokes this when
/// there is something to say — D12's quiet pass path is simply never calling it.
fn emit_additional_context(hook_event_name: &str, text: &str) {
    let payload = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": hook_event_name,
            "additionalContext": text,
            "suppressOutput": true,
        }
    });
    println!("{}", serde_json::to_string(&payload).unwrap_or_default());
}

// =============================================================================
// pre-op dispatch
// =============================================================================

fn dispatch_pre_op(op: &NormalizedOp, store: &Path, config: &Config) -> i32 {
    let tool_op = match op {
        NormalizedOp::PreOp(t) => t,
        _ => return EXIT_OK, // not actually a pre-op payload -> quiet allow
    };
    let cfg = guard_config(store, config);

    // 1. The write-guard branch (A5(c)'s closed write-capable tool set). A DENY
    //    SHORT-CIRCUITS: stderr + exit 2, nothing else emitted.
    if is_write_tool(&tool_op.tool_name)
        && let Some(target) = &tool_op.target_path
    {
        let content = tool_op.proposed_content.as_deref().unwrap_or("");
        if let GuardVerdict::Deny(reason) =
            check_write(store, target, content, tool_op.is_full_write, &cfg)
        {
            eprintln!(
                "rejolt hook: refused write to {} — {reason}",
                target.display()
            );
            return EXIT_DENY;
        }
    }

    // 2. Recall (D19): Advisory joins additionalContext; Silence emits nothing.
    let telemetry = Telemetry::for_store(store, config.clone());
    let mut parts: Vec<String> = Vec::new();
    if let RecallOutcome::Advisory(advisory) = recall(op, store, &telemetry) {
        parts.push(advisory.text);
    }

    // 3. Write-context, iff this is a candidate memory write (a full, fenced,
    //    non-grammar, non-infra write — i.e. Write only, never Edit/MultiEdit).
    if is_write_tool(&tool_op.tool_name)
        && let Some(target) = &tool_op.target_path
    {
        let content = tool_op.proposed_content.as_deref().unwrap_or("");
        if is_candidate_memory_write(target, content, tool_op.is_full_write, &cfg) {
            let wc = write_context(store, target, content, &cfg);
            parts.push(wc.text);
        }
    }

    if !parts.is_empty() {
        emit_additional_context("PreToolUse", &parts.join("\n\n"));
    }
    EXIT_OK
}

// =============================================================================
// post-op dispatch (PostToolUse cannot block — NEVER exit nonzero)
// =============================================================================

fn dispatch_post_op(op: &NormalizedOp, store: &Path, config: &Config) -> i32 {
    let tool_op = match op {
        NormalizedOp::PostOp(t) => t,
        _ => return EXIT_OK,
    };

    // 1. Rebuild-refresh: a write to a store memory `.md` file or the grammar
    //    file. Fail-open: on error, keep the stale catalog + ONE loud stderr
    //    correction line (the only post-op stderr).
    if is_write_tool(&tool_op.tool_name)
        && let Some(target) = &tool_op.target_path
        && is_store_routing_write(store, target, tool_op.cwd.as_deref(), config)
    {
        let grammar_path = grammar_path_for(store, config);
        let build_cfg = BuildConfig {
            max_description_chars: config.max_description_chars,
        };
        if let Err(e) = rebuild(store, &grammar_path, &build_cfg) {
            eprintln!("rejolt hook post-op: rebuild-refresh failed, catalog stays stale: {e}");
        }
    }

    // 2. Read-signal: a Read of a store memory file. `log_read` itself gates on
    //    mark liveness (D25) — this call site does not re-check.
    if tool_op.tool_name == "Read"
        && let Some(target) = &tool_op.target_path
        && let Some(id) = store_memory_id(store, target, tool_op.cwd.as_deref())
    {
        let telemetry = Telemetry::for_store(store, config.clone());
        telemetry.log_read(&id);
    }

    EXIT_OK
}

/// Whether `target` (given the op's `cwd`) is a store write that should trigger
/// a rebuild-refresh: the configured grammar file, or a store memory `.md` file
/// (same "memory" definition [`crate::rebuild::scan_store`] uses — via
/// [`is_infra`] — so the refresh trigger never drifts from what `rebuild`
/// itself would scan). Canonicalized ADAPTER-lexically (§5.x; this is the
/// adapter layer, not the engine-realpath placement check).
fn is_store_routing_write(
    store: &Path,
    target: &Path,
    cwd: Option<&Path>,
    config: &Config,
) -> bool {
    let canon_target = canonicalize_lexical(target, cwd);
    let grammar_path = grammar_path_for(store, config);
    if canonicalize_lexical(&grammar_path, None) == canon_target {
        return true;
    }
    store_memory_id_from(store, &canon_target).is_some()
}

/// The memory id (file stem) of `target` iff it resolves (adapter-lexically)
/// under `store` as a non-infra `.md` file — else `None`.
fn store_memory_id(store: &Path, target: &Path, cwd: Option<&Path>) -> Option<String> {
    let canon_target = canonicalize_lexical(target, cwd);
    store_memory_id_from(store, &canon_target)
}

fn store_memory_id_from(store: &Path, canon_target: &Path) -> Option<String> {
    let canon_store = canonicalize_lexical(store, None);
    if !canon_target.starts_with(&canon_store) {
        return None;
    }
    let name = canon_target.file_name()?.to_str()?;
    if !name.ends_with(".md") || is_infra(name) {
        return None;
    }
    name.strip_suffix(".md").map(str::to_string)
}

// =============================================================================
// session-start dispatch (A7 ordering)
// =============================================================================

fn dispatch_session_start(op: &NormalizedOp, store: &Path, config: &Config) -> i32 {
    let cwd = match op {
        NormalizedOp::SessionStart { cwd } => cwd.clone(),
        _ => return EXIT_OK, // not actually a session-start payload -> quiet allow
    };

    // 1. The session marker FIRST (A7 ordering).
    let telemetry = Telemetry::for_store(store, config.clone());
    telemetry.log_session();

    // 2. The maintenance-due check → run the curation pass when due (A7 ordering:
    //    the marker + this check both run BEFORE the at-home floor skip). The pass
    //    is internally concurrency-guarded (O_EXCL lock + recheck-under-lock, WR-01/
    //    WR-02), so a racing session-start no-ops; fail-open — its outcome is
    //    discarded and a maintenance fault never blocks session start (quiet, §8).
    if maintenance_due(store, &telemetry) {
        let _ = crate::curation::maintain(store, config, false);
    }

    // 3. The at-home floor skip, THEN the advisories (A7 ordering: 1 and 2 run
    //    unconditionally above; only the advisories are skipped at-home).
    if is_at_home(cwd.as_deref()) {
        return EXIT_OK;
    }

    let mut parts: Vec<String> = Vec::new();
    let read = read_artifacts(&index_path(store), &report_path(store));

    // (a) The routability delta (the D18 routabilityReport reader).
    if let Some(loaded) = read.loaded() {
        let rr = &loaded.report.routability_report;
        if rr.unroutable_count > 0 {
            parts.push(format!(
                "routability: {} unroutable memor{} — {}",
                rr.unroutable_count,
                if rr.unroutable_count == 1 { "y" } else { "ies" },
                rr.unroutable_ids.join(", ")
            ));
        }
    }

    // (b) The §11 drift guardrail — REUSED (rebuild::drift_guardrail), not
    // rebuilt: re-scan the store (a read, no artifact write) against the
    // ALREADY-loaded index above (the SAME `read`, no second artifact read).
    if let Some(loaded) = read.loaded()
        && let Ok((memories, _malformed)) = crate::rebuild::scan_store(store)
    {
        let drift = drift_guardrail(&memories, &loaded.index);
        parts.extend(drift.advisories);
    }

    if !parts.is_empty() {
        emit_additional_context("SessionStart", &parts.join("\n"));
    }
    EXIT_OK
}

/// Whether `cwd` is "at home" — the synapse tiebreaker's box-brain same-store
/// check, transplanted to rejolt's single global-store v1: there is no
/// per-project store keying here, so this does NOT resolve a git repo root the
/// way the synapse bash hook does (P1: zero runtime deps beyond the binary —
/// no `git` subprocess). "At home" iff `cwd`'s ADAPTER-LEXICAL canonicalization
/// (§5.x, no symlink resolution) equals `$HOME`'s. An unknown `cwd` or unset
/// `$HOME` defaults to **NOT at home** (inject) — the synapse tiebreaker's own
/// bias: a missing floor advisory is the costly direction, a stray duplicate is
/// merely cosmetic.
fn is_at_home(cwd: Option<&Path>) -> bool {
    let (Some(cwd), Some(home)) = (cwd, std::env::var_os("HOME")) else {
        return false;
    };
    let home = PathBuf::from(home);
    canonicalize_lexical(cwd, None) == canonicalize_lexical(&home, None)
}

// =============================================================================
// The maintenance-due signal (§8; P8 exposes it, P12/WP-6 owns the pass)
// =============================================================================

/// How many NEW telemetry records trigger the maintenance pass (§8, hardcoded
/// count-form; `curation::maintain`, WP-6, re-verifies this UNDER THE LOCK,
/// WR-02 — this is only the session-start "is it worth checking" signal).
pub const MAINTENANCE_TRIGGER_COUNT: u64 = 50;
/// The maintenance-state filename (infra: underscore-prefixed — a store scan
/// skips it, `crate::rebuild::is_infra`).
pub const MAINTENANCE_STATE_FILENAME: &str = "_maintenance_state.json";

/// The maintenance pass's persisted state — read-only from this packet. WP-6
/// owns writing it (claim-before-mutate, CORE-SPEC §8 WR-01).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct MaintenanceState {
    /// The live telemetry file's physical line count as of the last pass.
    #[serde(rename = "lastPassLine", default)]
    last_pass_line: u64,
}

/// Whether the self-curation maintenance pass is due at this session start
/// (§8): the LIVE telemetry file has grown by `>= MAINTENANCE_TRIGGER_COUNT`
/// (50) physical lines since `_maintenance_state.json`'s `lastPassLine`. A
/// rotation-shrunk line count (the live file rotated since the last pass) is
/// treated as a full reset — `delta = current` — matching the proven synapse
/// hook's handling. Read-only and fail-open: an absent/malformed state file
/// reads as `lastPassLine: 0`; an unreadable telemetry file reads as 0 lines.
///
/// This packet computes and exposes the signal; it does NOT call the pass
/// (`curation::maintain`, WP-6, is not wired in this worktree — see the
/// `// INTEGRATOR:` marker in [`dispatch_session_start`]).
pub fn maintenance_due(store: &Path, telemetry: &Telemetry) -> bool {
    let current = telemetry_line_count(telemetry.telemetry_file());
    let last_pass_line = read_maintenance_state(store).last_pass_line;
    let delta = current.checked_sub(last_pass_line).unwrap_or(current);
    delta >= MAINTENANCE_TRIGGER_COUNT
}

/// The live telemetry file's physical line count (newline-delimited). An
/// unreadable file (absent, permissions) reads as 0 — fail-open.
fn telemetry_line_count(path: &Path) -> u64 {
    std::fs::read(path)
        .map(|bytes| bytes.iter().filter(|&&b| b == b'\n').count() as u64)
        .unwrap_or(0)
}

/// Read `_maintenance_state.json` under `store`. Absent / malformed → the
/// default (`lastPassLine: 0`) — fail-open, never a hook-path error.
fn read_maintenance_state(store: &Path) -> MaintenanceState {
    std::fs::read_to_string(store.join(MAINTENANCE_STATE_FILENAME))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_dir(tag: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d =
            std::env::temp_dir().join(format!("rejolt-hook-t-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // ---- is_at_home --------------------------------------------------------

    #[test]
    fn at_home_matches_lexical_home_and_defaults_to_not_at_home() {
        let Some(home) = std::env::var_os("HOME") else {
            return; // no HOME on this host: nothing to assert
        };
        let home = PathBuf::from(home);
        // GOOD: cwd == $HOME (even with a `.`/`..` detour) -> at home.
        assert!(is_at_home(Some(&home)));
        assert!(is_at_home(Some(&home.join("sub/.."))));
        // BAD: a different cwd -> not at home.
        assert!(!is_at_home(Some(Path::new("/tmp/some/other/project"))));
        // BAD: no cwd at all -> defaults to NOT at home (inject, the costly-miss
        // direction is worse than a stray duplicate).
        assert!(!is_at_home(None));
    }

    // ---- store resolution helpers -------------------------------------------

    #[test]
    fn is_write_tool_is_the_closed_a5c_set() {
        for t in ["Write", "Edit", "MultiEdit"] {
            assert!(is_write_tool(t), "{t} is write-capable");
        }
        for t in ["Read", "Bash", "WebSearch", "WebFetch"] {
            assert!(!is_write_tool(t), "{t} is not write-capable");
        }
    }

    #[test]
    fn store_memory_id_classifies_infra_vs_memory() {
        let store = test_dir("memid");
        assert_eq!(
            store_memory_id(&store, &store.join("gpu-notes.md"), None),
            Some("gpu-notes".to_string())
        );
        // Infra files are not memories.
        assert_eq!(
            store_memory_id(&store, &store.join("MEMORY.md"), None),
            None
        );
        assert_eq!(
            store_memory_id(&store, &store.join("_grammar.toml"), None),
            None
        );
        // Outside the store entirely.
        assert_eq!(
            store_memory_id(&store, Path::new("/elsewhere/gpu-notes.md"), None),
            None
        );
        let _ = std::fs::remove_dir_all(&store);
    }

    // ---- maintenance_due -----------------------------------------------------

    fn telemetry_at(dir: &Path) -> Telemetry {
        Telemetry::new(
            dir.join("rt"),
            dir.join("_recall_telemetry.jsonl"),
            Config::default(),
        )
    }

    #[test]
    fn maintenance_due_matrix() {
        let store = test_dir("maint");
        let tel = telemetry_at(&store);

        // BAD: no telemetry file at all -> 0 lines -> not due.
        assert!(!maintenance_due(&store, &tel));

        // BAD: 49 lines, no prior state -> not due (just under the threshold).
        write_lines(&store, 49);
        assert!(!maintenance_due(&store, &tel));

        // GOOD: 50 lines, no prior state (lastPassLine defaults to 0) -> due.
        write_lines(&store, 50);
        assert!(maintenance_due(&store, &tel));

        // BAD: a prior pass state advances the baseline -> not due until +50 more.
        std::fs::write(
            store.join(MAINTENANCE_STATE_FILENAME),
            r#"{"lastPassLine": 50}"#,
        )
        .unwrap();
        assert!(!maintenance_due(&store, &tel));
        write_lines(&store, 99);
        assert!(!maintenance_due(&store, &tel));
        write_lines(&store, 100);
        assert!(maintenance_due(&store, &tel));

        // GOOD: a rotation-shrunk line count (current < lastPassLine) resets the
        // baseline to `delta = current`, matching the proven synapse handling.
        std::fs::write(
            store.join(MAINTENANCE_STATE_FILENAME),
            r#"{"lastPassLine": 500}"#,
        )
        .unwrap();
        write_lines(&store, 50); // rotated: far fewer lines than lastPassLine
        assert!(
            maintenance_due(&store, &tel),
            "a rotation-shrunk count must reset, not go negative-and-never-fire"
        );

        let _ = std::fs::remove_dir_all(&store);
    }

    fn write_lines(store: &Path, n: usize) {
        let mut s = String::new();
        for i in 0..n {
            s.push_str(&format!("{{\"ts\":{i},\"signal\":\"session\"}}\n"));
        }
        std::fs::write(store.join("_recall_telemetry.jsonl"), s).unwrap();
    }

    // `resolve_store` itself reads process-global env vars
    // (`config::default_config_path` → `XDG_CONFIG_HOME`/`HOME`); mutating those
    // in-process races other tests reading `HOME` under the harness (see
    // `normalize.rs`'s own note on why its tests read, never set, `HOME`). It is
    // exercised end-to-end instead by the subprocess-based integration tests in
    // `tests/hook_dispatch.rs`, where each `Command::env(...)` is per-process and
    // race-free.
}
