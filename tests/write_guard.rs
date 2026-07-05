//! Write-guard conformance — the SINGLE fail-closed boundary (WP-4 / P9; D6, D21,
//! D5, A5, A6; §6). The most rigorous packet: EVERY deny reason ships a fixture
//! that DENIES and a near-miss fixture that ALLOWS, so a false-deny regression
//! (the #1-rule violation) is caught in both directions.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::guard::{
    DenyReason, GuardConfig, GuardVerdict, StoreRoots, check_write, write_context,
};
use rejolt::rebuild::{BuildConfig, rebuild};

// =============================================================================
// Helpers
// =============================================================================

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("rejolt-wp4-guard-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn put(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).expect("write file");
}

/// Write the grammar + memories, then rebuild the store (index + report present).
fn setup(tag: &str, grammar_text: &str, memories: &[(&str, &str)]) -> PathBuf {
    let store = unique_dir(tag);
    for (name, content) in memories {
        put(&store, name, content);
    }
    put(&store, "_grammar.toml", grammar_text);
    rebuild(
        &store,
        &store.join("_grammar.toml"),
        &BuildConfig::default(),
    )
    .expect("rebuild");
    store
}

fn cfg(store: &Path, box_root: Option<PathBuf>) -> GuardConfig {
    GuardConfig {
        grammar_path: store.join("_grammar.toml"),
        roots: StoreRoots { box_root },
    }
}

fn full_write(store: &Path, target: &Path, content: &str, cfg: &GuardConfig) -> GuardVerdict {
    check_write(store, target, content, true, cfg)
}

fn assert_deny(v: &GuardVerdict, code: &str) {
    match v {
        GuardVerdict::Deny(r) => assert_eq!(r.code(), code, "wrong deny reason: {r} ({r:?})"),
        GuardVerdict::Allow => panic!("expected Deny({code}), got Allow"),
    }
}

/// A grammar with a full arg + synonym vocab so byArg/bySynonym are populated once a
/// member memory carries the tag.
const RG_GRAMMAR: &str = "grammar-version = 1\n\n[tool.ripgrep]\ngloss = \"ripgrep\"\nplacement = \"either\"\ncommands = [\"rg\"]\nargs = [\"release\"]\nsynonyms = [\"grep\"]\n";

/// A member memory tagged `ripgrep`, so the grammar's `release`/`grep` vocab is
/// flattened into byArg/bySynonym at rebuild.
const RG_SEED: &str =
    "---\ndescription: seed memory for ripgrep vocab\nmetadata:\n  tags: [ripgrep]\n---\nbody\n";

// =============================================================================
// Tier 1 — shape / evidence (RB3)
// =============================================================================

#[test]
fn shape_evidence_denies_invalid_frontmatter_allows_valid() {
    let store = setup("shape", RG_GRAMMAR, &[("seed.md", RG_SEED)]);
    let c = cfg(&store, None);

    // DENY: top-level `triggers:` is a D21 named error at the guard.
    let bad = "---\ntriggers:\n  commands: [x]\nmetadata:\n  tags: [ripgrep]\n---\nbody\n";
    assert_deny(
        &full_write(&store, &store.join("bad.md"), bad, &c),
        "shape-evidence",
    );

    // ALLOW near-miss: a valid frontmatter-bearing memory.
    let good = "---\ndescription: a clean valid note\nmetadata:\n  tags: [ripgrep]\n---\nbody\n";
    assert!(
        full_write(&store, &store.join("good.md"), good, &c).is_allow(),
        "a valid memory must pass tier 1"
    );
}

// =============================================================================
// Tier 2 — static degenerate gate (with catalog vocab) + rescues + fail-open
// =============================================================================

#[test]
fn static_degeneracy_denies_with_vocab_rescues_allow() {
    let store = setup("static", RG_GRAMMAR, &[("seed.md", RG_SEED)]);
    let c = cfg(&store, None);
    let mk = |triggers: &str, desc: &str| {
        format!(
            "---\ndescription: {desc}\nmetadata:\n  tags: [ripgrep]\n  triggers:\n{triggers}---\nbody\n"
        )
    };

    // DENY: only a generic command, no lever (catalog vocab present).
    let degen = mk("    commands: [restart]\n", "only a generic command");
    assert_deny(
        &full_write(&store, &store.join("degen.md"), &degen, &c),
        "static-degenerate",
    );
    // DENY: only a broad path.
    let broad = mk("    paths: [\"/**\"]\n", "only a broad path");
    assert_deny(
        &full_write(&store, &store.join("broad.md"), &broad, &c),
        "static-degenerate",
    );

    // ALLOW near-miss: a routable arg rescues the generic command.
    let arg_rescue = mk(
        "    commands: [restart]\n    args: [release]\n",
        "restart with a routable arg",
    );
    assert!(
        full_write(&store, &store.join("arg.md"), &arg_rescue, &c).is_allow(),
        "a routable arg must rescue the static gate"
    );
    // ALLOW near-miss: a specific path rescues.
    let path_rescue = mk(
        "    commands: [restart]\n    paths: [\"/etc/foo.conf\"]\n",
        "restart with a specific path",
    );
    assert!(
        full_write(&store, &store.join("pathr.md"), &path_rescue, &c).is_allow(),
        "a specific path must rescue the static gate"
    );
}

#[test]
fn static_gate_fails_open_when_catalog_vocab_absent() {
    // A store with NO rebuilt index → read_artifacts is Missing → the static gate
    // (and every corpus-aware tier) is skipped → the degenerate write ALLOWS.
    let store = unique_dir("no-index");
    put(&store, "_grammar.toml", RG_GRAMMAR);
    let c = cfg(&store, None);
    let degen = "---\ndescription: degenerate with no index\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [restart]\n---\nbody\n";
    assert!(
        full_write(&store, &store.join("x.md"), degen, &c).is_allow(),
        "absent catalog vocab → static gate fails open"
    );
}

// =============================================================================
// Tier 3 — dedup backstop (new-file only; consolidation always passes)
// =============================================================================

#[test]
fn dedup_backstop_denies_new_duplicate_allows_consolidation_and_near_miss() {
    let existing = "---\ndescription: alpha beta charlie delta echo\nmetadata:\n  tags: [dedup-topic]\n---\nbody\n";
    let store = setup("dedup", RG_GRAMMAR, &[("existing.md", existing)]);
    let c = cfg(&store, None);

    // DENY: a NEW file identical to the existing memory (similarity 1.0 >= 0.85).
    let dup = "---\ndescription: alpha beta charlie delta echo\nmetadata:\n  tags: [dedup-topic]\n---\nbody\n";
    assert_deny(
        &full_write(&store, &store.join("dup-new.md"), dup, &c),
        "duplicate",
    );

    // ALLOW: existing-file consolidation always passes (target IS the existing file).
    assert!(
        full_write(&store, &store.join("existing.md"), dup, &c).is_allow(),
        "consolidation (existing file) must pass the dedup backstop"
    );

    // ALLOW near-miss: 3/5 shared description words → score 0.84 < 0.85.
    let near = "---\ndescription: alpha beta charlie foxtrot golf\nmetadata:\n  tags: [dedup-topic]\n---\nbody\n";
    assert!(
        full_write(&store, &store.join("near.md"), near, &c).is_allow(),
        "a 0.84 near-miss must pass (never a false deny at 0.84)"
    );
}

// =============================================================================
// Tier 4 — BLOCK-degenerate collision (new-file only)
// =============================================================================

#[test]
fn collision_block_degenerate_denies_guide_broad_allows() {
    // floor+1 = 9 member memories all routing via `rg` → a proposed `commands: [rg]`
    // co-fires with 9 (> floor 8) and declares no live lever → BLOCK-degenerate.
    let mut memories: Vec<(String, String)> = Vec::new();
    for i in 0..(rejolt::projection::COLLISION_GUIDE_FLOOR + 1) {
        memories.push((
            format!("m{i}.md"),
            format!(
                "---\ndescription: distinct desc {i}\nmetadata:\n  tags: [ripgrep]\n---\nbody\n"
            ),
        ));
    }
    let mem_refs: Vec<(&str, &str)> = memories
        .iter()
        .map(|(n, c)| (n.as_str(), c.as_str()))
        .collect();
    let store = setup("collision", RG_GRAMMAR, &mem_refs);
    let c = cfg(&store, None);

    // DENY: only `commands: [rg]` — broad co-fire, no live lever.
    let block = "---\ndescription: zzz proposed unique words\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [rg]\n---\nbody\n";
    assert_deny(
        &full_write(&store, &store.join("block.md"), block, &c),
        "collision-block-degenerate",
    );

    // ALLOW near-miss: the SAME broad co-fire, but a specific path is a live lever →
    // GUIDE-broad (advisory), never a block.
    let guide = "---\ndescription: yyy proposed unique words\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [rg]\n    paths: [\"/etc/specific-lever.conf\"]\n---\nbody\n";
    assert!(
        full_write(&store, &store.join("guide.md"), guide, &c).is_allow(),
        "a live lever downgrades BLOCK to advisory GUIDE-broad"
    );
}

// =============================================================================
// Tier 5 — high-confidence misplacement
// =============================================================================

const PLACEMENT_GRAMMAR: &str = "grammar-version = 1\n\n[domain.boxonly]\ngloss = \"box general fact\"\nplacement = \"box\"\ncommands = [\"boxcmd\"]\n\n[tool.ripgrep]\ngloss = \"ripgrep\"\nplacement = \"either\"\ncommands = [\"rg\"]\n";

#[test]
fn misplacement_denies_all_box_to_non_box_allows_mixed_and_box_target() {
    let store = setup("misplace", PLACEMENT_GRAMMAR, &[]);
    let box_root = fs::canonicalize(&store).unwrap();
    let c = cfg(&store, Some(box_root));
    // A RECOGNIZED non-box memory location (the `repo-memory` class, F24): the
    // placement gate applies here, per the ground-truth `_classify_target`.
    let repo_memory = unique_dir("outside").join("some-repo").join("memory");
    fs::create_dir_all(&repo_memory).unwrap();
    // An `other` target: no `memory` component — no grammar authority, no gate.
    let other = unique_dir("other-dir");

    // DENY: all grammar-known tags are box-placement, written to a RECOGNIZED
    // non-box store (repo memory/ dir).
    let boxmem = "---\ndescription: a box general fact\nmetadata:\n  tags: [boxonly]\n---\nbody\n";
    let v = full_write(&store, &repo_memory.join("mem.md"), boxmem, &c);
    assert_deny(&v, "misplacement");
    if let GuardVerdict::Deny(DenyReason::Misplacement { correct_box_path }) = &v {
        assert!(
            correct_box_path.contains("mem.md"),
            "the deny must name the correct box path: {correct_box_path}"
        );
    }

    // ALLOW: the SAME all-box memory at an `other` target fails open (F24 —
    // ground truth: 'other' targets are unchanged pass-through,
    // memory_surface.py:1663; pre-fix this was wrongly denied).
    assert!(
        full_write(&store, &other.join("mem.md"), boxmem, &c).is_allow(),
        "an `other` target has no grammar authority — the placement gate must not fire"
    );

    // ALLOW near-miss: mixed placement (box + either) → fails open, even in a
    // recognized non-box store.
    let mixed = "---\ndescription: mixed placement note\nmetadata:\n  tags: [boxonly, ripgrep]\n---\nbody\n";
    assert!(
        full_write(&store, &repo_memory.join("mixed.md"), mixed, &c).is_allow(),
        "mixed/either placement must fail open"
    );

    // ALLOW: the same all-box memory written UNDER the box store is correctly placed.
    assert!(
        full_write(&store, &store.join("inside.md"), boxmem, &c).is_allow(),
        "an all-box memory in the box store is correctly placed"
    );
}

// =============================================================================
// A6 — the diff-aware grammar-write surface
// =============================================================================

#[test]
fn grammar_write_denies_new_error_allows_clean_and_preexisting_error() {
    let dir = unique_dir("a6");
    let gpath = dir.join("_grammar.toml");
    let clean = "grammar-version = 1\n\n[tool.ripgrep]\ngloss = \"ripgrep\"\nplacement = \"either\"\ncommands = [\"rg\"]\n";
    fs::write(&gpath, clean).unwrap();
    let c = GuardConfig {
        grammar_path: gpath.clone(),
        roots: StoreRoots::default(),
    };

    // DENY: a fourth facet table is a NEW parse error vs the clean current file.
    let fourth = format!(
        "{clean}\n[platform.x]\ngloss = \"g\"\nplacement = \"either\"\ncommands = [\"c\"]\n"
    );
    assert_deny(&full_write(&dir, &gpath, &fourth, &c), "grammar-write");

    // DENY: a duplicate-facet tag is a NEW validation error vs the clean file.
    let dup_facet = "grammar-version = 1\n\n[tool.dupe]\ngloss = \"a\"\nplacement = \"either\"\ncommands = [\"c\"]\n\n[domain.dupe]\ngloss = \"b\"\nplacement = \"either\"\ncommands = [\"d\"]\n";
    assert_deny(&full_write(&dir, &gpath, dup_facet, &c), "grammar-write");

    // ALLOW: a clean grammar write introduces no error.
    let clean2 = "grammar-version = 1\n\n[tool.ripgrep]\ngloss = \"ripgrep!\"\nplacement = \"either\"\ncommands = [\"rg\"]\n";
    assert!(
        full_write(&dir, &gpath, clean2, &c).is_allow(),
        "a clean grammar write allows"
    );

    // ALLOW: a write that carries only a PRE-EXISTING error (not a new one). Point
    // the current file at an already-broken grammar; the proposed carries the SAME
    // broken tag → the error set does not grow → allow (never false-deny an edit to
    // an already-broken file).
    let broken = "grammar-version = 1\n\n[domain.weak]\ngloss = \"w\"\nplacement = \"either\"\nsynonyms = [\"foo\"]\n";
    fs::write(&gpath, broken).unwrap();
    let broken_plus = format!(
        "{broken}\n[tool.ripgrep]\ngloss = \"rg\"\nplacement = \"either\"\ncommands = [\"rg\"]\n"
    );
    assert!(
        full_write(&dir, &gpath, &broken_plus, &c).is_allow(),
        "a write carrying only a pre-existing error must allow (diff-aware)"
    );
}

#[test]
fn grammar_partial_edit_fails_open_and_bootstrap_allows() {
    let dir = unique_dir("a6-open");
    let gpath = dir.join("_grammar.toml");
    let clean = "grammar-version = 1\n\n[tool.rg]\ngloss = \"rg\"\nplacement = \"either\"\ncommands = [\"rg\"]\n";
    fs::write(&gpath, clean).unwrap();
    let c = GuardConfig {
        grammar_path: gpath.clone(),
        roots: StoreRoots::default(),
    };

    // A PARTIAL grammar edit (is_full_write = false) fails open — even one that
    // WOULD introduce a fourth-table error if it were the whole file.
    let would_break = "[platform.x]\ngloss = \"g\"\nplacement = \"either\"\ncommands = [\"c\"]\n";
    assert!(
        check_write(&dir, &gpath, would_break, false, &c).is_allow(),
        "a partial grammar edit must fail open"
    );

    // Bootstrap: the grammar file is ABSENT → any full write allows.
    let missing = dir.join("nope").join("_grammar.toml");
    let c2 = GuardConfig {
        grammar_path: missing.clone(),
        roots: StoreRoots::default(),
    };
    fs::create_dir_all(missing.parent().unwrap()).unwrap();
    let anything = "grammar-version = 1\n\n[platform.x]\ngloss = \"g\"\nplacement = \"e\"\n";
    assert!(
        check_write(&dir, &missing, anything, true, &c2).is_allow(),
        "grammar absent = bootstrap = allow"
    );
}

// =============================================================================
// Fail-open boundary (§2.9 / §6): partial edit, frontmatter-less, infra
// =============================================================================

#[test]
fn fail_open_partial_edit_frontmatterless_and_infra() {
    let store = setup("failopen", RG_GRAMMAR, &[("seed.md", RG_SEED)]);
    let c = cfg(&store, None);
    // Even a would-be-degenerate memory content passes as a PARTIAL edit.
    let degen =
        "---\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [restart]\n---\nbody\n";
    assert!(
        check_write(&store, &store.join("m.md"), degen, false, &c).is_allow(),
        "a partial edit always fails open"
    );
    // Frontmatter-less content is not a memory → fail open (never a shape deny).
    assert!(
        full_write(
            &store,
            &store.join("code.md"),
            "fn main() {}\nno frontmatter here\n",
            &c
        )
        .is_allow(),
        "frontmatter-less content fails open"
    );
    // An infra file (underscore-prefixed) is exempt before any tier.
    assert!(
        full_write(&store, &store.join("_notes.md"), degen, &c).is_allow(),
        "infra files are exempt"
    );
}

// =============================================================================
// Write-context payload (§6): schema + digest + candidates + placement; budget
// =============================================================================

#[test]
fn write_context_carries_schema_digest_candidates_and_placement() {
    let existing = "---\ndescription: gpu vram tips\nmetadata:\n  tags: [ripgrep]\n---\nbody\n";
    let store = setup("wc", RG_GRAMMAR, &[("existing.md", existing)]);
    let c = cfg(&store, None);

    let proposed =
        "---\ndescription: gpu vram diagnostics\nmetadata:\n  tags: [ripgrep]\n---\nbody\n";
    let wc = write_context(&store, &store.join("new.md"), proposed, &c);

    assert!(wc.text.contains("metadata.triggers"), "schema hint present");
    assert!(wc.text.contains("ripgrep"), "grammar vocab digest present");
    assert!(
        !wc.dedup_candidates.is_empty(),
        "a similar existing memory surfaces as a dedup candidate"
    );
    assert!(
        wc.dedup_candidates.iter().all(|d| d.score >= 0.2),
        "above the 0.2 floor"
    );
    assert!(
        wc.placement_guidance.is_some(),
        "placement guidance present"
    );
    assert!(
        !wc.used_digest_fallback,
        "a small grammar fits under budget"
    );
    assert!(wc.text.chars().count() <= rejolt::guard::WRITE_CONTEXT_BUDGET);
}

#[test]
fn write_context_uses_digest_fallback_over_budget() {
    // A large grammar whose FULL digest exceeds the 9500-char budget forces the
    // one-line-per-tag fallback.
    let mut g = String::from("grammar-version = 1\n");
    for i in 0..80 {
        g.push_str(&format!(
            "\n[domain.tag{i}]\ngloss = \"a reasonably long human gloss for tag number {i} here\"\nplacement = \"either\"\ncommands = [\"command-for-tag-{i}\"]\npaths = [\"/some/specific/place/for/tag/{i}/**\"]\nargs = [\"arg-{i}\"]\nsynonyms = [\"synonym-{i}\"]\n"
        ));
    }
    let store = setup("wc-big", &g, &[]);
    let c = cfg(&store, None);
    let proposed = "---\ndescription: a note\nmetadata:\n  tags: [tag0]\n---\nbody\n";
    let wc = write_context(&store, &store.join("new.md"), proposed, &c);
    assert!(
        wc.used_digest_fallback,
        "a >9500-char full digest must trigger the one-line-per-tag fallback"
    );
}

// =============================================================================
// FIX 1 (MAJOR) — is_new resolved via engine-realpath, so a consolidation
// addressed by a non-canonical (`~`-expanded) path is EXEMPT from the new-file
// tiers, not false-denied (§6/§7 consolidation exemption; the #1 failure mode).
// =============================================================================

#[test]
fn consolidation_via_noncanonical_tilde_path_is_exempt_from_new_file_tiers() {
    // A `~`-addressed path resolves via engine-realpath but is literally absent, so
    // deriving is_new from the RAW path would mis-see a rewrite as a new file. We
    // need a store UNDER $HOME for `~` to resolve to it (hermetic: a unique subdir
    // we create + remove).
    let Some(home) = std::env::var_os("HOME") else {
        return; // no HOME to anchor a `~` path; nothing to demonstrate
    };
    let home = PathBuf::from(home);
    static N: AtomicU32 = AtomicU32::new(0);
    let rel = format!(
        ".cache/rejolt-wp4-consol-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    );
    let store = home.join(&rel);
    fs::create_dir_all(&store).unwrap();

    // floor+1 member memories routing via `rg` → a `commands:[rg]` set
    // BLOCK-degenerates as a NEW file (no live lever).
    for i in 0..(rejolt::projection::COLLISION_GUIDE_FLOOR + 1) {
        fs::write(
            store.join(format!("m{i}.md")),
            format!("---\ndescription: distinct member wording {i}\nmetadata:\n  tags: [ripgrep]\n---\nbody\n"),
        )
        .unwrap();
    }
    // The existing memory being consolidated (already on disk).
    fs::write(
        store.join("existing.md"),
        "---\ndescription: old subject placeholder\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [rg]\n---\nbody\n",
    )
    .unwrap();
    fs::write(store.join("_grammar.toml"), RG_GRAMMAR).unwrap();
    rebuild(
        &store,
        &store.join("_grammar.toml"),
        &BuildConfig::default(),
    )
    .expect("rebuild");
    let c = cfg(&store, None);

    // The fresh consolidation content: a would-BLOCK trigger set (commands:[rg]) but
    // a description distinct from every corpus memory (so the deny below is the
    // COLLISION tier, not the dedup tier).
    let fresh = "---\ndescription: brand fresh consolidation wording\nmetadata:\n  tags: [ripgrep]\n  triggers:\n    commands: [rg]\n---\nbody\n";

    // Sanity: the SAME content to a genuinely-NEW path is BLOCK-degenerate.
    let new_tilde = PathBuf::from(format!("~/{rel}/genuinely-new.md"));
    assert!(
        !new_tilde.exists(),
        "the raw `~` path is not literally on disk"
    );
    assert_deny(
        &full_write(&store, &new_tilde, fresh, &c),
        "collision-block-degenerate",
    );

    // THE FIX: the SAME content to `~/{rel}/existing.md` resolves (engine-realpath)
    // to the EXISTING file → is_new=false → exempt from dedup + collision → ALLOW.
    let existing_tilde = PathBuf::from(format!("~/{rel}/existing.md"));
    assert!(
        !existing_tilde.exists(),
        "the raw `~` path is not literally on disk"
    );
    assert!(
        full_write(&store, &existing_tilde, fresh, &c).is_allow(),
        "a `~`-addressed consolidation must be exempt (is_new=false), never false-denied"
    );

    let _ = fs::remove_dir_all(&store);
}

// =============================================================================
// FIX 2 (MINOR) — A6 diff-aware guard allows over an UNPARSEABLE current grammar
// (no sound diff exists against an unenumerable baseline; parity with bootstrap).
// =============================================================================

#[test]
fn grammar_write_allows_over_unparseable_baseline() {
    let dir = unique_dir("a6-unparse");
    let gpath = dir.join("_grammar.toml");
    // A malformed-TOML current grammar (unparseable): unterminated table header.
    fs::write(&gpath, "grammar-version = 1\n[tool.x\ngloss = broken\n").unwrap();
    let c = GuardConfig {
        grammar_path: gpath.clone(),
        roots: StoreRoots::default(),
    };

    // A proposed write that FIXES the syntax but still carries a validation error
    // (synonyms-only tag → no-evidence). Over an unparseable baseline → ALLOW: a
    // strictly-better, now-parseable write is never false-denied.
    let fixed_but_invalid = "grammar-version = 1\n\n[domain.weak]\ngloss = \"w\"\nplacement = \"either\"\nsynonyms = [\"foo\"]\n";
    assert!(
        full_write(&dir, &gpath, fixed_but_invalid, &c).is_allow(),
        "an unparseable current baseline must allow (like bootstrap)"
    );

    // Contrast: a CLEAN current baseline + the SAME new validation error → DENY,
    // so the fix does not weaken the clean-baseline deny.
    fs::write(
        &gpath,
        "grammar-version = 1\n\n[tool.ok]\ngloss = \"ok\"\nplacement = \"either\"\ncommands = [\"ok\"]\n",
    )
    .unwrap();
    assert_deny(
        &full_write(&dir, &gpath, fixed_but_invalid, &c),
        "grammar-write",
    );
}
