//! The engine config surface — the full §10 tunable table, loaded from
//! `config.toml` (plan P15, R7; D25, §10).
//!
//! D25's core lesson is a **single-source** one: synapse *declared*
//! `dedupeTtlSeconds = 900` in its config yet *hardcoded* `-mmin -15` in the
//! recall hook — a latent divergence where the config knob and the code that
//! honored it drifted apart (extraction finding). This struct is the ONE place
//! the tunable magnitudes live; [`crate::telemetry`] reads its TTL / rotation /
//! window from here, and (WP-7) recall reads its tier weights + confidence
//! thresholds from here too — nothing hardcodes a second copy.
//!
//! ## Unknown keys WARN, never fail (R7) — the OPPOSITE of the grammar
//!
//! The grammar loader ([`crate::grammar`]) uses `deny_unknown_fields`: an unknown
//! grammar key is a hard exit-2 error, because the grammar is a taxonomy whose
//! closed shape is load-bearing. Config is the opposite: an unknown config key is
//! **advisory** ([`load`] collects it as a warning and proceeds on the frozen
//! defaults), and on the hook path ([`load_for_hook`]) it is silently ignored —
//! a malformed config must never fail a fail-open host operation. Absent keys fall
//! back to the frozen §10 defaults via `#[serde(default)]` + [`Default`].
//!
//! ## Which values are config vs const (§10)
//!
//! Per §10 the *form* of the surface gate and the score penalties is a §2
//! invariant (hardcoded), while `tierWeights`, the confidence thresholds,
//! `collisionGuideFloor`, the window, and the promote/demote thresholds are
//! config. `_TEL_MAX` is a const **but** R7 makes it resizable at calibration, so
//! it is carried here as a settable field, not a hard `const`. This struct carries
//! the whole §10 table so the surface is legible and single-sourced; each consumer
//! reads the field it needs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The `[tierWeights]` sub-table (§10): strong / medium / weak recall tier weights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TierWeights {
    /// Strong-tier weight (command / path). §10 default 10.
    pub strong: i64,
    /// Medium-tier weight (arg). §10 default 6.
    pub medium: i64,
    /// Weak-tier weight (synonym). §10 default 3.
    pub weak: i64,
}

impl Default for TierWeights {
    fn default() -> Self {
        TierWeights {
            strong: 10,
            medium: 6,
            weak: 3,
        }
    }
}

/// The `[storeRoots]` sub-table (§3, §5.x): the configured memory-store roots the
/// placement classifier compares a target against. The CLI maps this into a
/// [`crate::guard::StoreRoots`] for the write guard.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoreRootsConfig {
    /// The box-brain store root. Absent → placement enforcement fails open (a
    /// target cannot be classified box vs non-box, so no misplacement deny fires).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub box_root: Option<PathBuf>,
}

/// The full §10 tunable surface, single-sourced (D25). Frozen defaults live in the
/// [`Default`] impl; [`load`] layers `config.toml` over them, warning (never
/// failing) on unknown keys (R7). The three marks/telemetry tunables
/// (`dedupe_ttl_seconds`, `tel_max_bytes`, `telemetry_window_days`) keep their
/// WP-2b names/types so [`crate::telemetry`] reads them unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// `[tierWeights]` — recall tier weights (§10; lifted from the WP-3 consts, R7).
    pub tier_weights: TierWeights,
    /// `confidenceHighThreshold` (§10, default 10): score ≥ this ⇒ "high".
    pub confidence_high_threshold: i64,
    /// `confidenceMediumThreshold` (§10, default 6): score ≥ this ⇒ "medium".
    pub confidence_medium_threshold: i64,
    /// `collisionGuideFloor` (§10, default 8): the single corpus-breadth cutoff.
    /// (Consumed by [`crate::projection`]; carried here as the config surface.)
    pub collision_guide_floor: usize,
    /// `promoteThreshold` (§10, default 0.4) — curation seat promotion (WP-6).
    pub promote_threshold: f64,
    /// `demoteThreshold` (§10, default 0.05) — curation seat demotion (WP-6).
    pub demote_threshold: f64,
    /// `telemetryWindowDays` (§10, default 30). Read by [`crate::telemetry`].
    pub telemetry_window_days: u64,
    /// `minEvidenceSessions` (§10, default 10) — curation min-evidence (WP-6).
    pub min_evidence_sessions: u64,
    /// `minEvidenceDays` (§10, default 30) — curation min-evidence (WP-6).
    pub min_evidence_days: u64,
    /// `seatPromoteMinFires` (§10, default 5) — seat dual-gate (WP-6).
    pub seat_promote_min_fires: u64,
    /// `dedupeTtlSeconds` (§10, default 900). Read by [`crate::telemetry`].
    pub dedupe_ttl_seconds: u64,
    /// `_TEL_MAX` (§10, default 1 MiB) — telemetry rotation bound. Resizable at
    /// calibration (R7). Read by [`crate::telemetry`].
    #[serde(rename = "_TEL_MAX")]
    pub tel_max_bytes: u64,
    /// `maxResults` (§10 secondary, default 3) — recall result cap.
    pub max_results: usize,
    /// `maxDescriptionChars` (§10 secondary, default 220) — snippet truncation.
    /// The CLI maps this into [`crate::rebuild::BuildConfig`].
    pub max_description_chars: usize,
    /// `WRITE_CONTEXT_BUDGET` (§10, default 9500). Consumed by [`crate::guard`].
    #[serde(rename = "WRITE_CONTEXT_BUDGET")]
    pub write_context_budget: usize,
    /// `DEDUP_BACKSTOP_THRESHOLD` (§10, default 0.85). Consumed by [`crate::guard`].
    /// FIX 3: the accepted key preserves the §10 SCREAMING_SNAKE spelling (like its
    /// `_TEL_MAX` / `WRITE_CONTEXT_BUDGET` / `BUDGET_MS` siblings), not camelCase.
    #[serde(rename = "DEDUP_BACKSTOP_THRESHOLD")]
    pub dedup_backstop_threshold: f64,
    /// bench `BUDGET_MS` (§10, default 55) — the §10-documented static live-advisory
    /// design budget. FIX 4: recorded here for §10 completeness ONLY; it is
    /// **SUPERSEDED by the A4 calibration design budget** (synthetic-1000 p95 × 3.0,
    /// stored per-baseline as `design_budget_ms`) and is **NOT consumed by
    /// [`crate::bench`]** — the calibrated budget is the sole WARN authority (A4).
    #[serde(rename = "BUDGET_MS")]
    pub bench_budget_ms: u64,
    /// `[storeRoots]` — placement roots for the write guard.
    pub store_roots: StoreRootsConfig,
    /// `grammarPath` — an explicit grammar file override. Absent → the CLI uses the
    /// store-side `_grammar.toml`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grammar_path: Option<PathBuf>,
}

impl Default for Config {
    /// The FROZEN §10 defaults.
    fn default() -> Self {
        Config {
            tier_weights: TierWeights::default(),
            confidence_high_threshold: 10,
            confidence_medium_threshold: 6,
            collision_guide_floor: 8,
            promote_threshold: 0.4,
            demote_threshold: 0.05,
            telemetry_window_days: 30,
            min_evidence_sessions: 10,
            min_evidence_days: 30,
            seat_promote_min_fires: 5,
            dedupe_ttl_seconds: 900,
            tel_max_bytes: 1_048_576,
            max_results: 3,
            max_description_chars: 220,
            write_context_budget: 9500,
            dedup_backstop_threshold: 0.85,
            bench_budget_ms: 55,
            store_roots: StoreRootsConfig::default(),
            grammar_path: None,
        }
    }
}

// =============================================================================
// Loading (direct-CLI: warn on unknown keys; hook: fail-open silent)
// =============================================================================

/// The conventional per-store config path: `<store>/config.toml`.
pub fn config_path(store_dir: &Path) -> PathBuf {
    store_dir.join("config.toml")
}

/// A loaded config plus the advisory warnings a direct CLI should surface (loud,
/// D12). `warnings` holds one line per unknown config key (R7): advisory, not fatal.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    /// The parsed config (frozen defaults for every absent key).
    pub config: Config,
    /// Unknown-key advisories (never fatal; the CLI prints these to stderr).
    pub warnings: Vec<String>,
}

/// A fatal config load error — the config/taxonomy (exit-2) class for a DIRECT CLI
/// only. A malformed config on the hook path is never fatal (see [`load_for_hook`]).
#[derive(Debug)]
pub enum ConfigError {
    /// The config file could not be read (an existing-but-unreadable path).
    Read(std::io::Error),
    /// The config text was not valid TOML, or a KNOWN key had the wrong type. (An
    /// unknown key is NOT this — it warns.) The string is the toml/serde message.
    Parse(String),
}

impl ConfigError {
    /// Every config load error is config/taxonomy → exit 2 (A5/D20 taxonomy).
    pub fn exit_code(&self) -> i32 {
        2
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Read(e) => write!(f, "config read error: {e}"),
            ConfigError::Parse(msg) => write!(f, "config parse error: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Load `config.toml` for a DIRECT CLI (loud). An absent file yields the frozen
/// defaults with no warnings; a malformed TOML / wrong-typed KNOWN key is a
/// [`ConfigError`] (exit-2 config/taxonomy); an unknown key is collected as a
/// warning and the load still succeeds on the defaults (R7).
pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
    if !path.exists() {
        return Ok(LoadedConfig {
            config: Config::default(),
            warnings: Vec::new(),
        });
    }
    let text = std::fs::read_to_string(path).map_err(ConfigError::Read)?;
    // Typed parse first: a malformed TOML or a wrong-typed KNOWN key errors here
    // (exit-2). Unknown keys are ignored by serde (no deny_unknown_fields) and are
    // surfaced as warnings below.
    let config: Config = toml::from_str(&text).map_err(|e| ConfigError::Parse(e.to_string()))?;
    let warnings = unknown_key_warnings(&text, &config);
    Ok(LoadedConfig { config, warnings })
}

/// Load `config.toml` for the HOOK path (fail-open, silent): a missing, malformed,
/// or unknown-key-bearing config all resolve to a usable [`Config`] with NO
/// warning and NO error — a broken config must never fail a fail-open host
/// operation, and the hook path is never loud. WP-5's hook dispatch calls this.
pub fn load_for_hook(path: &Path) -> Config {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Config::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

/// Collect one advisory line per unknown key in `text` (R7). Sound against nested
/// tables: the known key tree is the parsed [`Config`] re-serialized to a
/// [`toml::Value`], and any key present in the raw text but absent from that tree
/// (recursively) is unknown. Because absent-option fields are skipped on
/// serialize, a *present* known optional key is still in the tree, so it is never
/// mis-flagged.
fn unknown_key_warnings(text: &str, config: &Config) -> Vec<String> {
    let Ok(raw) = toml::from_str::<toml::Value>(text) else {
        return Vec::new(); // a parse failure is handled by the typed parse, not here
    };
    let Ok(known) = toml::Value::try_from(config) else {
        return Vec::new();
    };
    let mut unknown = Vec::new();
    collect_unknown_keys(&raw, &known, "", &mut unknown);
    unknown
        .into_iter()
        .map(|k| {
            format!("config: unknown key `{k}` ignored (advisory; using the frozen §10 default)")
        })
        .collect()
}

/// Recurse the raw vs known table trees, pushing the dotted path of every key that
/// exists in `raw` but not in `known`. Recurses only where BOTH sides are tables.
fn collect_unknown_keys(
    raw: &toml::Value,
    known: &toml::Value,
    prefix: &str,
    out: &mut Vec<String>,
) {
    let (toml::Value::Table(rt), toml::Value::Table(kt)) = (raw, known) else {
        return;
    };
    for (k, v) in rt {
        let path = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match kt.get(k) {
            None => out.push(path),
            Some(kv) => collect_unknown_keys(v, kv, &path, out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_defaults() {
        let c = Config::default();
        assert_eq!(c.dedupe_ttl_seconds, 900, "§10 dedupeTtlSeconds");
        assert_eq!(c.tel_max_bytes, 1_048_576, "§10 _TEL_MAX = 1 MiB");
        assert_eq!(c.telemetry_window_days, 30, "§10 telemetryWindowDays");
        assert_eq!(c.tier_weights, TierWeights::default());
        assert_eq!(c.confidence_high_threshold, 10);
        assert_eq!(c.confidence_medium_threshold, 6);
        assert_eq!(c.collision_guide_floor, 8);
        assert_eq!(c.max_results, 3);
        assert_eq!(c.max_description_chars, 220);
        assert_eq!(c.write_context_budget, 9500);
        assert_eq!(c.bench_budget_ms, 55);
        assert!((c.dedup_backstop_threshold - 0.85).abs() < 1e-9);
    }

    #[test]
    fn config_surface_matches_the_still_const_consumers() {
        // Several §10 values are carried on the config SURFACE here for legibility +
        // later-WP consumption, while their live consumers still read a module const
        // (collisionGuideFloor → projection; WRITE_CONTEXT_BUDGET + DEDUP_BACKSTOP →
        // guard; tier weights / confidence thresholds are already read from config by
        // recall). This pins them equal so the surface can NEVER silently diverge from
        // the code that consumes it — if a future edit moves one, this trips.
        let c = Config::default();
        assert_eq!(
            c.collision_guide_floor,
            crate::projection::COLLISION_GUIDE_FLOOR,
            "config surface must match projection's const"
        );
        assert_eq!(
            c.write_context_budget,
            crate::guard::WRITE_CONTEXT_BUDGET,
            "config surface must match guard's WRITE_CONTEXT_BUDGET"
        );
        assert!(
            (c.dedup_backstop_threshold - crate::guard::DEDUP_BACKSTOP_THRESHOLD).abs() < 1e-9,
            "config surface must match guard's DEDUP_BACKSTOP_THRESHOLD"
        );
    }

    #[test]
    fn tunables_are_settable_for_calibration_and_tests() {
        // R7: _TEL_MAX is resizable at calibration; TTL/window are config knobs.
        let c = Config {
            dedupe_ttl_seconds: 5,
            tel_max_bytes: 128,
            telemetry_window_days: 7,
            ..Config::default()
        };
        assert_ne!(c, Config::default());
    }

    #[test]
    fn absent_file_yields_defaults_no_warnings() {
        let missing = std::env::temp_dir().join("rejolt-cfg-does-not-exist-xyz.toml");
        let loaded = load(&missing).expect("absent file is not an error");
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn partial_config_fills_absent_keys_from_defaults() {
        // Only one key set; every other §10 value must be the frozen default.
        let text = "maxResults = 7\n";
        let c: Config = toml::from_str(text).unwrap();
        assert_eq!(c.max_results, 7);
        assert_eq!(c.max_description_chars, 220, "absent key → frozen default");
        assert_eq!(
            c.tier_weights,
            TierWeights::default(),
            "absent sub-table → default"
        );
    }

    #[test]
    fn unknown_keys_warn_but_never_fail() {
        let text = "maxResults = 7\nbogusKey = 1\n[tierWeights]\nstrong = 11\nmystery = 2\n";
        let config: Config = toml::from_str(text).unwrap();
        let warnings = unknown_key_warnings(text, &config);
        assert_eq!(config.max_results, 7);
        assert_eq!(config.tier_weights.strong, 11);
        assert!(
            warnings.iter().any(|w| w.contains("bogusKey")),
            "top-level unknown: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("tierWeights.mystery")),
            "nested unknown: {warnings:?}"
        );
        // A known + present optional key must NOT be mis-flagged as unknown.
        let text2 = "grammarPath = \"/x/g.toml\"\n";
        let cfg2: Config = toml::from_str(text2).unwrap();
        assert!(
            unknown_key_warnings(text2, &cfg2).is_empty(),
            "present optional key mis-flagged"
        );
    }

    #[test]
    fn dedup_backstop_uses_screaming_snake_key() {
        // FIX 3: the §10 spelling is the accepted key (no warning); the camelCase
        // form is unknown (warned + ignored → the field keeps its default).
        let text = "DEDUP_BACKSTOP_THRESHOLD = 0.5\n";
        let c: Config = toml::from_str(text).unwrap();
        assert!(
            (c.dedup_backstop_threshold - 0.5).abs() < 1e-9,
            "§10 spelling applied"
        );
        assert!(
            unknown_key_warnings(text, &c).is_empty(),
            "§10 spelling is NOT unknown"
        );

        let camel = "dedupBackstopThreshold = 0.5\n";
        let c2: Config = toml::from_str(camel).unwrap();
        assert!(
            (c2.dedup_backstop_threshold - 0.85).abs() < 1e-9,
            "the camelCase form is not the accepted key → default kept"
        );
        assert!(
            unknown_key_warnings(camel, &c2)
                .iter()
                .any(|w| w.contains("dedupBackstopThreshold")),
            "the camelCase form warns as unknown"
        );
    }

    #[test]
    fn malformed_config_on_hook_path_never_fatal() {
        // A hook-path load of broken TOML must not panic/err — it falls back to
        // the frozen defaults (fail-open, silent). We drive load_for_hook against a
        // real file so the whole read+parse path is exercised.
        let dir = std::env::temp_dir().join(format!("rejolt-cfg-hook-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, "this is = = not valid toml {{{").unwrap();
        assert_eq!(load_for_hook(&p), Config::default(), "malformed → defaults");
        // And a hook load of a wrong-typed KNOWN key is also non-fatal.
        std::fs::write(&p, "maxResults = \"not-an-int\"\n").unwrap();
        assert_eq!(load_for_hook(&p), Config::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_typed_known_key_is_fatal_for_direct_cli() {
        let dir = std::env::temp_dir().join(format!("rejolt-cfg-direct-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, "maxResults = \"not-an-int\"\n").unwrap();
        let err = load(&p).expect_err("wrong-typed known key is a config/taxonomy error");
        assert_eq!(err.exit_code(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
