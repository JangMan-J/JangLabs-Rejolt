//! Bootstrap conformance (WP-7 / P14; D13, D17, D23, A7, §13). Idempotence,
//! never-overwrite, the empty grammar seed, and the bootstrap-local fail-open
//! verification rows. (The `--print-hooks` D13 "engine never writes host settings"
//! proof is driven end-to-end through the binary in `tests/cli_contract.rs`.)

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::bootstrap::{self, EMPTY_GRAMMAR_SEED};
use rejolt::config::Config;
use rejolt::grammar;
use rejolt::rebuild::{index_path, report_path};

fn unique_base(tag: &str) -> std::path::PathBuf {
    static C: AtomicU32 = AtomicU32::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("rejolt-boot-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A snapshot of the store's OBSERVABLE regular-file state: filename → content
/// (symlinks are read through, so `_grammar.toml` reads as the grammar text).
fn snapshot(store: &Path) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for e in std::fs::read_dir(store).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Ok(content) = std::fs::read_to_string(e.path()) {
            m.insert(name, content);
        }
    }
    m
}

#[test]
fn empty_grammar_seed_is_version_line_alone_and_validates() {
    // R5/OWNER: the empty seed is exactly `grammar-version = 1` and it validates
    // (the ONLY zero-evidence grammar that passes — D23).
    assert_eq!(EMPTY_GRAMMAR_SEED, "grammar-version = 1\n");
    assert!(grammar::parse_and_validate(EMPTY_GRAMMAR_SEED).is_ok());
}

#[test]
fn bootstrap_seeds_expected_files_and_is_idempotent() {
    let base = unique_base("idem");
    let store = base.join("store");
    let grammar = base.join("lab").join("_grammar.toml"); // lab grammar OUTSIDE the store

    let r1 = bootstrap::bootstrap(&store, &grammar, &Config::default()).expect("bootstrap");
    // Expected files exist (§13): MEMORY.md, the store-side grammar symlink, and a
    // valid empty catalog pair.
    assert!(store.join("MEMORY.md").exists(), "MEMORY.md");
    assert!(
        store.join("_grammar.toml").exists(),
        "store grammar symlink"
    );
    assert!(index_path(&store).exists(), "flat index");
    assert!(report_path(&store).exists(), "catalog report");
    // The empty seed was written to the lab grammar (absent before).
    assert_eq!(
        std::fs::read_to_string(&grammar).unwrap(),
        EMPTY_GRAMMAR_SEED
    );
    // Empty store → 0 unroutable.
    assert_eq!(
        r1.rebuild.unroutable_count, 0,
        "routabilityReport: 0 unroutable"
    );
    assert!(r1.store_created && r1.grammar_seeded && r1.grammar_symlinked && r1.memory_created);

    // The store-side grammar is a SYMLINK (not a copy) — §13 install-manifest.
    let meta = std::fs::symlink_metadata(store.join("_grammar.toml")).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "_grammar.toml must be a symlink"
    );

    // Idempotence: a second run with no input change → identical observable state.
    let snap1 = snapshot(&store);
    let r2 = bootstrap::bootstrap(&store, &grammar, &Config::default()).expect("bootstrap 2");
    let snap2 = snapshot(&store);
    assert_eq!(
        snap1, snap2,
        "a second bootstrap must leave identical observable state"
    );
    assert!(
        !r2.store_created && !r2.grammar_seeded && !r2.grammar_symlinked && !r2.memory_created,
        "a second bootstrap creates nothing"
    );
    // The catalog was atomically rewritten to the SAME generation (deterministic).
    assert_eq!(
        r1.rebuild.generation, r2.rebuild.generation,
        "generation is deterministic"
    );
}

#[test]
fn bootstrap_never_overwrites_user_files() {
    let base = unique_base("noclobber");
    let store = base.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let grammar = base.join("_grammar.toml");

    // Pre-existing user files: a custom MEMORY.md and a real (non-empty) grammar.
    std::fs::write(
        store.join("MEMORY.md"),
        "# CUSTOM ROUTER\nseat-block: keep-me\n",
    )
    .unwrap();
    let user_grammar = "grammar-version = 1\n\n[domain.gpu]\ngloss = \"gpu\"\nplacement = \"box\"\ncommands = [\"nvidia-smi\"]\n";
    std::fs::write(&grammar, user_grammar).unwrap();
    // A pre-existing ordinary memory must survive untouched too.
    let mem = "---\nname: keep\ndescription: keep me\nmetadata:\n  tags: [misc]\n---\nbody\n";
    std::fs::write(store.join("keep.md"), mem).unwrap();

    bootstrap::bootstrap(&store, &grammar, &Config::default()).expect("bootstrap");

    assert_eq!(
        std::fs::read_to_string(store.join("MEMORY.md")).unwrap(),
        "# CUSTOM ROUTER\nseat-block: keep-me\n",
        "MEMORY.md must never be overwritten"
    );
    assert_eq!(
        std::fs::read_to_string(&grammar).unwrap(),
        user_grammar,
        "an existing grammar must never be overwritten with the empty seed"
    );
    assert_eq!(
        std::fs::read_to_string(store.join("keep.md")).unwrap(),
        mem,
        "an existing memory must never be touched"
    );
}

#[test]
fn bootstrap_verification_rows_hold() {
    // The bootstrap-local fail-open rows: recall on a missing catalog is silent +
    // rebuild-free, and the .surface-disabled kill-switch is structurally wired.
    let base = unique_base("verify");
    let store = base.join("store");
    let grammar = base.join("_grammar.toml");
    let report = bootstrap::bootstrap(&store, &grammar, &Config::default()).expect("bootstrap");
    assert_eq!(
        report.verification.missing_catalog_failopen,
        Some(true),
        "missing-catalog fail-open"
    );
    assert_eq!(
        report.verification.surface_disabled_wired,
        Some(true),
        ".surface-disabled wired"
    );
    assert!(
        report.verification.structural_ok(),
        "both structural rows must hold"
    );
}

/// FIX 1 (A7/A5): a correct bootstrap must NOT fail its exit code just because probe
/// scaffolding could not be created — the store was seeded correctly. The probes now
/// run inside the freshly-seeded (known-writable) store, so a hostile `$TMPDIR` is
/// irrelevant, and a skipped probe never gates. This drives the BINARY with `TMPDIR`
/// at a read-only path and asserts exit 0 + a correctly-seeded store.
#[test]
fn bootstrap_exit_zero_even_with_readonly_tmpdir() {
    use std::process::{Command, Stdio};

    let base = unique_base("ro-tmp");
    let store = base.join("store");
    let grammar = base.join("_grammar.toml");

    // A read-only TMPDIR (mode 0o500): the OLD probes scaffolded under $TMPDIR and
    // would have returned false → exit 1; the FIXED probes scaffold under the store.
    let ro_tmp = base.join("ro-tmp");
    std::fs::create_dir_all(&ro_tmp).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ro_tmp, std::fs::Permissions::from_mode(0o500)).unwrap();
    }

    let bin = env!("CARGO_BIN_EXE_rejolt");
    let out = Command::new(bin)
        .args([
            "bootstrap",
            "--store",
            store.to_str().unwrap(),
            "--grammar",
            grammar.to_str().unwrap(),
        ])
        .env("TMPDIR", &ro_tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run bootstrap");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&ro_tmp, std::fs::Permissions::from_mode(0o700));
    }

    assert_eq!(
        out.status.code(),
        Some(0),
        "a correct bootstrap must exit 0 despite a read-only TMPDIR; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(store.join("MEMORY.md").exists());
    assert!(store.join("_flat_index.tsv").exists());
    assert!(store.join("_memory_catalog.json").exists());
    // No probe scaffolding leaked into the store's observable state.
    let leaked: Vec<_> = std::fs::read_dir(&store)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".rejolt-verify")
        })
        .collect();
    assert!(leaked.is_empty(), "probe scaffolding must be cleaned up");
}

/// A pre-existing INVALID user grammar is a config/taxonomy error (exit-2 class),
/// surfaced — never silently overwritten with the empty seed.
#[test]
fn bootstrap_rejects_a_preexisting_invalid_grammar() {
    let base = unique_base("badgrammar");
    let store = base.join("store");
    let grammar = base.join("_grammar.toml");
    // A grammar tag with synonyms-only evidence fails validation (D3, exit 2).
    std::fs::write(
        &grammar,
        "grammar-version = 1\n\n[domain.weak]\ngloss = \"w\"\nplacement = \"either\"\nsynonyms = [\"foo\"]\n",
    )
    .unwrap();
    let err = bootstrap::bootstrap(&store, &grammar, &Config::default())
        .expect_err("an invalid grammar must fail bootstrap");
    assert_eq!(
        err.exit_code(),
        2,
        "grammar error is config/taxonomy (exit 2)"
    );
}
