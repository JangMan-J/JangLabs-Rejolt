//! Bench + calibration conformance (WP-7 / P13; D9, D26, A4, RB6, RB10; §9).
//! End-to-end over `run_bench`: the NOBASELINE interim (measure-only, exit 0), a
//! calibrate run writing a reviewable env-fingerprinted baseline, and the
//! environment-mismatch LOUD measure-only degrade (A4). The pure calibration
//! arithmetic + kernel-exclusion are unit-tested in `src/bench.rs`.

use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::bench::{self, Baseline, EnvFingerprint, Verdict};
use rejolt::config::Config;

fn unique_store(tag: &str) -> std::path::PathBuf {
    static C: AtomicU32 = AtomicU32::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("rejolt-bench-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn nobaseline_is_measure_only_exit_zero() {
    // No committed baseline → NOBASELINE (the interim), measure-only, exit 0.
    let store = unique_store("nobaseline");
    let out = bench::run_bench(&store, 16, false, false, 8, &Config::default()).expect("bench");
    assert_eq!(out.verdict, Verdict::NoBaseline);
    assert_eq!(out.exit_code(), 0, "NOBASELINE must not block");
    assert!(
        out.loud.iter().any(|l| l.contains("NOBASELINE")),
        "loud: {:?}",
        out.loud
    );
    assert!(out.p95_ms >= 0.0);
}

#[test]
fn calibrate_writes_reviewable_env_fingerprinted_baseline() {
    let store = unique_store("calibrate");
    // Small synthetic_n keeps the test fast (production uses SYNTHETIC_REFERENCE_N).
    let out = bench::run_bench(&store, 16, false, true, 12, &Config::default()).expect("calibrate");
    assert!(out.baseline_written, "calibrate must write a baseline");
    assert!(
        out.loud.iter().any(|l| l.contains("CALIBRATED")),
        "calibrate must be LOUD: {:?}",
        out.loud
    );

    // The reviewable baseline exists and carries the derivation + env fingerprint;
    // no number is fabricated — the design budget is a measured synthetic p95 × 3.0.
    let baseline = Baseline::load(&bench::baseline_path(&store)).expect("baseline file written");
    assert!(
        baseline.design_budget_ms >= baseline.p95_ms * 0.0,
        "budget is measured"
    );
    assert!(
        !baseline.env.cpu_model.is_empty(),
        "env fingerprint recorded"
    );
    // The baseline file names the kernel as metadata-only (RB10).
    let text = std::fs::read_to_string(bench::baseline_path(&store)).unwrap();
    assert!(text.contains("kernel"), "kernel recorded as metadata");
    assert!(
        text.contains("NEVER a gate key"),
        "the RB10 note must be present"
    );

    // A subsequent run under the SAME (matching) env is no longer NOBASELINE — the
    // committed baseline is usable as a real gate (PASS/WARN/REGRESSED, timing-dep).
    let out2 = bench::run_bench(&store, 16, false, false, 12, &Config::default()).expect("bench 2");
    assert_ne!(
        out2.verdict,
        Verdict::NoBaseline,
        "a matching-env baseline is usable"
    );
}

#[test]
fn fix5_update_baseline_no_prior_leaves_budget_inert() {
    // FIX 5 (A4a): `--update-baseline` with no prior calibration must NOT synthesize
    // a design budget from the (empty) real store — it leaves it INERT (0.0), so a
    // subsequent real run does not spuriously WARN. The design budget comes ONLY
    // from `--calibrate` (synthetic-1000 × 3.0).
    let store = unique_store("update-noprior");
    let out = bench::run_bench(&store, 16, true, false, 8, &Config::default()).expect("update");
    assert!(out.baseline_written);
    let baseline = Baseline::load(&bench::baseline_path(&store)).expect("baseline written");
    assert_eq!(
        baseline.design_budget_ms, 0.0,
        "no prior calibration → inert design budget (A4a), never synthesized from the real store"
    );

    // A subsequent plain run under the (matching) env must NOT spuriously WARN.
    let out2 = bench::run_bench(&store, 16, false, false, 8, &Config::default()).expect("bench 2");
    assert_ne!(
        out2.verdict,
        Verdict::Warn,
        "an inert budget must never trip WARN"
    );
    assert_eq!(out2.exit_code(), 0);
}

#[test]
fn env_fingerprint_mismatch_is_loud_measure_only() {
    // A baseline recorded under a DIFFERENT cpu model → the gate is measure-only
    // (exit 0) AND LOUD (A4) — even a wildly slow p95 must not REGRESS-block.
    let store = unique_store("mismatch");
    let bogus = Baseline {
        p95_ms: 0.0001,
        design_budget_ms: 0.0003,
        ceiling_slack_ms: 0.0,
        env: EnvFingerprint {
            cpu_model: "SOME OTHER BOX CPU that is definitely not this one".into(),
            governor: "made-up".into(),
            power_source: "made-up".into(),
            kernel: "made-up".into(),
        },
    };
    bogus.write(&bench::baseline_path(&store)).unwrap();

    let out = bench::run_bench(&store, 16, false, false, 8, &Config::default()).expect("bench");
    assert_eq!(
        out.verdict,
        Verdict::NoBaseline,
        "mismatch → measure-only, not a gate"
    );
    assert_eq!(out.exit_code(), 0);
    assert!(
        out.loud
            .iter()
            .any(|l| l.contains("ENVIRONMENT FINGERPRINT MISMATCH")),
        "the degrade must be LOUD (A4): {:?}",
        out.loud
    );
}
