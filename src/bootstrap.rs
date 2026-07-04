//! Bootstrap — seed a clean, empty store (plan P14; D13, D17, D23, D19, A7,
//! OWNER-R5; CORE-SPEC §13 `[DESIGNED]`).
//!
//! Bootstrap seeds an EMPTY store: the grammar seed is the **version line alone**
//! (`grammar-version = 1`, R5/OWNER); a minimal `MEMORY.md` router; the store-side
//! `_grammar.toml` relative symlink; and one first `rebuild` producing a valid
//! empty catalog with `routabilityReport: 0 unroutable`. It is **idempotent** (a
//! second run with no input change leaves the same observable store state — the
//! catalog is atomically rewritten, never duplicated) and **never overwrites** an
//! existing `MEMORY.md`, grammar, memory, telemetry, or taxonomy file.
//!
//! ## The engine NEVER writes host policy (D13 / N7)
//!
//! There is **no `--import-legacy`** (D17 — void) and bootstrap writes **no host
//! permission policy or host settings**, including the hook wiring: `--print-hooks`
//! (handled by the CLI via [`crate::hooks`]) EMITS the settings block to stdout for
//! the human to place; the engine never writes it (§0/§12/§13).
//!
//! ## Fail-open verification suite (bootstrap-LOCAL rows here; WP-8 owns host rows)
//!
//! Bootstrap runs the A7 mark-dir writability probe (emitting the one inert-
//! telemetry advisory if unwritable) and STRUCTURALLY asserts, against throwaway
//! probe stores, that the engine's fail-open contract holds locally:
//! `.surface-disabled` is detectable, and recall on a missing catalog allows +
//! surfaces nothing WITHOUT rebuilding. The full host-behavior rows are WP-8.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::grammar::GrammarError;
use crate::normalize::{NormalizedOp, ToolOp};
use crate::rebuild::{BuildConfig, RebuildError, RebuildOutcome, index_path, rebuild, report_path};
use crate::telemetry::Telemetry;

/// The store-side grammar file (an infra file: underscore-prefixed → scan skips it).
/// A relative symlink to the lab grammar; the flagless CLIs resolve their grammar
/// here (§13 install-manifest boundary: the ONE managed store file).
pub const STORE_GRAMMAR_FILENAME: &str = "_grammar.toml";
/// The minimal router seed (§13). Never overwrites an existing `MEMORY.md`.
pub const MEMORY_ROUTER_FILENAME: &str = "MEMORY.md";
/// The empty grammar seed literal (R5/OWNER): the version line ALONE.
pub const EMPTY_GRAMMAR_SEED: &str = "grammar-version = 1\n";
/// The minimal `MEMORY.md` router seed (§13): the header alone, no seats.
pub const MEMORY_ROUTER_SEED: &str = "# Memory Router\n";
/// The kill-switch marker basename (P8/§13 fail-open row). WP-5's hook dispatch
/// consults [`is_surface_disabled`]; bootstrap asserts it is structurally wired.
pub const SURFACE_DISABLED_FILENAME: &str = ".surface-disabled";

/// The store-side grammar path (`<store>/_grammar.toml`).
pub fn store_grammar_path(store: &Path) -> PathBuf {
    store.join(STORE_GRAMMAR_FILENAME)
}

/// The `.surface-disabled` kill-switch marker path under a store.
pub fn surface_disabled_marker(store: &Path) -> PathBuf {
    store.join(SURFACE_DISABLED_FILENAME)
}

/// Whether the `.surface-disabled` kill-switch is present (P8 / §13). WP-5's hook
/// dispatch gates on this; here it is the structurally-wired helper bootstrap
/// verifies.
pub fn is_surface_disabled(store: &Path) -> bool {
    surface_disabled_marker(store).exists()
}

// =============================================================================
// Report shapes
// =============================================================================

/// The bootstrap-local fail-open verification results (A7 + §13 structural rows).
/// Every field is advisory (fail-open); a structural contract break is reported
/// loudly but only [`VerificationReport::structural_ok`] gates the exit code.
///
/// The structural rows are **tri-state** (`Option<bool>`): `Some(true)` = the probe
/// RAN and the contract held; `Some(false)` = the probe RAN and observed a BREAK
/// (the only thing that gates exit 1); `None` = the probe could not set up its
/// scaffolding (e.g. a read-only filesystem) and was **SKIPPED** — advisory, never a
/// forced failure (A7/A5 fail-open: a hardened environment must not fail an
/// otherwise-correct, idempotently-seeded bootstrap).
#[derive(Debug, Clone)]
pub struct VerificationReport {
    /// Whether the runtime mark dir is writable (A7). `false` ⇒ telemetry inert.
    pub mark_dir_writable: bool,
    /// The one inert-telemetry advisory (A7), present iff the mark dir is unwritable.
    pub inert_advisory: Option<String>,
    /// Structural: recall on a MISSING catalog allows, surfaces nothing, and does
    /// NOT rebuild the catalog (the §13 fail-open row). `None` = probe skipped.
    pub missing_catalog_failopen: Option<bool>,
    /// Structural: the `.surface-disabled` kill-switch marker is detectable (P8).
    /// `None` = probe skipped.
    pub surface_disabled_wired: Option<bool>,
}

impl VerificationReport {
    /// Whether no structural fail-open contract check OBSERVED a break. A break
    /// (`Some(false)`) is an engine-contract failure and fails the bootstrap (exit
    /// 1); a SKIPPED probe (`None`, scaffolding unavailable) is advisory and never
    /// gates — otherwise a read-only `TMPDIR` would fail a correct bootstrap
    /// (A7/A5). An unwritable mark dir (advisory) never gates either.
    pub fn structural_ok(&self) -> bool {
        self.missing_catalog_failopen != Some(false) && self.surface_disabled_wired != Some(false)
    }
}

/// What bootstrap created + did (the loud creation report the CLI renders).
#[derive(Debug, Clone)]
pub struct BootstrapReport {
    /// Whether the store directory was created (vs already present).
    pub store_created: bool,
    /// Whether the lab grammar file was seeded (`grammar-version = 1`) vs already
    /// present (never overwritten).
    pub grammar_seeded: bool,
    /// Whether the store-side `_grammar.toml` symlink was created vs already present.
    pub grammar_symlinked: bool,
    /// Whether `MEMORY.md` was created vs already present (never overwritten).
    pub memory_created: bool,
    /// The first rebuild's outcome (valid empty catalog; `0 unroutable`).
    pub rebuild: RebuildOutcome,
    /// The fail-open verification results.
    pub verification: VerificationReport,
}

/// A bootstrap failure. A grammar/taxonomy error is exit-2 class; an I/O error is
/// exit-1 (operational). The CLI maps these.
#[derive(Debug)]
pub enum BootstrapError {
    /// A filesystem error seeding a file, symlinking, or rebuilding.
    Io(std::io::Error),
    /// The (user-supplied) grammar failed to validate (exit-2 config/taxonomy).
    Grammar(GrammarError),
}

impl std::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootstrapError::Io(e) => write!(f, "bootstrap I/O error: {e}"),
            BootstrapError::Grammar(e) => write!(f, "bootstrap grammar error: {e}"),
        }
    }
}

impl std::error::Error for BootstrapError {}

impl BootstrapError {
    /// The exit code: a grammar error is config/taxonomy (2); I/O is operational (1).
    pub fn exit_code(&self) -> i32 {
        match self {
            BootstrapError::Grammar(_) => 2,
            BootstrapError::Io(_) => 1,
        }
    }
}

// =============================================================================
// bootstrap
// =============================================================================

/// Bootstrap a store at `store`, using `grammar` as the (lab) grammar file. Seeds
/// the empty grammar (if absent), the store-side symlink, and `MEMORY.md` (if
/// absent), runs the first rebuild, and runs the bootstrap-local fail-open
/// verification suite. Idempotent + never-overwrite (P14). `--print-hooks` is the
/// CLI's concern; the engine writes no host settings (D13).
pub fn bootstrap(
    store: &Path,
    grammar: &Path,
    config: &Config,
) -> Result<BootstrapReport, BootstrapError> {
    // 1. Store dir.
    let store_created = !store.exists();
    std::fs::create_dir_all(store).map_err(BootstrapError::Io)?;

    // 2. Grammar seed (never overwrite): the version line ALONE (R5/OWNER).
    let grammar_seeded = seed_if_absent(grammar, EMPTY_GRAMMAR_SEED).map_err(BootstrapError::Io)?;

    // 3. Store-side relative symlink (never overwrite an existing store grammar).
    let grammar_symlinked =
        seed_store_grammar_symlink(store, grammar).map_err(BootstrapError::Io)?;

    // 4. Minimal MEMORY.md router (never overwrite).
    let memory_created = seed_if_absent(&store.join(MEMORY_ROUTER_FILENAME), MEMORY_ROUTER_SEED)
        .map_err(BootstrapError::Io)?;

    // 5. First rebuild via the store-side grammar (→ valid empty catalog).
    let build_cfg = BuildConfig {
        max_description_chars: config.max_description_chars,
    };
    let rebuild = rebuild(store, &store_grammar_path(store), &build_cfg).map_err(|e| match e {
        RebuildError::Grammar(g) => BootstrapError::Grammar(g),
        RebuildError::Io(io) => BootstrapError::Io(io),
    })?;

    // 6. Fail-open verification suite (bootstrap-local rows).
    let verification = verify_fail_open(store, config);

    Ok(BootstrapReport {
        store_created,
        grammar_seeded,
        grammar_symlinked,
        memory_created,
        rebuild,
        verification,
    })
}

/// Write `contents` to `path` iff it does not already exist. Returns whether it was
/// created. NEVER overwrites (P14): an existing file is left byte-for-byte intact.
fn seed_if_absent(path: &Path, contents: &str) -> std::io::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(true)
}

/// Create `<store>/_grammar.toml` as a RELATIVE symlink to the lab `grammar` file,
/// iff a store grammar does not already exist (symlink OR regular file — never
/// overwrite). Returns whether it was created. A relative target keeps the store
/// portable (§13); if the relative path cannot be computed, an absolute symlink is
/// used as a safe fallback.
fn seed_store_grammar_symlink(store: &Path, grammar: &Path) -> std::io::Result<bool> {
    let link = store_grammar_path(store);
    // `symlink_metadata` does not follow — a dangling/self-symlink still counts as
    // present, so we never clobber a store grammar the user placed.
    if std::fs::symlink_metadata(&link).is_ok() {
        return Ok(false);
    }
    // If the lab grammar IS already the store grammar (same path), nothing to link.
    if paths_equal(&link, grammar) {
        return Ok(false);
    }
    let target = relative_symlink_target(store, grammar);
    std::os::unix::fs::symlink(&target, &link)?;
    Ok(true)
}

/// Compute a relative symlink target from `store` (the symlink's directory) to the
/// `grammar` file, using canonical paths where possible (both exist by this point).
/// Falls back to the grammar's absolute path if a relative path cannot be derived.
fn relative_symlink_target(store: &Path, grammar: &Path) -> PathBuf {
    let (Ok(store_c), Ok(grammar_c)) =
        (std::fs::canonicalize(store), std::fs::canonicalize(grammar))
    else {
        return absolute_lexical(grammar);
    };
    relative_from(&store_c, &grammar_c).unwrap_or(grammar_c)
}

/// The relative path from directory `from` to file `to` (both absolute + normalized).
/// `None` if they share no root (different prefixes) — the caller then uses absolute.
fn relative_from(from: &Path, to: &Path) -> Option<PathBuf> {
    let from_comps: Vec<_> = from.components().collect();
    let to_comps: Vec<_> = to.components().collect();
    // Require a shared root component (both absolute under the same prefix).
    if from_comps.first() != to_comps.first() {
        return None;
    }
    let mut i = 0;
    while i < from_comps.len() && i < to_comps.len() && from_comps[i] == to_comps[i] {
        i += 1;
    }
    let mut result = PathBuf::new();
    for _ in i..from_comps.len() {
        result.push("..");
    }
    for c in &to_comps[i..] {
        result.push(c.as_os_str());
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    Some(result)
}

/// Best-effort absolute path (lexical; does not resolve symlinks).
fn absolute_lexical(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Whether two paths point at the same file (canonical compare; falls back to a
/// lexical-absolute compare when canonicalize fails).
fn paths_equal(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => absolute_lexical(a) == absolute_lexical(b),
    }
}

// =============================================================================
// Fail-open verification suite (bootstrap-local)
// =============================================================================

/// Run the bootstrap-local fail-open checks (A7 + §13 structural rows).
///
/// FIX 1: the structural probes build their scaffolding INSIDE the just-seeded
/// store (which was demonstrably writable — we just wrote `MEMORY.md`, the symlink,
/// and the catalog into it), NOT under `$TMPDIR`. A hardened / read-only `TMPDIR`
/// must not fail an otherwise-correct bootstrap. If the store subtree itself cannot
/// host the scaffolding, the probes are SKIPPED (`None`, advisory), never a forced
/// failure — only a probe that actually RAN and observed a break gates exit 1.
fn verify_fail_open(store: &Path, config: &Config) -> VerificationReport {
    // A7: mark-dir writability + the one inert advisory.
    let tel = Telemetry::for_store(store, config.clone());
    let mark_dir_writable = tel.mark_dir_writable();
    let inert_advisory = tel.inert_telemetry_advisory();

    // Probe scaffolding under the known-writable store; removed before returning so
    // the store's observable state (and idempotence) is untouched.
    let probe_base = store.join(probe_base_name());
    let (missing_catalog_failopen, surface_disabled_wired) =
        match std::fs::create_dir_all(&probe_base) {
            Ok(()) => {
                let a = probe_missing_catalog_failopen(&probe_base, config);
                let b = probe_surface_disabled_wired(&probe_base);
                let _ = std::fs::remove_dir_all(&probe_base);
                (a, b)
            }
            // Even the store subtree cannot host scaffolding → both SKIPPED (advisory).
            Err(_) => (None, None),
        };

    VerificationReport {
        mark_dir_writable,
        inert_advisory,
        missing_catalog_failopen,
        surface_disabled_wired,
    }
}

/// Structural probe (§13 fail-open row): recall on a store with NO catalog must
/// allow (return silence) and must NOT create the catalog (index-only, never
/// rebuild-on-read). Runs against a throwaway empty store UNDER `base`. Returns
/// `None` when the probe scaffolding cannot be created (SKIPPED, advisory);
/// `Some(false)` ONLY when the contract is actually observed broken.
fn probe_missing_catalog_failopen(base: &Path, config: &Config) -> Option<bool> {
    let dir = base.join("missing-catalog");
    std::fs::create_dir_all(&dir).ok()?; // scaffolding unavailable → SKIPPED (None)
    let tel = Telemetry::new(dir.join("rt"), dir.join("tel.jsonl"), config.clone());
    let op = NormalizedOp::PreOp(ToolOp {
        tool_name: "Bash".to_string(),
        command_text: Some("ls".to_string()),
        ..Default::default()
    });
    let out = crate::recall::recall(&op, &dir, &tel);
    // Silence (allow, nothing surfaced) AND no catalog materialized on read.
    let silent = out.is_silent();
    let no_index = !index_path(&dir).exists();
    let no_report = !report_path(&dir).exists();
    Some(silent && no_index && no_report)
}

/// Structural probe (P8): the `.surface-disabled` marker is detectable — absent
/// before, present after touching it. Runs UNDER `base`. `None` = SKIPPED
/// (scaffolding unavailable); `Some(false)` = the marker was undetectable (a break).
fn probe_surface_disabled_wired(base: &Path) -> Option<bool> {
    let dir = base.join("surface-disabled");
    std::fs::create_dir_all(&dir).ok()?; // scaffolding unavailable → SKIPPED (None)
    let before = is_surface_disabled(&dir);
    std::fs::write(surface_disabled_marker(&dir), b"").ok()?; // cannot even write the probe → SKIPPED
    let after = is_surface_disabled(&dir);
    Some(!before && after)
}

/// A per-process-unique probe subdirectory name (hidden, removed before return).
fn probe_base_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!(".rejolt-verify-{}-{n}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d =
            std::env::temp_dir().join(format!("rejolt-boot-t-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn empty_grammar_seed_is_the_version_line_alone() {
        // R5/OWNER: the seed is exactly `grammar-version = 1` — and it validates.
        assert_eq!(EMPTY_GRAMMAR_SEED, "grammar-version = 1\n");
        assert!(crate::grammar::parse_and_validate(EMPTY_GRAMMAR_SEED).is_ok());
    }

    #[test]
    fn seed_if_absent_never_overwrites() {
        let dir = test_dir("seed-noclobber");
        let f = dir.join("MEMORY.md");
        assert!(
            seed_if_absent(&f, "# Memory Router\n").unwrap(),
            "first write creates"
        );
        std::fs::write(&f, "USER EDITED\n").unwrap();
        assert!(
            !seed_if_absent(&f, "# Memory Router\n").unwrap(),
            "second is a no-op"
        );
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "USER EDITED\n",
            "never overwritten"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn structural_probes_hold_locally() {
        // The engine's fail-open contract holds: missing-catalog recall is silent +
        // rebuild-free, and the kill-switch marker is detectable.
        let base = test_dir("probes-hold");
        let cfg = Config::default();
        assert_eq!(probe_missing_catalog_failopen(&base, &cfg), Some(true));
        assert_eq!(probe_surface_disabled_wired(&base), Some(true));
    }

    #[test]
    fn fix1_unavailable_scaffolding_skips_and_never_gates() {
        // FIX 1 (A7/A5): if the probe scaffolding cannot be created (here: `base` is
        // a FILE, so `create_dir_all(base/sub)` fails), the probes are SKIPPED
        // (`None`) — never a forced `Some(false)`.
        let dir = test_dir("probe-skip");
        let not_a_dir = dir.join("iam-a-file");
        std::fs::write(&not_a_dir, b"x").unwrap();
        assert_eq!(
            probe_missing_catalog_failopen(&not_a_dir, &Config::default()),
            None,
            "unwritable scaffolding → SKIPPED, not a failure"
        );
        assert_eq!(probe_surface_disabled_wired(&not_a_dir), None);

        // A report of SKIPPED probes must NOT gate the exit code (fail-open).
        let skipped = VerificationReport {
            mark_dir_writable: true,
            inert_advisory: None,
            missing_catalog_failopen: None,
            surface_disabled_wired: None,
        };
        assert!(
            skipped.structural_ok(),
            "skipped probes are advisory, never a gate"
        );

        // A genuine engine-contract BREAK (a probe that RAN and observed false) does
        // still gate exit 1.
        let broken = VerificationReport {
            missing_catalog_failopen: Some(false),
            ..skipped.clone()
        };
        assert!(
            !broken.structural_ok(),
            "an observed contract break still gates"
        );
        // A mix of SKIPPED + PASSED still passes.
        let mixed = VerificationReport {
            missing_catalog_failopen: Some(true),
            surface_disabled_wired: None,
            ..skipped.clone()
        };
        assert!(mixed.structural_ok());
    }
}
