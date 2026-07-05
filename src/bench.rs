//! Bench + calibration machinery — the four-verdict regression gate (plan P13;
//! D9, D26, A4, A7, RB6, RB10; CORE-SPEC §9).
//!
//! `bench` times the WP-3 recall path over a corpus, computes p95, and renders one
//! of the four §9 verdicts against a committed, environment-fingerprinted baseline.
//! Only **REGRESSED** exits non-zero (§9). This module builds the MACHINERY; the
//! actual calibration numbers are OUTPUTS that land in a separate reviewable commit
//! at the first end-to-end run (R1) — **nothing here fabricates a number**.
//!
//! ## The four verdicts (§9)
//!
//! - **PASS** — `p95 ≤ ceiling` AND within the design budget.
//! - **WARN** — over the design budget but `≤ ceiling`; advisory, exit 0.
//! - **REGRESSED** — a CALIBRATED baseline (`ceiling_slack_ms > 0.0`) AND
//!   `p95 > ceiling`; a structural slowdown, exit 1 (blocks). An uncalibrated
//!   baseline has no valid jitter floor to gate on, so this check is inert
//!   (measure-only) until `--calibrate` runs.
//! - **NOBASELINE** — no baseline file; measure-only, exit 0. This is also the
//!   interim state until a baseline is committed — so it needs no special case.
//!
//! `ceiling = baseline_p95 + slack_floor` (D9/A4(c)). D9 supersedes CORE-SPEC
//! §9's static `max(25%, 15 ms)` slack; A4(c) defines the replacement as the
//! calibrated jitter floor ALONE — no static relative or absolute term. The two
//! "55 ms" are never conflated: `BUDGET_MS = 55` is the live-advisory design
//! budget (over it ⇒ WARN, exit 0), never a hard cliff.
//!
//! ## Calibration (A4 — the derivation is SPEC, the numbers are OUTPUTS)
//!
//! `--calibrate` PINS (never invents) the derivation:
//! - **design budget** = synthetic-1000 p95 × safety factor **3.0** (frozen).
//! - **ceiling slack floor** = `max(3×σ(p95), observed min→max p95 band)` over ≥5
//!   runs of ≥100 samples.
//! - writes a reviewable baseline carrying the **environment fingerprint** = CPU
//!   model + governor + power source. The **kernel is recorded metadata, NEVER a
//!   gate key** (RB10) — a rolling-kernel box must not trip the gate on every boot.
//! - measures the real telemetry record rate and recommends resizing `_TEL_MAX` if
//!   30 days does not fit the rotation bound (R7/RB6).
//!
//! On an environment-fingerprint **mismatch** OR a **missing** baseline the gate is
//! measure-only AND **LOUD** — silent degradation is a conformance failure (A4).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::index::IndexRecord;
use crate::normalize::{NormalizedOp, ToolOp};
use crate::rebuild::{BuildConfig, index_path, rebuild};
use crate::recall::recall;
use crate::telemetry::{Telemetry, telemetry_path};
use crate::tier::Axis;

/// The frozen calibration safety factor (A4): design budget = synth-1000 p95 × 3.0.
pub const SAFETY_FACTOR: f64 = 3.0;
/// The reference corpus size for the design-budget synthetic run (A4).
pub const SYNTHETIC_REFERENCE_N: usize = 1000;
/// Calibration requires ≥5 runs (A4) for the slack floor.
pub const CALIBRATION_MIN_RUNS: usize = 5;
/// Calibration requires ≥100 samples per run (A4).
pub const CALIBRATION_MIN_SAMPLES: usize = 100;
/// The default `--samples` for a plain `bench` run.
pub const DEFAULT_SAMPLES: usize = 200;
/// The baseline file (infra: underscore-prefixed so the store scan skips it).
pub const BASELINE_FILENAME: &str = "_recall_p95_baseline.toml";

// =============================================================================
// Environment fingerprint (A4 / RB10) — kernel is metadata, never a gate key
// =============================================================================

/// The environment fingerprint the baseline carries (A4). The **gate key** is
/// `(cpu_model, governor, power_source)`; `kernel` is recorded metadata and is
/// **NEVER** part of the gate key (RB10) — this box is a rolling-kernel box, so a
/// kernel bump must not trip the perf gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EnvFingerprint {
    /// CPU model (`/proc/cpuinfo` "model name"). A gate key.
    pub cpu_model: String,
    /// CPU frequency governor (`scaling_governor`). A gate key.
    pub governor: String,
    /// Power source (`AC` / `battery` / `unknown`). A gate key.
    pub power_source: String,
    /// Kernel release (`/proc/sys/kernel/osrelease`). **Metadata only — NEVER a
    /// gate key** (RB10).
    pub kernel: String,
}

impl EnvFingerprint {
    /// Detect the live environment fingerprint. Every field fails soft to
    /// `"unknown"` (containers / cronless boxes lack cpufreq or power_supply).
    pub fn detect() -> EnvFingerprint {
        EnvFingerprint {
            cpu_model: detect_cpu_model(),
            governor: detect_governor(),
            power_source: detect_power_source(),
            kernel: detect_kernel(),
        }
    }

    /// The gate key: `(cpu_model, governor, power_source)`. The kernel is
    /// deliberately excluded (RB10) — it is recorded, never gated on.
    pub fn gate_key(&self) -> (&str, &str, &str) {
        (&self.cpu_model, &self.governor, &self.power_source)
    }

    /// Whether two fingerprints share a gate key (kernel differences are ignored).
    pub fn matches(&self, other: &EnvFingerprint) -> bool {
        self.gate_key() == other.gate_key()
    }
}

fn first_nonempty_line_field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix(key)?;
        let v = rest.trim_start_matches([':', ' ', '\t']).trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

fn detect_cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| first_nonempty_line_field(&t, "model name"))
        .unwrap_or_else(|| "unknown".to_string())
}

fn detect_governor() -> String {
    read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .unwrap_or_else(|| "unknown".to_string())
}

fn detect_kernel() -> String {
    read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_else(|| "unknown".to_string())
}

fn detect_power_source() -> String {
    let base = Path::new("/sys/class/power_supply");
    let Ok(entries) = fs::read_dir(base) else {
        return "unknown".to_string();
    };
    let mut has_battery = false;
    for e in entries.flatten() {
        let p = e.path();
        let typ = read_trimmed(p.join("type")).unwrap_or_default();
        if typ == "Mains" {
            if read_trimmed(p.join("online")).as_deref() == Some("1") {
                return "AC".to_string();
            }
        } else if typ == "Battery" {
            has_battery = true;
        }
    }
    if has_battery {
        "battery".to_string()
    } else {
        "unknown".to_string()
    }
}

fn read_trimmed(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// =============================================================================
// The committed baseline (reviewable; carries the env fingerprint)
// =============================================================================

/// The committed, reviewable perf baseline (A4). Serialized as TOML so a
/// `--update-baseline` / `--calibrate` write is a legible diff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Baseline {
    /// The accepted steady-state recall p95 (ms) — the regression anchor (§9).
    pub p95_ms: f64,
    /// The design budget (ms) = synthetic-1000 p95 × [`SAFETY_FACTOR`] (A4).
    pub design_budget_ms: f64,
    /// The ceiling slack floor (ms) = `max(3σ, min→max band)` over the calibration
    /// runs (A4). Recorded for auditability alongside the §9 relative ceiling.
    pub ceiling_slack_ms: f64,
    /// The environment fingerprint this baseline was measured under.
    pub env: EnvFingerprint,
}

impl Baseline {
    /// Load a baseline from `path`. `None` on a missing/unparseable file (→ the gate
    /// reads NOBASELINE — measure-only).
    ///
    /// Walk-back fix F7 (2026-07-04, A4(e)): a PARSEABLE baseline carrying
    /// non-finite or negative magnitudes (TOML accepts `nan`/`inf`) is nonsense,
    /// not a gate input — `nan > 0.0` is false, so a NaN slack would silently
    /// disarm REGRESSED, and an `inf` budget silently never WARNs. Such a file
    /// loads as `None`, landing on the LOUD NOBASELINE advisory instead of a
    /// silent degrade.
    pub fn load(path: &Path) -> Option<Baseline> {
        let text = fs::read_to_string(path).ok()?;
        let b: Baseline = toml::from_str(&text).ok()?;
        let sane = |v: f64| v.is_finite() && v >= 0.0;
        (sane(b.p95_ms) && sane(b.design_budget_ms) && sane(b.ceiling_slack_ms)).then_some(b)
    }

    /// Write the baseline as reviewable TOML (with a header comment).
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let body = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let text = format!(
            "# rejolt recall p95 baseline — reviewable calibration output (A4).\n\
             # The kernel field is recorded metadata ONLY and is NEVER a gate key (RB10).\n\
             {body}"
        );
        crate::catalog::write_atomic(path, &text)
    }
}

/// The baseline path under a store.
pub fn baseline_path(store_dir: &Path) -> PathBuf {
    store_dir.join(BASELINE_FILENAME)
}

// =============================================================================
// Pure arithmetic (§9 ceiling; A4 derivation) — unit-tested directly
// =============================================================================

/// The nearest-rank p95 over `samples` (ms). Empty → 0.0.
pub fn percentile_95(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Nearest-rank: rank = ceil(0.95 · n), 1-based → 0-based index rank-1.
    let rank = (0.95 * sorted.len() as f64).ceil() as usize;
    let idx = rank.clamp(1, sorted.len()) - 1;
    sorted[idx]
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Population standard deviation of `xs`. Empty → 0.0.
pub fn stddev(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let m = mean(xs);
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / xs.len() as f64;
    var.sqrt()
}

/// The design budget (A4): synthetic-1000 p95 × [`SAFETY_FACTOR`] (3.0, frozen).
pub fn design_budget_ms(synthetic_p95: f64) -> f64 {
    synthetic_p95 * SAFETY_FACTOR
}

/// The ceiling slack floor (A4): `max(3×σ(p95), observed min→max p95 band)` over the
/// calibration runs. Empty / single-run → 0.0.
pub fn ceiling_slack_floor(run_p95s: &[f64]) -> f64 {
    if run_p95s.len() < 2 {
        return 0.0;
    }
    let three_sigma = 3.0 * stddev(run_p95s);
    let max = run_p95s.iter().cloned().fold(f64::MIN, f64::max);
    let min = run_p95s.iter().cloned().fold(f64::MAX, f64::min);
    three_sigma.max(max - min)
}

/// The regression ceiling (D9/A4(c)): `baseline_p95 + slack_floor`, where
/// `slack_floor` is the baseline's `ceiling_slack_ms` — the A4(c)-calibrated
/// `max(3σ, min→max band)` over the calibration runs. D9 explicitly
/// **supersedes** CORE-SPEC §9's static `max(25%, 15 ms)` slack ("all
/// magnitudes deferred to the D26 calibration protocol"), and A4(c) defines the
/// replacement as the calibrated jitter floor ALONE — no static relative or
/// absolute term. At this reseed's sub-millisecond recall scale (D16: 0.7–2.4
/// ms) a static 15 ms term would swamp any realistic structural regression, so
/// it must not be transplanted here (A4's own rationale). A baseline with no
/// calibrated slack (`0.0`, e.g. an uncalibrated `--update-baseline`) reduces
/// this to a pure pass-through of `baseline_p95` — see [`verdict_of`], which
/// treats that case as measure-only rather than gating on it.
pub fn regression_ceiling(baseline_p95: f64, slack_floor: f64) -> f64 {
    baseline_p95 + slack_floor
}

/// The R7 `_TEL_MAX` resize recommendation: the smallest `_TEL_MAX` (bytes) such
/// that ~2× of it spans at least `window_days` at the measured byte rate. `None`
/// when the current `_TEL_MAX` already fits 30 days, or when the record rate is
/// unmeasurable (`span_days <= 0`).
pub fn recommended_tel_max(
    total_bytes: u64,
    span_days: f64,
    window_days: u64,
    current_tel_max: u64,
) -> Option<u64> {
    if span_days <= 0.0 || total_bytes == 0 {
        return None;
    }
    let bytes_per_day = total_bytes as f64 / span_days;
    // rotation bound ≈ 2×_TEL_MAX span; require it to cover window_days.
    let needed = (bytes_per_day * window_days as f64 / 2.0).ceil() as u64;
    (needed > current_tel_max).then_some(needed)
}

// =============================================================================
// Verdict
// =============================================================================

/// The four §9 verdicts. Only [`Verdict::Regressed`] exits non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// `p95 ≤ ceiling` AND within the design budget.
    Pass,
    /// Over the design budget but `≤ ceiling` — advisory, exit 0.
    Warn,
    /// `p95 > ceiling` — a structural slowdown, exit 1.
    Regressed,
    /// No usable baseline (absent, or an environment-fingerprint mismatch) —
    /// measure-only, exit 0, and LOUD (A4).
    NoBaseline,
}

impl Verdict {
    /// The verdict token printed on the bench verdict line.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Warn => "WARN",
            Verdict::Regressed => "REGRESSED",
            Verdict::NoBaseline => "NOBASELINE",
        }
    }
}

/// Render the verdict for a measured `p95` against `baseline` under `current_env`.
/// Returns the verdict plus the LOUD advisory lines (A4: env-mismatch / missing
/// baseline / warn drift are never silent). A missing baseline or a gate-key
/// mismatch both yield NOBASELINE (measure-only) + a loud line naming the
/// degradation and the recalibration step.
pub fn verdict_of(
    p95_ms: f64,
    baseline: Option<&Baseline>,
    current_env: &EnvFingerprint,
) -> (Verdict, Vec<String>) {
    let Some(b) = baseline else {
        return (
            Verdict::NoBaseline,
            vec![
                "bench: NOBASELINE — no committed p95 baseline; measure-only (exit 0). \
                 Run `rejolt bench --store DIR --calibrate` to derive and commit one."
                    .to_string(),
            ],
        );
    };
    if !b.env.matches(current_env) {
        // LOUD degrade (A4): silent degradation is a conformance failure.
        let (cbc, cbg, cbp) = b.env.gate_key();
        let (cc, cg, cp) = current_env.gate_key();
        return (
            Verdict::NoBaseline,
            vec![format!(
                "bench: ENVIRONMENT FINGERPRINT MISMATCH — the perf gate is measure-only (exit 0), \
                 NOT a regression gate. baseline=[cpu={cbc}, governor={cbg}, power={cbp}] vs \
                 current=[cpu={cc}, governor={cg}, power={cp}] (kernel is metadata, never a gate \
                 key). Recalibrate with `rejolt bench --store DIR --calibrate`."
            )],
        );
    }
    // FIX 2 (A4c)/D9: the ceiling is the calibrated slack floor ALONE — the
    // §9 static max(25%, 15 ms) is superseded, not folded in underneath.
    let ceiling = regression_ceiling(b.p95_ms, b.ceiling_slack_ms);
    if b.ceiling_slack_ms > 0.0 && p95_ms > ceiling {
        (
            Verdict::Regressed,
            vec![format!(
                "bench: REGRESSED — p95 {p95_ms:.2} ms > ceiling {ceiling:.2} ms \
                 (baseline {:.2} ms + calibrated slack {:.2} ms); a structural slowdown, exit 1.",
                b.p95_ms, b.ceiling_slack_ms
            )],
        )
    } else if b.design_budget_ms > 0.0 && p95_ms > b.design_budget_ms {
        // WARN only against a CALIBRATED design budget (> 0). An inert budget (0.0 —
        // an update-baseline with no prior calibration, FIX 5) never trips WARN; only
        // `--calibrate` sets a real synthetic-1000-derived budget (A4a).
        (
            Verdict::Warn,
            vec![format!(
                "bench: WARN — p95 {p95_ms:.2} ms over the design budget {:.2} ms but ≤ ceiling \
                 {ceiling:.2} ms (advisory, exit 0). Accept the drift with \
                 `rejolt bench --store DIR --update-baseline`.",
                b.design_budget_ms
            )],
        )
    } else {
        (Verdict::Pass, Vec::new())
    }
}

// =============================================================================
// The bench run
// =============================================================================

/// The outcome of a bench run (the CLI renders + maps to an exit code).
#[derive(Debug, Clone)]
pub struct BenchOutcome {
    /// The measured recall p95 (ms).
    pub p95_ms: f64,
    /// How many samples were timed.
    pub samples: usize,
    /// The §9 verdict.
    pub verdict: Verdict,
    /// The LOUD advisory lines (env mismatch, WARN drift, calibration outputs,
    /// `_TEL_MAX` resize recommendation) — every one is surfaced (D12/A4).
    pub loud: Vec<String>,
    /// Whether this run wrote/rewrote the baseline.
    pub baseline_written: bool,
}

impl BenchOutcome {
    /// The process exit code: only REGRESSED is non-zero (§9).
    pub fn exit_code(&self) -> i32 {
        match self.verdict {
            Verdict::Regressed => 1,
            _ => 0,
        }
    }
}

/// Run the bench gate over `store` (§9). Times the WP-3 recall path, computes p95,
/// optionally calibrates / updates the baseline, and renders the four-verdict gate.
/// `synthetic_n` is the reference-corpus size for the calibrate design budget (the
/// CLI passes [`SYNTHETIC_REFERENCE_N`]; tests pass a small n). Nothing here
/// fabricates a number — every value is measured.
pub fn run_bench(
    store: &Path,
    samples: usize,
    update_baseline: bool,
    calibrate: bool,
    synthetic_n: usize,
    config: &Config,
) -> std::io::Result<BenchOutcome> {
    let ops = ops_for_store(store);
    let p95 = measure_p95(store, &ops, samples, config);
    let mut loud: Vec<String> = Vec::new();
    let mut baseline_written = false;

    if calibrate {
        loud.extend(calibrate_and_write(store, &ops, synthetic_n, config)?);
        baseline_written = true;
    } else if update_baseline {
        loud.push(update_baseline_write(store, p95)?);
        baseline_written = true;
    }

    let baseline = Baseline::load(&baseline_path(store));
    let current_env = EnvFingerprint::detect();
    let (verdict, vloud) = verdict_of(p95, baseline.as_ref(), &current_env);
    loud.extend(vloud);

    Ok(BenchOutcome {
        p95_ms: p95,
        samples,
        verdict,
        loud,
        baseline_written,
    })
}

/// `--calibrate` (A4): derive the design budget from a synthetic-1000 run, the
/// slack floor from ≥5×≥100-sample real-store runs, record the env fingerprint,
/// and recommend a `_TEL_MAX` resize (R7). Writes the reviewable baseline. Returns
/// the loud calibration report lines.
fn calibrate_and_write(
    store: &Path,
    ops: &[NormalizedOp],
    synthetic_n: usize,
    config: &Config,
) -> std::io::Result<Vec<String>> {
    // Design budget from the synthetic-N reference corpus (built in a temp store).
    let synth = SyntheticStore::build(synthetic_n, config)?;
    let synth_p95 = measure_p95(&synth.dir, &synth.ops, CALIBRATION_MIN_SAMPLES, config);
    let budget = design_budget_ms(synth_p95);

    // Slack floor from ≥5 runs of ≥100 samples on the REAL store.
    let mut run_p95s = Vec::with_capacity(CALIBRATION_MIN_RUNS);
    for _ in 0..CALIBRATION_MIN_RUNS {
        run_p95s.push(measure_p95(store, ops, CALIBRATION_MIN_SAMPLES, config));
    }
    let slack = ceiling_slack_floor(&run_p95s);
    let baseline_p95 = percentile_95(&run_p95s.clone());

    let env = EnvFingerprint::detect();
    let baseline = Baseline {
        p95_ms: baseline_p95,
        design_budget_ms: budget,
        ceiling_slack_ms: slack,
        env: env.clone(),
    };
    baseline.write(&baseline_path(store))?;

    let mut loud = vec![
        format!(
            "bench: CALIBRATED — synthetic-{synthetic_n} p95 {synth_p95:.2} ms × {SAFETY_FACTOR} \
             = design budget {budget:.2} ms; slack floor max(3σ, band) = {slack:.2} ms; \
             baseline p95 {baseline_p95:.2} ms."
        ),
        format!(
            "bench: environment fingerprint recorded — cpu={}, governor={}, power={} \
             (kernel={} is metadata, NEVER a gate key).",
            env.cpu_model, env.governor, env.power_source, env.kernel
        ),
    ];

    // R7: measure the real telemetry record rate; recommend a _TEL_MAX resize if
    // 30 days does not fit the rotation bound.
    match measure_tel_rate(store) {
        Some((total_bytes, span_days)) => {
            match recommended_tel_max(
                total_bytes,
                span_days,
                config.telemetry_window_days,
                config.tel_max_bytes,
            ) {
                Some(rec) => loud.push(format!(
                    "bench: R7 — measured telemetry {total_bytes} B over {span_days:.2} d; \
                     current _TEL_MAX {} B does not fit {} d of telemetry — resize _TEL_MAX to \
                     ≥ {rec} B (set `_TEL_MAX = {rec}` in config.toml).",
                    config.tel_max_bytes, config.telemetry_window_days
                )),
                None => loud.push(format!(
                    "bench: R7 — measured telemetry {total_bytes} B over {span_days:.2} d; \
                     current _TEL_MAX {} B already fits {} d (no resize needed).",
                    config.tel_max_bytes, config.telemetry_window_days
                )),
            }
        }
        None => loud.push(
            "bench: R7 — insufficient telemetry to measure a record rate; _TEL_MAX left \
             unchanged (re-run calibration once telemetry has accrued)."
                .to_string(),
        ),
    }

    Ok(loud)
}

/// `--update-baseline` (§9): rewrite the REGRESSION baseline (real-store p95 + the
/// current env fingerprint), preserving a prior calibration's design budget + slack
/// ONLY within the same gate-key environment.
///
/// FIX 5 (A4a): the `design_budget_ms` comes ONLY from `--calibrate` (synthetic-1000
/// p95 × 3.0). With no prior calibration it is left **inert** (`0.0`) — NEVER
/// synthesized from the real store, whose p95 (especially the empty D17 store) would
/// otherwise write a sub-millisecond budget that spuriously WARNs every real run.
/// An inert (`0.0`) budget never trips WARN (see [`verdict_of`]); only the regression
/// ceiling gates until `--calibrate` runs.
///
/// Walk-back fix F5 (2026-07-04, A4(d)/(e)): the prior budget/slack are carried
/// forward ONLY when the prior baseline's env fingerprint matches the current
/// gate key. Carrying them across an env change would stamp the CURRENT
/// fingerprint onto magnitudes measured under a DIFFERENT one — laundering a
/// stale-env calibration past the very mismatch detector A4(d) installs, and
/// silently at that (the A4(e) conformance failure). Env changed ⇒ both go
/// inert (`0.0`, gate measure-only per the N14 rule) with a LOUD line naming
/// the recalibration step.
fn update_baseline_write(store: &Path, p95: f64) -> std::io::Result<String> {
    let prior = Baseline::load(&baseline_path(store));
    let env = EnvFingerprint::detect();
    let env_matches = prior.as_ref().is_some_and(|b| b.env.matches(&env));
    let (budget, slack) = match &prior {
        Some(b) if env_matches => (b.design_budget_ms, b.ceiling_slack_ms),
        // env changed or no prior: inert until --calibrate re-derives (A4a/d/e)
        _ => (0.0, 0.0),
    };
    let baseline = Baseline {
        p95_ms: p95,
        design_budget_ms: budget,
        ceiling_slack_ms: slack,
        env,
    };
    baseline.write(&baseline_path(store))?;
    let budget_note = if budget > 0.0 {
        format!("design budget {budget:.2} ms (from a prior calibration) preserved")
    } else if prior.is_some() && !env_matches {
        "environment fingerprint CHANGED since the prior calibration — its budget/slack \
         were measured under a different environment and are left INERT (measure-only); \
         run `--calibrate` under THIS environment (A4d/e)"
            .to_string()
    } else {
        "design budget left INERT — run `--calibrate` to derive it from synthetic-1000 (A4a)"
            .to_string()
    };
    Ok(format!(
        "bench: regression baseline updated — p95 {p95:.2} ms committed with the current \
         environment fingerprint; {budget_note} (reviewable diff at {}).",
        baseline_path(store).display()
    ))
}

/// Measure recall p95 (ms) by timing the WP-3 recall path over `ops` for `samples`
/// iterations. Uses a THROWAWAY telemetry (a temp mark dir + temp file) so a bench
/// never pollutes the real dedup marks or store telemetry.
pub fn measure_p95(store: &Path, ops: &[NormalizedOp], samples: usize, config: &Config) -> f64 {
    if ops.is_empty() || samples == 0 {
        return 0.0;
    }
    let tel = throwaway_telemetry(config);
    let mut durs = Vec::with_capacity(samples);
    for i in 0..samples {
        let op = &ops[i % ops.len()];
        let t0 = Instant::now();
        let _ = recall(op, store, &tel);
        durs.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    percentile_95(&durs)
}

/// Derive a set of recall ops from the store's flat index — the byCommand patterns
/// (which the ops will actually hit) — so the timed path exercises a real walk.
/// Falls back to a single benign `ls` op (still loads + walks the index) when the
/// store has no command routes.
fn ops_for_store(store: &Path) -> Vec<NormalizedOp> {
    let mut ops = Vec::new();
    if let Ok(text) = fs::read_to_string(index_path(store)) {
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Ok(rec) = IndexRecord::parse(line)
                && rec.axis == Axis::Command
            {
                ops.push(bash_op(&rec.pattern));
                if ops.len() >= 64 {
                    break;
                }
            }
        }
    }
    if ops.is_empty() {
        ops.push(bash_op("ls"));
    }
    ops
}

/// A `PreOp` Bash op whose command text is `token` (so recall extracts it as a
/// command basename and walks byCommand).
fn bash_op(token: &str) -> NormalizedOp {
    NormalizedOp::PreOp(ToolOp {
        tool_name: "Bash".to_string(),
        command_text: Some(token.to_string()),
        ..Default::default()
    })
}

/// A throwaway telemetry pointed at a unique temp mark dir + temp file, so bench
/// timing writes nothing to the real dedup dir or store telemetry.
fn throwaway_telemetry(config: &Config) -> Telemetry {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("rejolt-bench-{}-{n}", std::process::id()));
    Telemetry::new(base.join("rt"), base.join("tel.jsonl"), config.clone())
}

/// Measure the store telemetry byte count + day-span (for the R7 rate). Reads the
/// live file + its `.1` rotation. `None` when there is no telemetry or the span is
/// unmeasurable.
fn measure_tel_rate(store: &Path) -> Option<(u64, f64)> {
    let tel = telemetry_path(store);
    let mut rot = tel.clone().into_os_string();
    rot.push(".1");
    let files = [tel, PathBuf::from(rot)];

    let mut total_bytes: u64 = 0;
    let mut min_ts = i64::MAX;
    let mut max_ts = i64::MIN;
    for f in &files {
        let Ok(text) = fs::read_to_string(f) else {
            continue;
        };
        total_bytes += text.len() as u64;
        for line in text.lines() {
            if let Some(ts) = extract_ts(line) {
                min_ts = min_ts.min(ts);
                max_ts = max_ts.max(ts);
            }
        }
    }
    if total_bytes == 0 || min_ts == i64::MAX || max_ts <= min_ts {
        return None;
    }
    let span_days = (max_ts - min_ts) as f64 / 86_400.0;
    (span_days > 0.0).then_some((total_bytes, span_days))
}

/// Pull the `ts` integer field out of a telemetry JSONL line (any record kind).
fn extract_ts(line: &str) -> Option<i64> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    v.get("ts").and_then(serde_json::Value::as_i64)
}

// =============================================================================
// Synthetic reference corpus (the design-budget input)
// =============================================================================

/// A synthetic store built in a temp dir (with a cleanup drop). Each memory carries
/// one distinct per-memory command trigger, so recall routes to it.
struct SyntheticStore {
    dir: PathBuf,
    ops: Vec<NormalizedOp>,
}

impl SyntheticStore {
    /// Build an `n`-memory synthetic store (empty grammar seed + per-memory command
    /// triggers), rebuild it, and derive one op per (a capped subset of) memories.
    fn build(n: usize, config: &Config) -> std::io::Result<SyntheticStore> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rejolt-synth-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let grammar = dir.join("_grammar.toml");
        fs::write(&grammar, "grammar-version = 1\n")?;
        let mut ops = Vec::new();
        for i in 0..n {
            let cmd = format!("synthcmd{i}");
            let body = format!(
                "---\nname: synth-{i}\ndescription: synthetic bench memory {i}\nmetadata:\n  \
                 tags: [synth-bench]\n  triggers:\n    commands: [{cmd}]\n---\nbody {i}\n"
            );
            fs::write(dir.join(format!("synth-{i}.md")), body)?;
            if ops.len() < 64 {
                ops.push(bash_op(&cmd));
            }
        }
        let build_cfg = BuildConfig {
            max_description_chars: config.max_description_chars,
        };
        // A rebuild failure here is a real I/O/grammar fault — surface it.
        rebuild(&dir, &grammar, &build_cfg).map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(SyntheticStore { dir, ops })
    }
}

impl Drop for SyntheticStore {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_baseline_carries_calibration_only_within_the_same_env() {
        // Walk-back fix F5 (A4d/e): a prior calibration's budget/slack survive
        // `--update-baseline` ONLY when the prior env fingerprint matches the
        // current gate key; across an env change they go inert (0.0) — carrying
        // them forward would stamp the current fingerprint onto magnitudes
        // measured under a different environment, disarming the A4(d) mismatch
        // detector silently.
        let store = std::env::temp_dir().join(format!(
            "rejolt-bench-f5-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&store).unwrap();

        // BAD: prior calibrated under a DIFFERENT env → budget/slack inert + loud.
        let other_env = EnvFingerprint {
            cpu_model: "some-other-cpu".into(),
            governor: "performance".into(),
            power_source: "battery".into(),
            kernel: "k".into(),
        };
        Baseline {
            p95_ms: 1.0,
            design_budget_ms: 12.0,
            ceiling_slack_ms: 0.5,
            env: other_env,
        }
        .write(&baseline_path(&store))
        .unwrap();
        let loud = update_baseline_write(&store, 2.0).unwrap();
        let updated = Baseline::load(&baseline_path(&store)).unwrap();
        assert_eq!(
            updated.design_budget_ms, 0.0,
            "cross-env budget must not carry"
        );
        assert_eq!(
            updated.ceiling_slack_ms, 0.0,
            "cross-env slack must not carry"
        );
        assert!(
            loud.contains("CHANGED"),
            "the degradation must be named LOUDLY (A4e): {loud}"
        );

        // GOOD: prior calibrated under the SAME env → budget/slack preserved.
        Baseline {
            p95_ms: 1.0,
            design_budget_ms: 12.0,
            ceiling_slack_ms: 0.5,
            env: EnvFingerprint::detect(),
        }
        .write(&baseline_path(&store))
        .unwrap();
        let _ = update_baseline_write(&store, 2.0).unwrap();
        let updated = Baseline::load(&baseline_path(&store)).unwrap();
        assert_eq!(updated.design_budget_ms, 12.0, "same-env budget carries");
        assert_eq!(updated.ceiling_slack_ms, 0.5, "same-env slack carries");

        let _ = std::fs::remove_dir_all(&store);
    }

    #[test]
    fn baseline_load_rejects_non_finite_or_negative_magnitudes() {
        // Walk-back fix F7 (A4e): TOML parses nan/inf; a baseline carrying them
        // must load as None (→ LOUD NOBASELINE), never silently disarm the gate.
        let store = std::env::temp_dir().join(format!("rejolt-bench-f7-{}", std::process::id()));
        std::fs::create_dir_all(&store).unwrap();
        let p = baseline_path(&store);
        let write = |slack: &str| {
            std::fs::write(
                &p,
                format!(
                    "p95_ms = 1.0\ndesign_budget_ms = 10.0\nceiling_slack_ms = {slack}\n\
                     [env]\ncpu_model = \"c\"\ngovernor = \"g\"\npower_source = \"AC\"\nkernel = \"k\"\n"
                ),
            )
            .unwrap();
        };
        for bad in ["nan", "inf", "-1.0"] {
            write(bad);
            assert!(
                Baseline::load(&p).is_none(),
                "ceiling_slack_ms = {bad} must not load as a gate input"
            );
        }
        // GOOD contrast: a sane baseline still loads.
        write("0.5");
        assert!(Baseline::load(&p).is_some(), "a finite baseline loads");
        let _ = std::fs::remove_dir_all(&store);
    }

    #[test]
    fn regression_ceiling_is_baseline_plus_calibrated_slack_only() {
        // D9/A4(c): the ceiling is baseline + the calibrated slack floor ALONE —
        // no static relative or absolute term participates at all.
        assert!((regression_ceiling(10.0, 0.0) - 10.0).abs() < 1e-9);
        assert!((regression_ceiling(100.0, 0.0) - 100.0).abs() < 1e-9);
        assert!((regression_ceiling(50.0, 40.0) - 90.0).abs() < 1e-9);
        assert!((regression_ceiling(10.0, 3.0) - 13.0).abs() < 1e-9);
    }

    #[test]
    fn fix2_calibrated_slack_absorbs_in_jitter_p95() {
        // A noisy box: calibrated jitter (40 ms). An in-jitter run must NOT trip a
        // blocking REGRESSED (the permanent-red drift A4c retires). baseline 50,
        // slack 40 → ceiling 90; measured 75 ≤ 90.
        let env = EnvFingerprint {
            cpu_model: "CPU".into(),
            governor: "g".into(),
            power_source: "AC".into(),
            kernel: "k".into(),
        };
        let baseline = Baseline {
            p95_ms: 50.0,
            design_budget_ms: 200.0, // high → the WARN branch is not what we're testing
            ceiling_slack_ms: 40.0,
            env: env.clone(),
        };
        let (v, _loud) = verdict_of(75.0, Some(&baseline), &env);
        assert_ne!(
            v,
            Verdict::Regressed,
            "an in-jitter p95 must not REGRESS-block"
        );
        assert!(
            75.0 <= regression_ceiling(50.0, 40.0),
            "the calibrated-slack ceiling absorbs it"
        );
    }

    #[test]
    fn lock_sub_ms_calibrated_baseline_catches_a_10x_slowdown() {
        // D9/A4(c) lock: at sub-millisecond scale (D16: 0.7–2.4 ms) a real 10×
        // structural slowdown must REGRESS — the old §9 static `max(25%, 15 ms)`
        // floor would have swallowed this (ceiling = 1.0 + 15.0 = 16.0 ≥ 10.0 →
        // wrongly PASS). With the static term gone, ceiling = 1.0 + 0.3 = 1.3 ms,
        // and 10.0 ms > 1.3 ms → REGRESSED.
        let env = EnvFingerprint {
            cpu_model: "CPU".into(),
            governor: "g".into(),
            power_source: "AC".into(),
            kernel: "k".into(),
        };
        let baseline = Baseline {
            p95_ms: 1.0,
            design_budget_ms: 100.0, // wide enough that WARN cannot mask this
            ceiling_slack_ms: 0.3,   // a real, tiny, calibrated A4(c) slack
            env: env.clone(),
        };
        let (v, _loud) = verdict_of(10.0, Some(&baseline), &env);
        assert_eq!(
            v,
            Verdict::Regressed,
            "a 10x sub-ms slowdown must REGRESS now that the static floor is gone"
        );
    }

    #[test]
    fn lock_uncalibrated_baseline_never_regresses() {
        // An uncalibrated baseline (ceiling_slack_ms == 0.0, e.g. a bare
        // `--update-baseline` with no prior `--calibrate`) has no valid jitter
        // floor to gate on — the REGRESSED check must be inert (measure-only)
        // regardless of how large the measured p95 is.
        let env = EnvFingerprint {
            cpu_model: "CPU".into(),
            governor: "g".into(),
            power_source: "AC".into(),
            kernel: "k".into(),
        };
        let uncalibrated = Baseline {
            p95_ms: 1.0,
            design_budget_ms: 0.0,
            ceiling_slack_ms: 0.0,
            env: env.clone(),
        };
        let (v, _loud) = verdict_of(10_000.0, Some(&uncalibrated), &env);
        assert_ne!(
            v,
            Verdict::Regressed,
            "an uncalibrated baseline must never REGRESS-block — measure-only until --calibrate"
        );
    }

    #[test]
    fn design_budget_is_synth_p95_times_three() {
        assert!((design_budget_ms(12.0) - 36.0).abs() < 1e-9);
    }

    #[test]
    fn ceiling_slack_floor_is_max_of_three_sigma_and_band() {
        // Tight cluster with one outlier → the band dominates (3σ < band).
        let runs = [10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 20.0];
        let band = 10.0; // max - min
        let three_sigma = 3.0 * stddev(&runs); // = 9.0
        assert!(
            three_sigma < band,
            "test setup: 3σ ({three_sigma}) must be < band"
        );
        assert!((ceiling_slack_floor(&runs) - band).abs() < 1e-9);

        // Spread cluster → 3σ dominates.
        let runs2 = [5.0, 25.0, 5.0, 25.0, 5.0, 25.0];
        let band2 = 20.0;
        let sigma2 = 3.0 * stddev(&runs2);
        assert!(sigma2 > band2, "test setup: 3σ ({sigma2}) must be > band");
        assert!((ceiling_slack_floor(&runs2) - sigma2).abs() < 1e-9);
    }

    #[test]
    fn percentile_95_nearest_rank() {
        let xs: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        assert_eq!(percentile_95(&xs), 95.0);
        assert_eq!(percentile_95(&[42.0]), 42.0);
        assert_eq!(percentile_95(&[]), 0.0);
    }

    #[test]
    fn kernel_is_never_in_the_gate_key() {
        // RB10: two fingerprints identical but for the kernel MATCH (kernel excluded).
        let a = EnvFingerprint {
            cpu_model: "CPU X".into(),
            governor: "performance".into(),
            power_source: "AC".into(),
            kernel: "7.2.0-rc1".into(),
        };
        let b = EnvFingerprint {
            kernel: "7.3.0-rc9".into(),
            ..a.clone()
        };
        assert!(
            a.matches(&b),
            "a kernel bump must not change the gate key (RB10)"
        );
        assert_eq!(a.gate_key(), b.gate_key());

        // A CPU change DOES change the gate key.
        let c = EnvFingerprint {
            cpu_model: "CPU Y".into(),
            ..a.clone()
        };
        assert!(!a.matches(&c));
    }

    #[test]
    fn nobaseline_when_absent() {
        let env = EnvFingerprint::detect();
        let (v, loud) = verdict_of(3.0, None, &env);
        assert_eq!(v, Verdict::NoBaseline);
        assert!(
            loud.iter().any(|l| l.contains("NOBASELINE")),
            "loud: {loud:?}"
        );
    }

    #[test]
    fn env_mismatch_is_measure_only_and_loud() {
        let cur = EnvFingerprint {
            cpu_model: "CPU CURRENT".into(),
            governor: "performance".into(),
            power_source: "AC".into(),
            kernel: "7.2".into(),
        };
        let baseline = Baseline {
            p95_ms: 5.0,
            design_budget_ms: 15.0,
            ceiling_slack_ms: 2.0,
            env: EnvFingerprint {
                cpu_model: "CPU OTHER".into(),
                ..cur.clone()
            },
        };
        let (v, loud) = verdict_of(9999.0, Some(&baseline), &cur);
        // Even a wildly-slow p95 is MEASURE-ONLY (exit 0) under a fingerprint mismatch.
        assert_eq!(v, Verdict::NoBaseline);
        assert!(
            loud.iter()
                .any(|l| l.contains("ENVIRONMENT FINGERPRINT MISMATCH")),
            "the degrade must be LOUD (A4): {loud:?}"
        );
    }

    #[test]
    fn verdicts_pass_warn_regressed_under_matching_env() {
        let env = EnvFingerprint {
            cpu_model: "CPU".into(),
            governor: "performance".into(),
            power_source: "AC".into(),
            kernel: "k".into(),
        };
        let baseline = Baseline {
            p95_ms: 10.0,
            design_budget_ms: 12.0,
            ceiling_slack_ms: 15.0,
            env: env.clone(),
        };
        // ceiling = 10 + calibrated slack 15 = 25 (D9/A4(c): no static term).
        assert_eq!(verdict_of(11.0, Some(&baseline), &env).0, Verdict::Pass); // ≤ budget
        assert_eq!(verdict_of(20.0, Some(&baseline), &env).0, Verdict::Warn); // > budget, ≤ ceiling
        assert_eq!(
            verdict_of(30.0, Some(&baseline), &env).0,
            Verdict::Regressed
        ); // > ceiling
    }

    #[test]
    fn fix5_inert_design_budget_never_warns() {
        // An update-baseline-with-no-prior baseline has an INERT (0.0) design budget:
        // the WARN-over-budget branch must never trip (only the regression ceiling
        // gates). A real p95 far above the 0.0 budget but under the (calibrated)
        // ceiling → PASS. The slack here is a real calibrated slack (not 0.0) so the
        // ceiling itself stays an active gate — see `lock_uncalibrated_baseline_
        // never_regresses` for the fully-uncalibrated (slack == 0.0) case.
        let env = EnvFingerprint {
            cpu_model: "CPU".into(),
            governor: "g".into(),
            power_source: "AC".into(),
            kernel: "k".into(),
        };
        let inert_budget = Baseline {
            p95_ms: 10.0,
            design_budget_ms: 0.0, // inert (A4a: only --calibrate sets a real budget)
            ceiling_slack_ms: 5.0, // a real calibrated slack → ceiling = 15
            env: env.clone(),
        };
        // 12 > 0.0 (the inert budget) but ≤ ceiling (15) → PASS, not WARN.
        assert_eq!(verdict_of(12.0, Some(&inert_budget), &env).0, Verdict::Pass);
        // The calibrated ceiling still gates a genuine regression.
        assert_eq!(
            verdict_of(20.0, Some(&inert_budget), &env).0,
            Verdict::Regressed
        );
    }

    #[test]
    fn recommended_tel_max_only_when_thirty_days_overflows() {
        // 1 MiB over 1 day, 30-day window, current _TEL_MAX 1 MiB: 30 days needs
        // ~15 MiB of headroom → recommend a resize.
        let one_mib = 1_048_576;
        let rec = recommended_tel_max(one_mib, 1.0, 30, one_mib);
        assert!(rec.is_some(), "30 days does not fit → resize");
        assert!(rec.unwrap() > one_mib);
        // A trickle already fits → no resize.
        assert_eq!(recommended_tel_max(1000, 30.0, 30, one_mib), None);
        // Unmeasurable rate → None.
        assert_eq!(recommended_tel_max(0, 0.0, 30, one_mib), None);
    }
}
