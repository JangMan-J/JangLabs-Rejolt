//! The single source of the marks / telemetry tunables (plan P11; D25, R7, §10).
//!
//! D25's core lesson is a **single-source** one: synapse *declared*
//! `dedupeTtlSeconds = 900` in its config yet *hardcoded* `-mmin -15` in the
//! recall hook — a latent divergence where the config knob and the code that
//! honored it drifted apart (extraction finding). This struct is the ONE place
//! the marks/telemetry code reads its tunables from; nothing in
//! [`crate::telemetry`] hardcodes a TTL, a rotation size, or a window length.
//!
//! ## Scope of THIS packet (WP-2b)
//!
//! Only the three marks/telemetry tunables live here, as frozen defaults with a
//! [`Default`] impl. **There is deliberately no serde / TOML loading here.** WP-7
//! (plan P15, R7) EXTENDS this struct with serde `config.toml` deserialization,
//! the rest of §10's tunables (`tierWeights`, `collisionGuideFloor`,
//! `promoteThreshold`, …), and unknown-keys-warn (advisory, never fatal on hook
//! paths). Keeping WP-2b config-loader-free keeps the divergence surface minimal
//! until that packet owns the whole §10 table.
//!
//! ## Which values are config vs const (§10)
//!
//! Per §10, `dedupeTtlSeconds` and `telemetryWindowDays` are config; `_TEL_MAX`
//! is a const *but* R7 makes it **resizable at calibration** (the window is
//! `min(telemetryWindowDays, rotation bound ≈ 2×_TEL_MAX)`, and calibration
//! resizes `_TEL_MAX` if 30 days does not fit). So it is carried here as a
//! settable field, not a hard `const`, so WP-7/P13 can rewrite it from a measured
//! record rate without a code change.

/// The marks / telemetry tunables, single-sourced (D25). Frozen defaults live in
/// the [`Default`] impl; WP-7 will layer `config.toml` deserialization over this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Dedup-mark TTL in seconds (§10 `dedupeTtlSeconds`, default 900 = 15 min).
    /// A dedup mark is LIVE iff its mtime is within this many seconds of now. This
    /// is the ONLY place the TTL is read — [`crate::telemetry`] reads it here and
    /// nowhere else (the D25 single-source fix for the `-mmin -15` divergence).
    pub dedupe_ttl_seconds: u64,
    /// Telemetry rotation threshold in bytes (§10 `_TEL_MAX`, default 1 MiB =
    /// 1_048_576). At/above this size the live telemetry file rotates to one `.1`
    /// generation. **Resizable per R7:** WP-7/P13 calibration rewrites it from the
    /// measured record rate if 30 days of telemetry does not fit the rotation
    /// bound — hence a field, not a hard `const`.
    pub tel_max_bytes: u64,
    /// Telemetry curation window in days (§10 `telemetryWindowDays`, default 30).
    /// The rectangular lookback for rate computation. The EFFECTIVE window the
    /// reader applies is `min(this, rotation bound ≈ 2×tel_max_bytes span)` (R7).
    pub telemetry_window_days: u64,
}

impl Default for Config {
    /// The FROZEN §10 defaults: 900 s TTL, 1 MiB rotation, 30-day window.
    fn default() -> Self {
        Config {
            dedupe_ttl_seconds: 900,
            tel_max_bytes: 1_048_576,
            telemetry_window_days: 30,
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
    }

    #[test]
    fn tunables_are_settable_for_calibration_and_tests() {
        // R7: _TEL_MAX is resizable at calibration; TTL/window are config knobs.
        // A test may point them anywhere — that is what makes liveness/rotation/
        // window behavior injectable without touching the real defaults.
        let c = Config {
            dedupe_ttl_seconds: 5,
            tel_max_bytes: 128,
            telemetry_window_days: 7,
        };
        assert_ne!(c, Config::default());
    }
}
