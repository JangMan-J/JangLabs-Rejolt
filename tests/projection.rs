//! Collision-projection conformance (WP-4 / P10; D8, D4, A2, §1, §7). Covers RB9
//! (recall ≡ projection through the ONE walk), the strict-`>`-floor verdict
//! semantics, and the shared §1 live-lever definition.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rejolt::frontmatter::Triggers;
use rejolt::grammar::parse_and_validate;
use rejolt::index::{Index, IndexRecord, WalkQuery};
use rejolt::projection::{
    COLLISION_GUIDE_FLOOR, Projection, Verdict, live_levers, project, walk_query_for,
};
use rejolt::rebuild::{BuildConfig, build_artifacts, scan_store};
use rejolt::tier::{Axis, Source};

// =============================================================================
// Helpers
// =============================================================================

fn unique_store(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("rejolt-wp4-proj-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp store");
    dir
}

fn put(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).expect("write store file");
}

const RG_GRAMMAR: &str = "grammar-version = 1\n\n[tool.ripgrep]\ngloss = \"ripgrep\"\nplacement = \"either\"\ncommands = [\"rg\"]\nsynonyms = [\"grep\"]\n";

fn index_from_store(store: &Path, grammar_text: &str) -> Index {
    let grammar = parse_and_validate(grammar_text).expect("valid grammar");
    let (memories, malformed) = scan_store(store).expect("scan store");
    let artifacts = build_artifacts(
        &memories,
        &malformed,
        &grammar,
        grammar_text,
        &BuildConfig::default(),
    );
    Index::from_records(artifacts.records)
}

fn triggers(commands: &[&str], paths: &[&str], args: &[&str], synonyms: &[&str]) -> Triggers {
    Triggers {
        commands: commands.iter().map(|s| s.to_string()).collect(),
        paths: paths.iter().map(|s| s.to_string()).collect(),
        args: args.iter().map(|s| s.to_string()).collect(),
        synonyms: synonyms.iter().map(|s| s.to_string()).collect(),
    }
}

/// A synthetic index whose `byCommand[cmd]` routes to `n` distinct memories — a
/// direct way to dial co-fire breadth for the verdict tests.
fn wide_command_index(cmd: &str, n: usize) -> Index {
    let recs = (0..n)
        .map(|i| IndexRecord {
            axis: Axis::Command,
            pattern: cmd.to_string(),
            route_tag: format!("m{i}"),
            source: Source::Memory,
            memory_id: format!("m{i}"),
            mem_type: String::new(),
            last_reviewed: String::new(),
            decline_count: 0,
            tags: vec!["t".into()],
            path: format!("/store/m{i}.md"),
            snippet: "d".into(),
        })
        .collect();
    Index::from_records(recs)
}

// =============================================================================
// RB9 — recall ≡ projection through the ONE walk (no second matcher, D4)
// =============================================================================

#[test]
fn rb9_projection_uses_the_same_walk_as_recall() {
    let store = unique_store("rb9");
    // Two existing memories that route via the ripgrep grammar tag (rg / grep).
    put(
        &store,
        "a.md",
        "---\nmetadata:\n  tags: [ripgrep]\n---\nbody\n",
    );
    put(
        &store,
        "b.md",
        "---\nmetadata:\n  tags: [ripgrep]\n---\nbody\n",
    );
    let idx = index_from_store(&store, RG_GRAMMAR);

    // A proposed NEW memory whose triggers are `rg` (command) + `grep` (synonym).
    let t = triggers(&["rg"], &[], &[], &["grep"]);

    // Projection walks the ONE index walk; the co-fire set is exactly the distinct
    // memory ids the SAME walk (built from the SAME trigger set) returns — proving
    // there is no second matcher (RB9). For a commands+synonyms set the projection's
    // walk query is byte-identical to the query recall would build.
    let proj = project(&t, &idx);
    let wq = walk_query_for(&t);
    let recall_style = WalkQuery {
        commands: vec!["rg".into()],
        synonyms: vec!["grep".into()],
        ..Default::default()
    };
    assert_eq!(wq, recall_style, "projection builds recall's walk query");

    let walk_ids: std::collections::BTreeSet<String> = idx
        .walk(&wq)
        .iter()
        .map(|h| h.record.memory_id.clone())
        .collect();
    let proj_ids: std::collections::BTreeSet<String> = proj.collisions.iter().cloned().collect();
    assert_eq!(
        proj_ids, walk_ids,
        "projection co-fires == the one walk's distinct hit set (RB9)"
    );
    assert_eq!(
        proj_ids,
        ["a".to_string(), "b".to_string()].into_iter().collect()
    );
    assert_eq!(proj.distinct_count, 2);
    // per_trigger is per-axis breadth, not a sum: command axis touched both, synonym
    // axis touched both, but distinct_count stays 2.
    assert_eq!(proj.per_trigger.get("command"), Some(&2));
    assert_eq!(proj.per_trigger.get("synonym"), Some(&2));
}

// =============================================================================
// Verdict: strict `>` floor (the load-bearing comparison)
// =============================================================================

#[test]
fn verdict_strict_greater_than_floor() {
    // breadth == floor → PASS.
    let at_floor = wide_command_index("restart", COLLISION_GUIDE_FLOOR);
    let p = project(&triggers(&["restart"], &[], &[], &[]), &at_floor);
    assert_eq!(p.distinct_count, COLLISION_GUIDE_FLOOR);
    assert_eq!(p.verdict, Verdict::Pass, "breadth == floor must PASS");

    // floor + 1, no live lever → BLOCK-degenerate.
    let over = wide_command_index("restart", COLLISION_GUIDE_FLOOR + 1);
    let p = project(&triggers(&["restart"], &[], &[], &[]), &over);
    assert_eq!(p.distinct_count, COLLISION_GUIDE_FLOOR + 1);
    assert!(p.live_levers.is_empty());
    assert_eq!(
        p.verdict,
        Verdict::BlockDegenerate,
        "floor+1 empty levers → BLOCK"
    );

    // floor + 1, but a specific path IS a live lever → GUIDE-broad (advisory).
    let p = project(
        &triggers(&["restart"], &["/etc/specific.conf"], &[], &[]),
        &over,
    );
    assert!(!p.live_levers.is_empty());
    assert_eq!(
        p.verdict,
        Verdict::GuideBroad,
        "floor+1 with a live lever → GUIDE"
    );
}

// =============================================================================
// The shared §1 live-lever definition (index-key membership, never hit counts)
// =============================================================================

#[test]
fn live_levers_are_index_key_membership() {
    let store = unique_store("levers");
    // Grammar declares an arg vocab (`release`) and a synonym vocab (`grep`).
    let grammar = "grammar-version = 1\n\n[tool.cargo]\ngloss = \"cargo\"\nplacement = \"either\"\ncommands = [\"cargo\"]\nargs = [\"release\"]\nsynonyms = [\"grep\"]\n";
    put(
        &store,
        "m.md",
        "---\nmetadata:\n  tags: [cargo]\n---\nbody\n",
    );
    let idx = index_from_store(&store, grammar);

    // arg live via byArg; synonym live via bySynonym; specific path live w/o catalog;
    // broad path + unknown arg/synonym are dead.
    let live = live_levers(
        &triggers(
            &[],
            &["/**", "/etc/x.conf"],
            &["release", "nope"],
            &["grep", "nope"],
        ),
        &idx,
    );
    assert_eq!(live.args, vec!["release".to_string()]);
    assert_eq!(live.synonyms, vec!["grep".to_string()]);
    assert_eq!(live.paths, vec!["/etc/x.conf".to_string()]);
    assert!(!live.is_empty());

    // The SAME definition the static gate reads — a dead-only set has no live lever.
    let dead = live_levers(&triggers(&[], &["/**"], &["nope"], &["nope"]), &idx);
    assert!(dead.is_empty());
}

// =============================================================================
// Fail-open: the empty projection carries live_levers (§7)
// =============================================================================

#[test]
fn empty_projection_carries_levers_and_passes() {
    let idx = Index::default();
    let live = live_levers(&triggers(&[], &["/etc/specific.conf"], &[], &[]), &idx);
    let p = Projection::empty(live);
    assert_eq!(p.verdict, Verdict::Pass);
    assert_eq!(p.distinct_count, 0);
    assert!(
        !p.live_levers.is_empty(),
        "the write can still read levers on a fault"
    );
}
