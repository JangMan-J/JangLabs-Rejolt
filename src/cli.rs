//! Command-line surface for `rejolt` (D20, D12, A5, D10; plan P15): one
//! multiplexed binary exposing nine subcommands plus a `hook` entry mode, each
//! with the FROZEN Appendix D flag / output / exit contract.
//!
//! ## Exit taxonomy (A5 / D20; direct-CLI modes)
//!
//! - **0** — ok.
//! - **1** — a failed check or operational failure (a guard DENY, a REGRESSED
//!   bench, validate findings, an I/O fault).
//! - **2** — usage / config / taxonomy (a clap parse error — clap exits 2 for us —
//!   a grammar validation error, or a malformed config / bad stdin JSON).
//!
//! Direct CLIs are **loud on success and fail closed on missing deps** (D12): a
//! human-facing report goes to stderr (or stdout for the plain report), and
//! machine output (`--json`, `--print-hooks`, the projection) goes to stdout.
//!
//! ## Wired vs not-yet-wired
//!
//! `bootstrap` (P14), `rebuild` (WP-2), `validate` (WP-1/WP-2), `check-write`
//! (WP-4), `project` (WP-4), `search` (WP-3), and `bench` (P13) are WIRED to their
//! engines. `maintain` / `seats` (WP-6) and the `hook` dispatch (WP-5) are
//! clearly-marked NOT-YET-WIRED stubs — their Appendix D flag/exit SURFACE is
//! defined here (so WP-8 consumes it verbatim), but the engine bodies land in
//! WP-5 / WP-6.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::bench;
use crate::bootstrap;
use crate::catalog::{ArtifactRead, read_artifacts};
use crate::config::{self, LoadedConfig};
use crate::frontmatter::Triggers;
use crate::grammar;
use crate::guard::{GuardConfig, GuardVerdict, StoreRoots, check_write};
use crate::hooks;
use crate::index::Index;
use crate::normalize::parse_host_event;
use crate::projection::{Projection, project};
use crate::rebuild::{
    BuildConfig, RebuildError, RebuildOutcome, index_path, rebuild, report_path, scan_store,
};
use crate::recall::{Advisory, RecallOutcome, recall};
use crate::telemetry::Telemetry;

/// Exit code: ok.
const EXIT_OK: i32 = 0;
/// Exit code: a failed check or operational failure.
const EXIT_FAIL: i32 = 1;
/// Exit code: usage / config / taxonomy.
const EXIT_USAGE: i32 = 2;

/// Top-level `rejolt` CLI parser.
#[derive(Debug, Parser)]
#[command(
    name = "rejolt",
    version,
    about = "Routed-memory reseed engine.",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// The nine D20 subcommands plus the `hook` adapter entry mode (Appendix D).
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Seed a clean, empty store (P14).
    Bootstrap {
        /// The store directory to seed.
        #[arg(long)]
        store: PathBuf,
        /// The (lab) grammar file. Seeded as the empty version line if absent.
        #[arg(long)]
        grammar: PathBuf,
        /// Emit the host hook settings block to stdout (the engine never writes it).
        #[arg(long)]
        print_hooks: bool,
    },
    /// Rebuild the compiled routing artifacts from the store (WP-2).
    Rebuild {
        /// The store directory.
        #[arg(long)]
        store: PathBuf,
        /// Emit the routability report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Validate the grammar + store (WP-1 / WP-2).
    Validate {
        /// The store directory.
        #[arg(long)]
        store: PathBuf,
        /// The grammar file (default: the store-side `_grammar.toml`).
        #[arg(long)]
        grammar: Option<PathBuf>,
    },
    /// Write-guard check for a full-file memory write (WP-4). Content on stdin.
    CheckWrite {
        /// The store directory.
        #[arg(long)]
        store: PathBuf,
        /// The write target path.
        #[arg(long)]
        target: PathBuf,
    },
    /// Collision projection for a proposed trigger set (WP-4). Triggers JSON on stdin.
    Project {
        /// The store directory.
        #[arg(long)]
        store: PathBuf,
    },
    /// Recall probe over a host event (WP-3). Event JSON on stdin.
    Search {
        /// The store directory.
        #[arg(long)]
        store: PathBuf,
        /// Emit results as JSON.
        #[arg(long)]
        json: bool,
        /// Seat-probe: exit 1 if this memory id is NOT surfaced.
        #[arg(long)]
        expect: Option<String>,
    },
    /// Self-curation maintenance pass (WP-6 — not yet wired).
    Maintain {
        /// The store directory.
        #[arg(long)]
        store: PathBuf,
        /// Force the pass even if the record-count trigger is not met.
        #[arg(long)]
        force: bool,
    },
    /// Seat governance report / proposal (WP-6 — not yet wired).
    Seats {
        /// The store directory.
        #[arg(long)]
        store: PathBuf,
        /// Write a PENDING-SEAT-CHANGES proposal block.
        #[arg(long)]
        propose: bool,
    },
    /// Performance bench + calibration (P13).
    Bench {
        /// The store directory.
        #[arg(long)]
        store: PathBuf,
        /// How many recall samples to time (default 200).
        #[arg(long)]
        samples: Option<usize>,
        /// Rewrite the committed p95 baseline to the current measurement.
        #[arg(long)]
        update_baseline: bool,
        /// Derive + commit a full calibration baseline (A4).
        #[arg(long)]
        calibrate: bool,
    },
    /// Host hook entry: `rejolt hook <event>` (WP-5 — dispatch not yet wired).
    Hook {
        /// The host lifecycle event being dispatched (payload arrives on stdin).
        #[arg(value_enum)]
        event: HookEvent,
    },
}

/// The frozen v1 hook event set (D19): `session-start` / `pre-op` / `post-op`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HookEvent {
    /// Session lifecycle start.
    SessionStart,
    /// Pre-operation (before a tool call commits).
    PreOp,
    /// Post-operation (after a tool call).
    PostOp,
}

/// Parse argv and dispatch. Returns the process exit code. A clap parse error
/// (usage) exits 2 before this returns (clap's default), matching the taxonomy.
pub fn run() -> i32 {
    let cli = Cli::parse();
    dispatch(&cli.command)
}

/// Dispatch a parsed command to its handler.
pub fn dispatch(command: &Command) -> i32 {
    match command {
        Command::Bootstrap {
            store,
            grammar,
            print_hooks,
        } => cmd_bootstrap(store, grammar, *print_hooks),
        Command::Rebuild { store, json } => cmd_rebuild(store, *json),
        Command::Validate { store, grammar } => cmd_validate(store, grammar.as_deref()),
        Command::CheckWrite { store, target } => cmd_check_write(store, target),
        Command::Project { store } => cmd_project(store),
        Command::Search {
            store,
            json,
            expect,
        } => cmd_search(store, *json, expect.as_deref()),
        Command::Maintain { store, force } => cmd_maintain(store, *force),
        Command::Seats { store, propose } => cmd_seats(store, *propose),
        Command::Bench {
            store,
            samples,
            update_baseline,
            calibrate,
        } => cmd_bench(store, *samples, *update_baseline, *calibrate),
        Command::Hook { event } => cmd_hook(*event),
    }
}

// =============================================================================
// Shared: config + grammar resolution + stdin
// =============================================================================

/// Load the store's `config.toml` for a direct CLI (loud). On a config/taxonomy
/// error, print it and return the exit-2 code as `Err`. Warnings are printed to
/// stderr (R7: advisory, never fatal).
fn load_config(store: &Path) -> Result<LoadedConfig, i32> {
    match config::load(&config::config_path(store)) {
        Ok(loaded) => {
            for w in &loaded.warnings {
                eprintln!("{w}");
            }
            Ok(loaded)
        }
        Err(e) => {
            eprintln!("rejolt: {e}");
            Err(e.exit_code())
        }
    }
}

/// Resolve the grammar path: an explicit `--grammar` flag, else `config.grammarPath`,
/// else the store-side `_grammar.toml`.
fn resolve_grammar(store: &Path, explicit: Option<&Path>, config: &config::Config) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Some(p) = &config.grammar_path {
        return p.clone();
    }
    bootstrap::store_grammar_path(store)
}

/// Read all of stdin as a UTF-8 string (best-effort; a read fault yields `""`).
fn read_stdin() -> String {
    std::io::read_to_string(std::io::stdin()).unwrap_or_default()
}

// =============================================================================
// bootstrap (P14)
// =============================================================================

fn cmd_bootstrap(store: &Path, grammar: &Path, print_hooks: bool) -> i32 {
    // Bootstrap runs before a config.toml is likely to exist; an absent file is fine.
    let loaded = match load_config(store) {
        Ok(l) => l,
        Err(code) => return code,
    };
    match bootstrap::bootstrap(store, grammar, &loaded.config) {
        Ok(report) => {
            print_bootstrap_report(store, &report);
            // A structural fail-open contract break is a failed check (exit 1); an
            // unwritable mark dir is advisory only (still exit 0).
            let code = if report.verification.structural_ok() {
                EXIT_OK
            } else {
                EXIT_FAIL
            };
            // --print-hooks: emit the settings block to STDOUT (D13: the engine
            // NEVER writes host settings; it only prints them for the human).
            if print_hooks {
                print!("{}", hooks::render_print_hooks(&hooks::current_bin()));
            }
            code
        }
        Err(e) => {
            eprintln!("rejolt bootstrap: {e}");
            e.exit_code()
        }
    }
}

/// The loud creation report (→ stderr, so `--print-hooks` stdout stays pure JSON).
fn print_bootstrap_report(store: &Path, report: &bootstrap::BootstrapReport) {
    let said = |created: bool| {
        if created {
            "created"
        } else {
            "already present"
        }
    };
    eprintln!(
        "rejolt bootstrap: store {} — {}",
        store.display(),
        said(report.store_created)
    );
    eprintln!("  grammar seed: {}", said(report.grammar_seeded));
    eprintln!(
        "  store grammar symlink `_grammar.toml`: {}",
        said(report.grammar_symlinked)
    );
    eprintln!("  MEMORY.md router: {}", said(report.memory_created));
    eprintln!(
        "  rebuild: generation {} — {} unroutable, {} excluded",
        report.rebuild.generation, report.rebuild.unroutable_count, report.rebuild.excluded_count
    );
    for a in &report.rebuild.drift_advisories {
        eprintln!("  drift: {a}");
    }
    let v = &report.verification;
    // The structural rows are tri-state: passed / broke / skipped (advisory).
    let row = |r: Option<bool>| match r {
        Some(true) => "pass",
        Some(false) => "BROKE",
        None => "skipped",
    };
    eprintln!(
        "  verify: mark-dir writable={}, missing-catalog fail-open={}, .surface-disabled wired={}",
        v.mark_dir_writable,
        row(v.missing_catalog_failopen),
        row(v.surface_disabled_wired)
    );
    if let Some(adv) = &v.inert_advisory {
        eprintln!("  {adv}");
    }
    if report.verification.structural_ok() {
        eprintln!("rejolt bootstrap: OK");
    } else {
        eprintln!("rejolt bootstrap: FAILED a structural fail-open check");
    }
}

// =============================================================================
// rebuild (WP-2)
// =============================================================================

/// The `--json` rebuild summary.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RebuildJson<'a> {
    generation: &'a str,
    unroutable_count: usize,
    unroutable_ids: &'a [String],
    excluded_count: usize,
    drift_advisories: &'a [String],
}

fn cmd_rebuild(store: &Path, json: bool) -> i32 {
    let loaded = match load_config(store) {
        Ok(l) => l,
        Err(code) => return code,
    };
    let grammar = resolve_grammar(store, None, &loaded.config);
    let build_cfg = BuildConfig {
        max_description_chars: loaded.config.max_description_chars,
    };
    match rebuild(store, &grammar, &build_cfg) {
        Ok(outcome) => {
            if json {
                let j = RebuildJson {
                    generation: &outcome.generation,
                    unroutable_count: outcome.unroutable_count,
                    unroutable_ids: &outcome.unroutable_ids,
                    excluded_count: outcome.excluded_count,
                    drift_advisories: &outcome.drift_advisories,
                };
                println!("{}", serde_json::to_string_pretty(&j).unwrap_or_default());
            } else {
                print_rebuild_human(&outcome);
            }
            EXIT_OK
        }
        Err(RebuildError::Grammar(g)) => {
            eprintln!("rejolt rebuild: grammar error (config/taxonomy): {g}");
            EXIT_USAGE
        }
        Err(RebuildError::Io(e)) => {
            eprintln!("rejolt rebuild: I/O error: {e}");
            EXIT_FAIL
        }
    }
}

fn print_rebuild_human(outcome: &RebuildOutcome) {
    println!(
        "rejolt rebuild: OK — generation {}, {} unroutable, {} excluded",
        outcome.generation, outcome.unroutable_count, outcome.excluded_count
    );
    for id in &outcome.unroutable_ids {
        println!("  unroutable: {id}");
    }
    for a in &outcome.drift_advisories {
        println!("  drift: {a}");
    }
}

// =============================================================================
// validate (WP-1 / WP-2)
// =============================================================================

fn cmd_validate(store: &Path, grammar_flag: Option<&Path>) -> i32 {
    let loaded = match load_config(store) {
        Ok(l) => l,
        Err(code) => return code,
    };
    let grammar_path = resolve_grammar(store, grammar_flag, &loaded.config);

    // Grammar (config/taxonomy → exit 2).
    let grammar_text = match std::fs::read_to_string(&grammar_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "rejolt validate: grammar unreadable at {} ({e}) — config error",
                grammar_path.display()
            );
            return EXIT_USAGE;
        }
    };
    if let Err(g) = grammar::parse_and_validate(&grammar_text) {
        eprintln!("rejolt validate: grammar error (config/taxonomy): {g}");
        return EXIT_USAGE;
    }

    // Store findings (exit 1 if any).
    let mut findings: Vec<String> = Vec::new();
    match scan_store(store) {
        Ok((_memories, malformed)) => {
            for (name, _) in &malformed {
                findings.push(format!("malformed memory file (skipped): {name}"));
            }
        }
        Err(e) => findings.push(format!("store unreadable: {e}")),
    }
    match read_artifacts(&index_path(store), &report_path(store)) {
        ArtifactRead::Consistent(_) => {}
        ArtifactRead::Missing => {
            findings.push("no compiled index/report — run `rejolt rebuild`".to_string());
        }
        ArtifactRead::Stale(a) | ArtifactRead::Malformed(a) => findings.push(a),
    }

    if findings.is_empty() {
        println!("rejolt validate: OK — grammar valid, store clean");
        EXIT_OK
    } else {
        for f in &findings {
            println!("rejolt validate: finding: {f}");
        }
        eprintln!("rejolt validate: {} finding(s)", findings.len());
        EXIT_FAIL
    }
}

// =============================================================================
// check-write (WP-4)
// =============================================================================

fn cmd_check_write(store: &Path, target: &Path) -> i32 {
    let loaded = match load_config(store) {
        Ok(l) => l,
        Err(code) => return code,
    };
    let guard_cfg = GuardConfig {
        grammar_path: resolve_grammar(store, None, &loaded.config),
        roots: StoreRoots {
            box_root: loaded.config.store_roots.box_root.clone(),
        },
    };
    // A direct check-write is a FULL-file write (the content on stdin IS the whole
    // proposed file), so is_full_write = true.
    let content = read_stdin();
    match check_write(store, target, &content, true, &guard_cfg) {
        GuardVerdict::Allow => {
            println!("rejolt check-write: ALLOW — {}", target.display());
            EXIT_OK
        }
        GuardVerdict::Deny(reason) => {
            println!("rejolt check-write: DENY [{}] — {reason}", reason.code());
            EXIT_FAIL
        }
    }
}

// =============================================================================
// project (WP-4)
// =============================================================================

/// The `project` / recall trigger-set input DTO (triggers JSON on stdin).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TriggersInput {
    commands: Vec<String>,
    paths: Vec<String>,
    args: Vec<String>,
    synonyms: Vec<String>,
}

impl TriggersInput {
    fn into_triggers(self) -> Triggers {
        Triggers {
            commands: self.commands,
            paths: self.paths,
            args: self.args,
            synonyms: self.synonyms,
        }
    }
}

/// The projection output DTO (§7 fields).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionJson {
    collisions: Vec<String>,
    per_trigger: std::collections::BTreeMap<String, usize>,
    distinct_count: usize,
    live_levers: LiveLeversJson,
    verdict: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveLeversJson {
    args: Vec<String>,
    paths: Vec<String>,
    synonyms: Vec<String>,
}

fn projection_json(p: &Projection) -> ProjectionJson {
    let verdict = match p.verdict {
        crate::projection::Verdict::Pass => "PASS",
        crate::projection::Verdict::GuideBroad => "GUIDE-broad",
        crate::projection::Verdict::BlockDegenerate => "BLOCK-degenerate",
    };
    ProjectionJson {
        collisions: p.collisions.clone(),
        per_trigger: p.per_trigger.clone(),
        distinct_count: p.distinct_count,
        live_levers: LiveLeversJson {
            args: p.live_levers.args.clone(),
            paths: p.live_levers.paths.clone(),
            synonyms: p.live_levers.synonyms.clone(),
        },
        verdict: verdict.to_string(),
    }
}

fn cmd_project(store: &Path) -> i32 {
    // project reads only the index (no grammar/config beyond the store path); load
    // config so a malformed config still surfaces as usage/config (exit 2).
    if let Err(code) = load_config(store) {
        return code;
    }
    let stdin = read_stdin();
    let triggers: TriggersInput = match serde_json::from_str(&stdin) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("rejolt project: invalid triggers JSON: {e}");
            return EXIT_USAGE;
        }
    };
    let triggers = triggers.into_triggers();
    let read = read_artifacts(&index_path(store), &report_path(store));
    // Fail-open on any index fault: an empty index yields an empty projection.
    let projection = match read.loaded() {
        Some(l) => project(&triggers, &l.index),
        None => project(&triggers, &Index::default()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&projection_json(&projection)).unwrap_or_default()
    );
    EXIT_OK
}

// =============================================================================
// search (WP-3)
// =============================================================================

/// The `search --json` advisory output DTO.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchJson {
    result: &'static str,
    confidence: String,
    memories: Vec<SearchMemoryJson>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchMemoryJson {
    memory_id: String,
    path: String,
    snippet: String,
    score: i64,
    confidence: String,
    citations: Vec<String>,
}

fn search_json(a: &Advisory) -> SearchJson {
    SearchJson {
        result: "advisory",
        confidence: a.confidence.clone(),
        memories: a
            .memories
            .iter()
            .map(|m| SearchMemoryJson {
                memory_id: m.memory_id.clone(),
                path: m.path.clone(),
                snippet: m.snippet.clone(),
                score: m.score,
                confidence: m.confidence.clone(),
                citations: m.citations.iter().map(|c| c.render()).collect(),
            })
            .collect(),
    }
}

fn cmd_search(store: &Path, json: bool, expect: Option<&str>) -> i32 {
    let loaded = match load_config(store) {
        Ok(l) => l,
        Err(code) => return code,
    };
    let stdin = read_stdin();
    let value: serde_json::Value = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rejolt search: invalid event JSON: {e}");
            return EXIT_USAGE;
        }
    };
    let op = parse_host_event(&value);
    // `search` is a direct diagnostic/inspection probe (the D20 scope-lens surface):
    // it uses a THROWAWAY telemetry so it never pollutes the real dedup marks or the
    // store telemetry, and repeated probes are deterministic (never deduped). It
    // still carries the store's loaded config so recall ranks per the config; the
    // real fire-logging happens on the hook path (WP-5), not here.
    let tel = probe_telemetry(loaded.config);
    let outcome = recall(&op, store, &tel);

    match &outcome {
        RecallOutcome::Silence => {
            if json {
                println!("{{\"result\":\"silence\"}}");
            } else {
                println!("rejolt search: (silence — no memory match)");
            }
        }
        RecallOutcome::Advisory(a) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&search_json(a)).unwrap_or_default()
                );
            } else {
                print!("{}", a.text);
            }
        }
    }

    // --expect ID: the seat-probe form — exit 1 when the id is NOT surfaced.
    if let Some(id) = expect {
        let present = outcome
            .advisory()
            .is_some_and(|a| a.memories.iter().any(|m| m.memory_id == id));
        if present {
            EXIT_OK
        } else {
            eprintln!("rejolt search: --expect `{id}` was NOT surfaced (seat-probe)");
            EXIT_FAIL
        }
    } else {
        EXIT_OK
    }
}

/// A throwaway telemetry for the `search` diagnostic probe (a temp mark dir + temp
/// file), so a probe never writes to the real dedup dir or store telemetry. It
/// carries the store's `config` so recall's ranking honors any config override.
fn probe_telemetry(config: config::Config) -> Telemetry {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("rejolt-search-{}-{n}", std::process::id()));
    Telemetry::new(base.join("rt"), base.join("tel.jsonl"), config)
}

// =============================================================================
// bench (P13)
// =============================================================================

fn cmd_bench(store: &Path, samples: Option<usize>, update_baseline: bool, calibrate: bool) -> i32 {
    let loaded = match load_config(store) {
        Ok(l) => l,
        Err(code) => return code,
    };
    let samples = samples.unwrap_or(bench::DEFAULT_SAMPLES);
    match bench::run_bench(
        store,
        samples,
        update_baseline,
        calibrate,
        bench::SYNTHETIC_REFERENCE_N,
        &loaded.config,
    ) {
        Ok(outcome) => {
            // Loud advisories (env mismatch / WARN drift / calibration outputs) → stderr.
            for l in &outcome.loud {
                eprintln!("{l}");
            }
            if outcome.baseline_written {
                eprintln!(
                    "rejolt bench: baseline written to {}",
                    bench::baseline_path(store).display()
                );
            }
            // The verdict line + numbers → stdout.
            println!(
                "rejolt bench: {} — p95 {:.2} ms over {} samples",
                outcome.verdict.as_str(),
                outcome.p95_ms,
                outcome.samples
            );
            outcome.exit_code()
        }
        Err(e) => {
            eprintln!("rejolt bench: I/O error: {e}");
            EXIT_FAIL
        }
    }
}

// =============================================================================
// maintain / seats / hook — NOT-YET-WIRED stubs (surface defined; bodies later)
// =============================================================================

fn cmd_maintain(_store: &Path, _force: bool) -> i32 {
    // NOT-YET-WIRED (WP-6): the self-curation maintenance pass (≥50-record trigger
    // recheck-under-lock, claim-before-mutate, the three floors) lands in WP-6. The
    // Appendix D surface (`--store [--force]`, exits 0/1/2) is defined above. A
    // direct CLI must not masquerade as success, so the stub is loud and exits 1.
    eprintln!(
        "rejolt maintain: NOT YET WIRED (WP-6) — the --store/--force surface is defined; \
         the curation engine lands in WP-6."
    );
    EXIT_FAIL
}

fn cmd_seats(_store: &Path, _propose: bool) -> i32 {
    // NOT-YET-WIRED (WP-6): seat governance (PENDING-SEAT-CHANGES replace-not-stack,
    // the seat dual-gate) lands in WP-6. The Appendix D surface (`--store
    // [--propose]`, exits 0/1/2) is defined above. Loud, exits 1.
    eprintln!(
        "rejolt seats: NOT YET WIRED (WP-6) — the --store/--propose surface is defined; \
         seat governance lands in WP-6."
    );
    EXIT_FAIL
}

fn cmd_hook(event: HookEvent) -> i32 {
    // WIRED (WP-5): crate::hook::dispatch resolves the store (from the GLOBAL
    // config — no `--store` flag on this subcommand, Appendix D), applies the
    // `.surface-disabled` kill-switch, parses the stdin payload, and dispatches
    // to recall / write-guard / write-context / rebuild-refresh / read-signal /
    // the session marker + maintenance-due check. Exit taxonomy per A5: quiet
    // allow (0) / write-guard deny (2, short-circuit) — NEVER 1.
    crate::hook::dispatch(event)
}
