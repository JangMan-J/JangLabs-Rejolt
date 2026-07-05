//! Config surface conformance (WP-7 / P15, R7; §10). The config loader is the
//! OPPOSITE of the grammar's `deny_unknown_fields`: unknown keys WARN (advisory,
//! never fatal), absent keys fall back to the frozen §10 defaults, and a malformed
//! config on the hook path is never fatal.

use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::config::{self, Config};
use rejolt::conformance::{Check, assert_counts, fixtures_root};

fn unique_dir(tag: &str) -> std::path::PathBuf {
    static C: AtomicU32 = AtomicU32::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("rejolt-cfg-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// G2: the config loader ACCEPTS valid configs (including ones with unknown keys,
/// which still load Ok) and REJECTS malformed / wrong-typed-known-key configs.
#[test]
fn config_load_g2_known_good_and_bad() {
    let check = Check::new("config-load", "config", |p| config::load(p).is_ok());
    assert_counts(&check, &fixtures_root());
}

/// R7: unknown keys are surfaced as advisory warnings but never fail the load, and
/// the known keys around them are still applied.
#[test]
fn unknown_keys_warn_but_load_succeeds() {
    let p = fixtures_root().join("config/good/unknown-keys-still-parse.toml");
    let loaded = config::load(&p).expect("unknown keys are never fatal (R7)");
    assert!(
        !loaded.warnings.is_empty(),
        "an unknown key must produce a warning"
    );
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| w.contains("bogusTopLevelKey")),
        "top-level unknown key not warned: {:?}",
        loaded.warnings
    );
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| w.contains("tierWeights.mysteryNested")),
        "nested unknown key not warned: {:?}",
        loaded.warnings
    );
    // The known keys around the unknowns are still applied.
    assert_eq!(loaded.config.max_results, 5);
    assert_eq!(loaded.config.tier_weights.strong, 11);
}

/// §10 defaults fill every absent key/file.
#[test]
fn frozen_defaults_when_key_or_file_absent() {
    // Absent FILE → all defaults, no warnings.
    let missing = unique_dir("absent").join("nope.toml");
    let loaded = config::load(&missing).expect("absent file is not an error");
    assert_eq!(loaded.config, Config::default());
    assert!(loaded.warnings.is_empty());

    // Absent KEY → that key's frozen default; only-defaults fixture sets one key.
    let p = fixtures_root().join("config/good/only-defaults.toml");
    let loaded = config::load(&p).expect("valid");
    assert_eq!(loaded.config.max_results, 3);
    assert_eq!(
        loaded.config.max_description_chars, 220,
        "absent key → §10 default"
    );
    assert_eq!(loaded.config.collision_guide_floor, 8);
    assert_eq!(loaded.config.dedupe_ttl_seconds, 900);
}

/// A malformed config on the HOOK path is never fatal (fail-open, silent → defaults).
#[test]
fn malformed_config_on_hook_path_never_fatal() {
    let malformed = fixtures_root().join("config/bad/malformed.toml");
    assert_eq!(
        config::load_for_hook(&malformed),
        Config::default(),
        "hook-path load of a malformed config must fall back to defaults, never fail"
    );
    let wrong_typed = fixtures_root().join("config/bad/wrong-typed-known-key.toml");
    assert_eq!(config::load_for_hook(&wrong_typed), Config::default());

    // The SAME wrong-typed key IS fatal for a direct CLI (exit-2 config/taxonomy).
    let err = config::load(&wrong_typed).expect_err("direct-CLI load rejects a wrong-typed key");
    assert_eq!(err.exit_code(), 2);
}
