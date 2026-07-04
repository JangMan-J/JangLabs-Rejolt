//! The write guard — the system's SINGLE fail-closed boundary (plan P9; D6, D21,
//! D5, A5, A6; CORE-SPEC §6).
//!
//! On an otherwise fail-open, advisory system, [`check_write`] is the ONE surface
//! that may hard-deny a host operation, and only for a **full-file write of a
//! frontmatter-bearing memory** or a **full-file write of the grammar file** (A6).
//! Everything else — a partial edit, frontmatter-less content, a non-memory path,
//! any index/projection fault — **fails open** (allows). A false DENY of a
//! legitimate write is the worst failure mode (the #1 rule), so this module denies
//! only for the enumerated reasons and never invents one.
//!
//! ## The tiers, in order (§6)
//!
//! For a memory write:
//! 1. **Shape / evidence** — [`crate::frontmatter::parse`]; invalid frontmatter or
//!    a missing/invalid `metadata` block denies (naming the violated rule, RB3).
//! 2. **Static degenerate gate** — a trigger set whose only evidence is degenerate
//!    across two arms (only generic/low-signal commands; only broad paths). Any
//!    live lever (§1) rescues. Fails open with no loaded index (catalog vocab
//!    absent — can't judge arg/synonym liveness).
//! 3. **Dedup backstop** — `0.6·tag_overlap + 0.4·bow-cosine ≥ 0.85` denies a
//!    NEW-FILE write, naming the existing memory. Consolidation always passes.
//! 4. **Collision tier** — a BLOCK-degenerate projection verdict (§7) hard-denies a
//!    NEW-FILE write; GUIDE-broad is advisory; any projection fault fails open.
//! 5. **High-confidence misplacement** — a memory ALL of whose grammar-known tags
//!    carry `placement: box`, written to a non-box target, denies (naming the box
//!    path). Ambiguous/mixed/unknown/`project`/`either` placement fails open.
//!
//! For the grammar file, the A6 diff-aware surface: a full-file grammar write is
//! denied iff it introduces validation errors the CURRENT file does not have; file
//! absent = bootstrap = allow; a partial grammar edit fails open.
//!
//! The unified §1 live-lever definition ([`crate::projection::live_levers`]) drives
//! BOTH tier 2 (static gate) and tier 4 (collision verdict), so the two can never
//! disagree; and tier 4 reuses [`crate::projection::project`], which walks the ONE
//! [`crate::index::Index::walk`] recall uses — there is no second matcher (D4).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::catalog::{MemorySummary, read_artifacts};
use crate::frontmatter::{self, Frontmatter, FrontmatterError, Triggers};
use crate::grammar::{self, Grammar, Placement, render_digest};
use crate::index::Index;
use crate::projection::{COLLISION_GUIDE_FLOOR, Verdict, live_levers, project};
use crate::rebuild::{index_path, report_path};
use crate::recall::GENERIC_VERBS;

// =============================================================================
// §10 constants (hardcoded — never loosen silently)
// =============================================================================

/// `DEDUP_BACKSTOP_THRESHOLD` (§10, hardcoded 0.85). A new-file write scoring at or
/// above this against an existing memory is denied as a duplicate.
pub const DEDUP_BACKSTOP_THRESHOLD: f64 = 0.85;
/// `DEDUP_CANDIDATE_FLOOR` (§10, hardcoded 0.2). Advisory display floor for
/// write-context dedup candidates — **not** a deny.
pub const DEDUP_CANDIDATE_FLOOR: f64 = 0.2;
/// `WRITE_CONTEXT_BUDGET` (§10, hardcoded 9500 chars). The write-context payload
/// uses the one-line-per-tag digest fallback when the full digest would exceed it.
pub const WRITE_CONTEXT_BUDGET: usize = 9500;
/// How many dedup candidates the write-context surfaces (top-N above the floor).
const DEDUP_TOP_N: usize = 5;

// =============================================================================
// Public verdict + deny reasons
// =============================================================================

/// The write guard's verdict. [`GuardVerdict::Deny`] is the ONLY fail-closed
/// outcome in the whole engine; every other path allows (§2.9).
#[derive(Debug, Clone, PartialEq)]
pub enum GuardVerdict {
    /// The write proceeds (the default; fail-open).
    Allow,
    /// The write is denied for exactly one enumerated reason (§6).
    Deny(DenyReason),
}

impl GuardVerdict {
    /// Whether this verdict allows the write.
    pub fn is_allow(&self) -> bool {
        matches!(self, GuardVerdict::Allow)
    }
    /// The deny reason, if any.
    pub fn deny_reason(&self) -> Option<&DenyReason> {
        match self {
            GuardVerdict::Deny(r) => Some(r),
            GuardVerdict::Allow => None,
        }
    }
}

/// The enumerated write-guard deny reasons (§6 / §14 "Write deny reasons"). This is
/// the closed set — the boundary denies for these and nothing else.
#[derive(Debug, Clone, PartialEq)]
pub enum DenyReason {
    /// Tier 1: invalid frontmatter shape or a missing/invalid `metadata` block. The
    /// inner [`FrontmatterError`] names the violated dialect/schema rule (RB3).
    ShapeEvidence(FrontmatterError),
    /// Tier 2: the trigger set is statically degenerate — its only evidence is
    /// generic/low-signal commands and/or broad paths, with no live lever.
    StaticDegenerate(StaticArm),
    /// Tier 3: a new-file write duplicates an existing memory (best similarity
    /// `>= 0.85`).
    Duplicate {
        /// The existing memory's id.
        existing_id: String,
        /// The existing memory's path.
        existing_path: String,
        /// The similarity score `[0, 1]`.
        score: f64,
    },
    /// Tier 4: a BLOCK-degenerate collision verdict (§7): co-fire breadth strictly
    /// above the floor AND no live lever.
    BlockDegenerate {
        /// The co-fire breadth (`distinct_count`).
        distinct_count: usize,
        /// The `collisionGuideFloor` it exceeded.
        floor: usize,
    },
    /// Tier 5: high-confidence misplacement — an all-`box`-tagged memory written to
    /// a non-box target. Carries the correct box path (self-healing).
    Misplacement {
        /// The path in the box store the memory should be written to.
        correct_box_path: String,
    },
    /// A6: a full-file grammar write introducing NEW validation errors.
    GrammarWrite {
        /// The error signatures the proposed grammar has that the current lacks.
        new_errors: Vec<String>,
    },
}

/// Which arm of the static degenerate gate fired (§6 tier 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticArm {
    /// Arm (a): the commands are all generic/low-signal, with no narrowing lever.
    GenericCommands(Vec<String>),
    /// Arm (b): the only behavioral evidence is broad glob(s).
    BroadPaths(Vec<String>),
}

impl DenyReason {
    /// A stable code for the deny reason (test/telemetry discriminator).
    pub fn code(&self) -> &'static str {
        match self {
            DenyReason::ShapeEvidence(_) => "shape-evidence",
            DenyReason::StaticDegenerate(_) => "static-degenerate",
            DenyReason::Duplicate { .. } => "duplicate",
            DenyReason::BlockDegenerate { .. } => "collision-block-degenerate",
            DenyReason::Misplacement { .. } => "misplacement",
            DenyReason::GrammarWrite { .. } => "grammar-write",
        }
    }
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DenyReason::ShapeEvidence(e) => {
                write!(f, "invalid memory shape/evidence: {e}")
            }
            DenyReason::StaticDegenerate(StaticArm::GenericCommands(cmds)) => write!(
                f,
                "triggers.commands are only generic/low-signal commands ({}) with no narrowing \
                 routable arg, routable synonym, or specific path — they carry no routing signal",
                cmds.join(", ")
            ),
            DenyReason::StaticDegenerate(StaticArm::BroadPaths(paths)) => write!(
                f,
                "triggers.paths are only overly-broad glob(s) ({}) with no domain signal — use a \
                 specific path (e.g. ~/.config/foo/**)",
                paths.join(", ")
            ),
            DenyReason::Duplicate {
                existing_id,
                existing_path,
                score,
            } => write!(
                f,
                "this new memory duplicates existing memory `{existing_id}` ({existing_path}) \
                 (similarity {score:.2} >= {DEDUP_BACKSTOP_THRESHOLD}); consolidate into it instead \
                 of writing a new file"
            ),
            DenyReason::BlockDegenerate {
                distinct_count,
                floor,
            } => write!(
                f,
                "this trigger set is degenerately broad: it co-fires with {distinct_count} other \
                 memories (> floor {floor}) yet declares no live narrowing lever — add a routable \
                 arg, a routable synonym, or a specific path"
            ),
            DenyReason::Misplacement { correct_box_path } => write!(
                f,
                "this memory's tags are all box-placement; write it to {correct_box_path} instead \
                 (box-general facts belong in the box-brain store)"
            ),
            DenyReason::GrammarWrite { new_errors } => write!(
                f,
                "this grammar write introduces new validation error(s): {}",
                new_errors.join(", ")
            ),
        }
    }
}

// =============================================================================
// Guard configuration (roots + grammar path)
// =============================================================================

/// The configured memory-store roots the placement classifier compares a target
/// against (§3, §5.x). WP-4 carries only the box root — the one root high-confidence
/// misplacement needs. **WP-7 wires** the real roots from `config.toml` and may
/// extend this with project / repo-memory roots for finer placement classification.
#[derive(Debug, Clone, Default)]
pub struct StoreRoots {
    /// The box-brain store root. `None` → placement enforcement fails open (a target
    /// cannot be classified box vs non-box, so no misplacement deny can be made).
    pub box_root: Option<PathBuf>,
}

/// The guard's out-of-band configuration: where the grammar file lives (for the A6
/// surface and the placement lookup) and the store roots. WP-5 (hook wiring) / WP-7
/// (CLI + config) construct this from the real store + `config.toml`.
#[derive(Debug, Clone)]
pub struct GuardConfig {
    /// The configured grammar file path (may be a store-side symlink; resolved via
    /// engine-realpath). A write to this path enters the A6 grammar surface.
    pub grammar_path: PathBuf,
    /// The store roots for placement classification.
    pub roots: StoreRoots,
}

// =============================================================================
// check_write — the entry point (WP-5 consumes this)
// =============================================================================

/// The write guard (§6). Enters a fail-closed path ONLY for a full-file write of a
/// frontmatter-bearing memory OR a full-file write of the grammar file (A6);
/// everything else fails open (allows).
///
/// - `store`: the store dir (holds the flat index + catalog report).
/// - `target_path`: where the write lands (the independent memory/grammar check).
/// - `proposed_content`: the full proposed file content (WP-3's `proposed_content`,
///   set only for a full Write).
/// - `is_full_write`: WP-3's tool-gated classification (Write = full; Edit/MultiEdit
///   = partial → this is `false`).
/// - `cfg`: grammar path + store roots.
pub fn check_write(
    store: &Path,
    target_path: &Path,
    proposed_content: &str,
    is_full_write: bool,
    cfg: &GuardConfig,
) -> GuardVerdict {
    // The grammar file (A6) is checked FIRST — it is underscore-prefixed, so it
    // must be handled before the infra exemption would otherwise skip it.
    if is_grammar_target(target_path, &cfg.grammar_path) {
        if !is_full_write {
            return GuardVerdict::Allow; // partial grammar edit fails open (A6)
        }
        return grammar_write_guard(proposed_content, &cfg.grammar_path);
    }

    // Infra files are exempt before any placement/memory logic (§3): they are never
    // guardable memories.
    if is_infra_basename(target_path) {
        return GuardVerdict::Allow;
    }

    // A partial edit always fails open — the guard cannot judge a diff (§6).
    if !is_full_write {
        return GuardVerdict::Allow;
    }

    // Frontmatter-less content is not a memory → fail open. "Frontmatter-bearing"
    // means the `---` fences are present; a fenced-but-invalid block IS a memory
    // write, which tier 1 then denies.
    if frontmatter::frontmatter_block(proposed_content).is_err() {
        return GuardVerdict::Allow;
    }

    memory_write_guard(store, target_path, proposed_content, cfg)
}

/// The memory-write tiers (§6, in order). The caller has confirmed a full-file write
/// of a frontmatter-bearing, non-grammar, non-infra target.
fn memory_write_guard(
    store: &Path,
    target_path: &Path,
    content: &str,
    cfg: &GuardConfig,
) -> GuardVerdict {
    // Tier 1: shape / evidence (RB3). Reuse the WP-1 parser; do not re-implement.
    let fm = match frontmatter::parse(content) {
        Ok(fm) => fm,
        Err(e) => return GuardVerdict::Deny(DenyReason::ShapeEvidence(e)),
    };
    let triggers = fm.metadata.triggers.clone().unwrap_or_default();
    // Resolve the target the SAME way the grammar/placement checks do (engine-
    // realpath: expand `~`, resolve symlinks on the existing prefix). Deriving
    // `is_new` from the RAW path would see a CONSOLIDATION addressed by a
    // non-canonical path (`~/store/x.md`, or a cwd-relative path) as `.exists()
    // == false` → a "new file" the new-file-only dedup + collision tiers could
    // BLOCK-deny — a false deny of a legitimate write (§6/§7 consolidation
    // exemption; the #1 failure mode).
    let resolved = engine_realpath(target_path);
    let is_new = !resolved.exists();
    let target_id = memory_id_of(&resolved);

    // Load the artifact pair through the single reader (fail open on any fault).
    let read = read_artifacts(&index_path(store), &report_path(store));
    let loaded = read.loaded();

    // Tier 2: static degenerate gate — only with a loaded index (catalog vocab
    // present). No index → fail open (can't judge arg/synonym liveness).
    if let Some(l) = loaded
        && let Some(arm) = static_gate(&triggers, &l.index)
    {
        return GuardVerdict::Deny(DenyReason::StaticDegenerate(arm));
    }

    // Tier 3: dedup backstop — NEW files only; consolidation always passes.
    if is_new
        && let Some(l) = loaded
        && let Some(dup) = dedup_deny(&fm, &l.report.memories, target_id.as_deref())
    {
        return GuardVerdict::Deny(dup);
    }

    // Tier 4: collision tier — NEW files only; any projection fault fails open. The
    // projection reuses the ONE walk (§7 / D4). GUIDE-broad is advisory, not a deny.
    if is_new && let Some(l) = loaded {
        let proj = project(&triggers, &l.index);
        if proj.verdict == Verdict::BlockDegenerate {
            return GuardVerdict::Deny(DenyReason::BlockDegenerate {
                distinct_count: proj.distinct_count,
                floor: COLLISION_GUIDE_FLOOR,
            });
        }
    }

    // Tier 5: high-confidence misplacement.
    if let Some(correct_box_path) = misplacement_box_path(&fm, target_path, cfg) {
        return GuardVerdict::Deny(DenyReason::Misplacement { correct_box_path });
    }

    GuardVerdict::Allow
}

// =============================================================================
// Tier 2: the static degenerate gate (§6) — shared §1 live-lever definition
// =============================================================================

/// Whether the static degenerate gate would deny `triggers` against `index`
/// (`true` = deny). The §11 drift guardrail ([`crate::rebuild`]) reuses this so the
/// advisory and the write-time gate agree. Requires a present index (the caller
/// only invokes it when the catalog vocab is available).
pub fn static_gate_denies(triggers: &Triggers, index: &Index) -> bool {
    static_gate(triggers, index).is_some()
}

/// The static degenerate gate (§6 tier 2), corpus-free but for the byArg/bySynonym
/// membership used to judge arg/synonym liveness. Two mutually-exclusive arms; any
/// live lever (routable arg/synonym or specific path) rescues both.
fn static_gate(triggers: &Triggers, index: &Index) -> Option<StaticArm> {
    let nonblank = |v: &[String]| -> Vec<String> {
        v.iter().filter(|s| !s.trim().is_empty()).cloned().collect()
    };
    let cmds = nonblank(&triggers.commands);
    let paths = nonblank(&triggers.paths);

    // §1 live-lever rescue — the SHARED definition ([`live_levers`]), the SAME one
    // the collision verdict (tier 4) reads. Deriving the rescue from it (rather
    // than re-implementing arg/synonym/path membership inline) is what makes §1's
    // "one definition, both tiers can never disagree" true at the fail-closed
    // boundary — a one-sided edit to liveness now moves BOTH tiers together.
    let any_lever = !live_levers(triggers, index).is_empty();

    // Arm (a): commands present, all generic/low-signal, and no lever rescues.
    if !cmds.is_empty() && !any_lever {
        let all_low_signal = cmds.iter().all(|c| is_generic_command(c));
        if all_low_signal {
            return Some(StaticArm::GenericCommands(cmds));
        }
    }
    // Arm (b): NO commands, only broad paths, and no lever rescues.
    if cmds.is_empty() && !paths.is_empty() && !any_lever {
        // `!any_lever` already implies no specific path, so every path is broad.
        return Some(StaticArm::BroadPaths(paths));
    }
    None
}

/// Whether a command basename is generic/low-signal (§6 tier 2 arm a). Normalized
/// the read path's way (strip + lowercase) before membership — so `Restart` and
/// ` restart ` are both caught — against the shared [`GENERIC_VERBS`] stop-list.
fn is_generic_command(cmd: &str) -> bool {
    let norm = cmd.trim().to_lowercase();
    GENERIC_VERBS.contains(&norm.as_str())
}

// =============================================================================
// Tier 3: dedup backstop (§6) — 0.6·tag_overlap + 0.4·bow-cosine
// =============================================================================

/// The dedup deny check: the best similarity of the proposed memory against every
/// existing memory (excluding itself). Denies iff `>= 0.85`.
fn dedup_deny(
    fm: &Frontmatter,
    memories: &[MemorySummary],
    target_id: Option<&str>,
) -> Option<DenyReason> {
    let best = best_dedup_candidate(fm, memories, target_id)?;
    if best.score >= DEDUP_BACKSTOP_THRESHOLD {
        Some(DenyReason::Duplicate {
            existing_id: best.id,
            existing_path: best.path,
            score: best.score,
        })
    } else {
        None
    }
}

/// One scored dedup candidate.
#[derive(Debug, Clone)]
struct ScoredCandidate {
    id: String,
    path: String,
    score: f64,
}

/// Score the proposed memory against every existing memory (excluding itself),
/// returning the single best candidate.
fn best_dedup_candidate(
    fm: &Frontmatter,
    memories: &[MemorySummary],
    target_id: Option<&str>,
) -> Option<ScoredCandidate> {
    let prop_tags: BTreeSet<String> = fm.metadata.tags.iter().cloned().collect();
    let prop_words = bag_of_words(fm.description.as_deref().unwrap_or(""));
    memories
        .iter()
        .filter(|m| Some(m.id.as_str()) != target_id)
        .map(|m| ScoredCandidate {
            id: m.id.clone(),
            path: m.path.clone(),
            score: dedup_score(&prop_tags, &prop_words, m),
        })
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// The similarity score `0.6·tag_overlap + 0.4·bow-cosine` (§6 / §10). `tag_overlap`
/// is Jaccard `|∩| / max(|∪|, 1)`; `bow-cosine` is a stdlib-`Counter`-shaped cosine
/// of the descriptions with [`DEDUP_STOPWORDS`] removed.
fn dedup_score(
    prop_tags: &BTreeSet<String>,
    prop_words: &BTreeMap<String, u64>,
    mem: &MemorySummary,
) -> f64 {
    let mem_tags: BTreeSet<String> = mem.tags.iter().cloned().collect();
    let inter = prop_tags.intersection(&mem_tags).count();
    let union = prop_tags.union(&mem_tags).count().max(1);
    let tag_overlap = inter as f64 / union as f64;

    let mem_words = bag_of_words(&mem.description);
    let cos = bow_cosine(prop_words, &mem_words);

    0.6 * tag_overlap + 0.4 * cos
}

/// Bag-of-words cosine over two `Counter`-shaped word-count maps: intersection is
/// `sum(min(a_i, b_i))` over shared words; denominator is
/// `sqrt(sum(a_i²)·sum(b_i²))`; a zero denominator yields `0.0` (parity with the
/// stdlib `Counter` intersection the spec names).
fn bow_cosine(a: &BTreeMap<String, u64>, b: &BTreeMap<String, u64>) -> f64 {
    let inter: u64 = a
        .iter()
        .filter_map(|(k, va)| b.get(k).map(|vb| (*va).min(*vb)))
        .sum();
    let na: u64 = a.values().map(|v| v * v).sum();
    let nb: u64 = b.values().map(|v| v * v).sum();
    let denom_sq = (na as f64) * (nb as f64);
    if denom_sq > 0.0 {
        inter as f64 / denom_sq.sqrt()
    } else {
        0.0
    }
}

/// Lowercased whitespace-split bag of words with [`DEDUP_STOPWORDS`] removed (a
/// multiset: repeated words count). Parity with synapse `_dedup_words`.
fn bag_of_words(text: &str) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for w in text.to_lowercase().split_whitespace() {
        if !dedup_stopwords().contains(w) {
            *counts.entry(w.to_string()).or_insert(0u64) += 1;
        }
    }
    counts
}

/// `DEDUP_STOPWORDS` (§10, hardcoded): English function words + the store-domain
/// noise words (`box`, `memory`, `memories`, `note`, `notes`, `lesson`, `lessons`)
/// that appear in nearly every description and carry no subject signal — without
/// them, single-tag overlap plus pure function-word cosine pushed DISTINCT memories
/// past 0.85 (WR-02). Carried verbatim from the synapse tiebreaker.
const DEDUP_STOPWORDS: &[&str] = &[
    "a", "an", "the", "this", "that", "these", "those", "it", "its", "is", "are", "was", "be",
    "been", "on", "in", "at", "of", "to", "for", "with", "from", "by", "via", "and", "or", "as",
    "how", "what", "when", "where", "why", "which", "use", "using", "used", "about", "into", "vs",
    // store-domain noise (load-bearing, §10):
    "box", "memory", "memories", "note", "notes", "lesson", "lessons",
];

fn dedup_stopwords() -> &'static BTreeSet<&'static str> {
    static SET: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| DEDUP_STOPWORDS.iter().copied().collect())
}

// =============================================================================
// Tier 5: high-confidence misplacement (§6 / §3 placement model)
// =============================================================================

/// The correct box path if the memory is high-confidence misplaced, else `None`
/// (fail open). Denies ONLY when the memory has ≥1 grammar-known tag, ALL grammar-
/// known tags carry `placement: box`, a box root is configured, and the target does
/// not resolve under it. Unknown/mixed/`project`/`either` placement fails open.
fn misplacement_box_path(fm: &Frontmatter, target: &Path, cfg: &GuardConfig) -> Option<String> {
    let box_root = cfg.roots.box_root.as_ref()?; // no box root wired → fail open
    // A valid grammar is required to trust placements; an invalid one fails open.
    let grammar_text = std::fs::read_to_string(&cfg.grammar_path).ok()?;
    let grammar = grammar::parse_and_validate(&grammar_text).ok()?;

    let known: Vec<Placement> = fm
        .metadata
        .tags
        .iter()
        .filter_map(|t| grammar_placement(&grammar, t))
        .collect();
    if known.is_empty() {
        return None; // all tags unknown to the grammar → fail open
    }
    if !known.iter().all(|p| *p == Placement::Box) {
        return None; // mixed / project / either → fail open
    }
    // Every grammar-known tag is box-placement. Correctly placed under the box store?
    if target_under(target, box_root) {
        return None;
    }
    // Misplaced: self-heal by naming the correct box path.
    let basename = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "memory-filename.md".to_string());
    Some(format!("{}/{}", box_root.display(), basename))
}

/// A tag's placement hint from the grammar (searching all three facets), or `None`
/// if the tag is unknown.
fn grammar_placement(g: &Grammar, tag: &str) -> Option<Placement> {
    g.domain
        .get(tag)
        .or_else(|| g.tool.get(tag))
        .or_else(|| g.pattern.get(tag))
        .map(|e| e.placement)
}

/// Whether `target` resolves under `root` (engine-realpath containment, §5.x). The
/// symlink-resolving realpath also blocks `../` escape.
fn target_under(target: &Path, root: &Path) -> bool {
    let t = engine_realpath(target);
    let r = engine_realpath(root);
    t == r || t.starts_with(&r)
}

// =============================================================================
// A6: the diff-aware grammar-write surface
// =============================================================================

/// The A6 grammar-write guard: deny a full-file grammar write iff it introduces a
/// validation-error signature the CURRENT file does not have (diff-aware, sound
/// against an already-broken baseline). File absent = bootstrap = allow.
fn grammar_write_guard(proposed_content: &str, grammar_path: &Path) -> GuardVerdict {
    // File absent (or unreadable) = bootstrap = allow (A6).
    let Ok(current_text) = std::fs::read_to_string(grammar_path) else {
        return GuardVerdict::Allow;
    };
    let current = grammar::error_signatures(&current_text);
    // An UNPARSEABLE current grammar has no enumerable baseline — `error_signatures`
    // collapses a TOML parse failure to the single coarse `parse` signature, which
    // cannot be soundly diffed against granular proposed errors (a write that FIXES
    // the syntax but still carries a validation error would look like it
    // "introduced" that error). Treat an unparseable baseline like an ABSENT one:
    // allow (fail-open; never deny a strictly-better, now-parseable write).
    if current.len() == 1 && current.contains("parse") {
        return GuardVerdict::Allow;
    }
    let proposed = grammar::error_signatures(proposed_content);
    let new_errors: Vec<String> = proposed.difference(&current).cloned().collect();
    if new_errors.is_empty() {
        GuardVerdict::Allow
    } else {
        GuardVerdict::Deny(DenyReason::GrammarWrite { new_errors })
    }
}

// =============================================================================
// The write-context payload (§6) — WP-5's write-context branch injects this
// =============================================================================

/// One dedup candidate surfaced in the write context (above the 0.2 floor).
#[derive(Debug, Clone, PartialEq)]
pub struct DedupCandidate {
    /// The candidate memory's id.
    pub id: String,
    /// The candidate memory's path.
    pub path: String,
    /// The similarity score.
    pub score: f64,
}

/// The write-context payload (§6): the schema + worked example, the grammar vocab
/// digest (or the one-line-per-tag fallback when over budget), the top-N dedup
/// candidates, and placement guidance. WP-5's write-context branch injects
/// [`WriteContext::text`] as `additionalContext`.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteContext {
    /// The assembled payload text, kept under [`WRITE_CONTEXT_BUDGET`].
    pub text: String,
    /// The dedup candidates above the 0.2 display floor (top-N).
    pub dedup_candidates: Vec<DedupCandidate>,
    /// Placement guidance, if the memory's tags resolve to a placement.
    pub placement_guidance: Option<String>,
    /// Whether the one-line-per-tag digest fallback was used (over budget).
    pub used_digest_fallback: bool,
}

/// The trigger schema + a worked example (§6). Static template head of the payload.
const TRIGGER_SCHEMA_HINT: &str = "\
Memory triggers route on OBSERVED BEHAVIOR (commands run, paths touched, arg tokens),
never on prompt text. Declare them under `metadata.triggers` (NEVER at the top level):

  metadata:
    tags: [gpu-tools]            # >=1 kebab-case tag, required
    triggers:
      commands: [nvidia-smi]     # strong
      paths: [\"~/.config/gpu/**\"] # strong (a SPECIFIC path narrows; a broad glob does not)
      args: [--no-cache]         # medium (routable via the grammar 'args:' vocab)
      synonyms: [vram]           # weak (routable via the grammar 'synonyms:' vocab)
";

/// Build the write-context payload (§6). Best-effort and fail-open: a parse failure
/// or a missing index yields a still-useful payload (schema + digest).
pub fn write_context(
    store: &Path,
    target_path: &Path,
    proposed_content: &str,
    cfg: &GuardConfig,
) -> WriteContext {
    let fm = frontmatter::parse(proposed_content).ok();
    let read = read_artifacts(&index_path(store), &report_path(store));

    // Top-N dedup candidates above the 0.2 floor (advisory display only).
    let mut dedup_candidates: Vec<DedupCandidate> = Vec::new();
    if let (Some(fmv), Some(l)) = (&fm, read.loaded()) {
        let prop_tags: BTreeSet<String> = fmv.metadata.tags.iter().cloned().collect();
        let prop_words = bag_of_words(fmv.description.as_deref().unwrap_or(""));
        let target_id = memory_id_of(target_path);
        let mut scored: Vec<DedupCandidate> = l
            .report
            .memories
            .iter()
            .filter(|m| Some(m.id.as_str()) != target_id.as_deref())
            .map(|m| DedupCandidate {
                id: m.id.clone(),
                path: m.path.clone(),
                score: dedup_score(&prop_tags, &prop_words, m),
            })
            .filter(|c| c.score >= DEDUP_CANDIDATE_FLOOR)
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(DEDUP_TOP_N);
        dedup_candidates = scored;
    }

    // Placement guidance from the memory's tags.
    let placement_guidance = fm.as_ref().and_then(|fmv| placement_guidance(fmv, cfg));

    // Grammar vocab digest (full), with a one-line-per-tag fallback if over budget.
    let grammar = std::fs::read_to_string(&cfg.grammar_path)
        .ok()
        .and_then(|t| grammar::parse_and_validate(&t).ok());

    let mut used_digest_fallback = false;
    let base = assemble_context(
        &digest_of(&grammar, false),
        &dedup_candidates,
        placement_guidance.as_deref(),
    );
    let text = if base.chars().count() <= WRITE_CONTEXT_BUDGET {
        base
    } else {
        used_digest_fallback = true;
        assemble_context(
            &digest_of(&grammar, true),
            &dedup_candidates,
            placement_guidance.as_deref(),
        )
    };

    WriteContext {
        text,
        dedup_candidates,
        placement_guidance,
        used_digest_fallback,
    }
}

/// Render the grammar digest for the write context: the full [`render_digest`], or
/// the one-line-per-tag fallback (`facet/tag [placement]: gloss`) when `fallback`.
fn digest_of(grammar: &Option<Grammar>, fallback: bool) -> String {
    let Some(g) = grammar else {
        return String::new();
    };
    if !fallback {
        return render_digest(g);
    }
    // One-line-per-tag fallback (no evidence lines).
    let mut out = format!(
        "# Trigger vocabulary — grammar-version {} (condensed)\n",
        g.grammar_version
    );
    for (facet, map) in [
        ("domain", &g.domain),
        ("tool", &g.tool),
        ("pattern", &g.pattern),
    ] {
        for (tag, entry) in map {
            out.push_str(&format!(
                "{facet}/{tag} [{}]: {}\n",
                entry.placement,
                entry.gloss.trim()
            ));
        }
    }
    out
}

/// Placement guidance for the memory's tags (advisory, not a deny). Names the
/// dominant placement hint of the grammar-known tags.
fn placement_guidance(fm: &Frontmatter, cfg: &GuardConfig) -> Option<String> {
    let grammar_text = std::fs::read_to_string(&cfg.grammar_path).ok()?;
    let grammar = grammar::parse_and_validate(&grammar_text).ok()?;
    let placements: Vec<Placement> = fm
        .metadata
        .tags
        .iter()
        .filter_map(|t| grammar_placement(&grammar, t))
        .collect();
    if placements.is_empty() {
        return None;
    }
    if placements.iter().all(|p| *p == Placement::Box) {
        Some("Placement: all tags are box-placement — write to the box-brain store.".to_string())
    } else if placements.iter().all(|p| *p == Placement::Project) {
        Some("Placement: all tags are project-placement — write to the project store.".to_string())
    } else {
        Some("Placement: mixed/either — either store is acceptable for these tags.".to_string())
    }
}

/// Assemble the write-context text from its parts.
fn assemble_context(
    digest: &str,
    candidates: &[DedupCandidate],
    placement: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(TRIGGER_SCHEMA_HINT);
    if !digest.is_empty() {
        out.push('\n');
        out.push_str(digest);
    }
    if !candidates.is_empty() {
        out.push_str("\nSimilar existing memories (consolidate rather than duplicate):\n");
        for c in candidates {
            out.push_str(&format!(
                "  - {} ({}) — similarity {:.2}\n",
                c.id, c.path, c.score
            ));
        }
    }
    if let Some(p) = placement {
        out.push('\n');
        out.push_str(p);
        out.push('\n');
    }
    out
}

// =============================================================================
// Target classification helpers
// =============================================================================

/// The memory id (file stem) of a target path.
fn memory_id_of(target: &Path) -> Option<String> {
    target.file_stem().map(|s| s.to_string_lossy().into_owned())
}

/// Whether the target basename is an infra file (§3): underscore-prefixed or
/// `MEMORY.md`. Infra files are exempt from the memory guard before any tier.
fn is_infra_basename(target: &Path) -> bool {
    match target.file_name().and_then(|n| n.to_str()) {
        Some(name) => name.starts_with('_') || name == "MEMORY.md",
        None => false,
    }
}

/// Whether the write target IS the configured grammar file (A6). Compared via
/// engine-realpath so a store-side symlink to the lab grammar resolves to the same
/// file — and so a new (not-yet-existing) grammar path still matches on identity.
fn is_grammar_target(target: &Path, grammar_path: &Path) -> bool {
    engine_realpath(target) == engine_realpath(grammar_path)
}

/// Engine-realpath (§5.x): expand a leading `~`, resolve symlinks on the deepest
/// EXISTING ancestor, and append the non-existent tail — so a new file (whose
/// parent exists) still canonicalizes. This is the symlink-resolving canonicalizer
/// placement uses (distinct from the adapter-lexical one in [`crate::normalize`]).
fn engine_realpath(p: &Path) -> PathBuf {
    let expanded = expand_home_path(p);
    let mut cur: &Path = &expanded;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canon) = std::fs::canonicalize(cur) {
            let mut out = canon;
            for seg in tail.iter().rev() {
                out.push(seg);
            }
            return out;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = parent;
            }
            // Nothing along the chain exists (or no filename): best-effort lexical.
            _ => return expanded,
        }
    }
}

/// Expand a leading `~` / `~/` to `$HOME`. `HOME` unset → the tilde is left literal.
fn expand_home_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    let Some(home) = std::env::var_os("HOME") else {
        return p.to_path_buf();
    };
    if s == "~" {
        PathBuf::from(home)
    } else if let Some(rest) = s.strip_prefix("~/") {
        PathBuf::from(home).join(rest)
    } else {
        p.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier::{Axis, Source};

    fn triggers(commands: &[&str], paths: &[&str], args: &[&str], synonyms: &[&str]) -> Triggers {
        Triggers {
            commands: commands.iter().map(|s| s.to_string()).collect(),
            paths: paths.iter().map(|s| s.to_string()).collect(),
            args: args.iter().map(|s| s.to_string()).collect(),
            synonyms: synonyms.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn rec(axis: Axis, key: &str) -> crate::index::IndexRecord {
        crate::index::IndexRecord {
            axis,
            pattern: key.to_string(),
            route_tag: "m".into(),
            source: Source::Memory,
            memory_id: "m".into(),
            mem_type: String::new(),
            last_reviewed: String::new(),
            decline_count: 0,
            tags: vec!["t".into()],
            path: "/s/m.md".into(),
            snippet: "d".into(),
        }
    }

    fn arg_index(key: &str) -> Index {
        Index::from_records(vec![rec(Axis::Arg, key)])
    }

    /// An index with an arg key (`release`, byArg only) AND a synonym key (`grep`,
    /// bySynonym only) — so the byArg-vs-bySynonym distinction is observable.
    fn vocab_index() -> Index {
        Index::from_records(vec![rec(Axis::Arg, "release"), rec(Axis::Synonym, "grep")])
    }

    #[test]
    fn static_gate_arms_and_rescues() {
        let idx = arg_index("release");
        // Arm A: only a generic command → deny.
        assert!(matches!(
            static_gate(&triggers(&["restart"], &[], &[], &[]), &idx),
            Some(StaticArm::GenericCommands(_))
        ));
        // Arm B: only a broad path → deny.
        assert!(matches!(
            static_gate(&triggers(&[], &["/**"], &[], &[]), &idx),
            Some(StaticArm::BroadPaths(_))
        ));
        // Rescue: a routable arg allows even with a generic command.
        assert!(static_gate(&triggers(&["restart"], &[], &["release"], &[]), &idx).is_none());
        // Rescue: a specific path allows.
        assert!(static_gate(&triggers(&["restart"], &["/etc/foo.conf"], &[], &[]), &idx).is_none());
        // A specific (non-generic) command is not degenerate (arm A needs all-generic).
        assert!(static_gate(&triggers(&["nvidia-smi"], &["/**"], &[], &[]), &idx).is_none());
        // An empty trigger set is never denied (routes via tags).
        assert!(static_gate(&triggers(&[], &[], &[], &[]), &idx).is_none());
    }

    /// LOCK (§1 unification): the static gate's RESCUE is the shared
    /// [`live_levers`] emptiness — for a degenerate-shaped set (generic command
    /// or broad-path base), the gate denies IFF `live_levers` is empty. This pins
    /// the "one definition, both tiers can never disagree" contract: if a future
    /// edit re-inlines a divergent liveness rule in `static_gate` (e.g. forgets
    /// the byArg-OR-bySynonym arg route), the `grep`-as-arg row below trips it.
    #[test]
    fn static_gate_rescue_equals_shared_live_levers() {
        let idx = vocab_index(); // byArg{release}, bySynonym{grep}
        // Each row is a DEGENERATE-shaped base (generic command / broad path only)
        // plus a candidate lever; the deny decision must track live_levers exactly.
        let matrix = [
            triggers(&["restart"], &[], &["release"], &[]), // arg lever (byArg)
            triggers(&["restart"], &[], &["grep"], &[]),    // arg lever via bySynonym
            triggers(&["restart"], &[], &[], &["grep"]),    // synonym lever
            triggers(&["restart"], &["/etc/x.conf"], &[], &[]), // specific-path lever
            triggers(&["restart"], &["/**"], &[], &[]),     // broad path → no lever
            triggers(&["restart"], &[], &["nope"], &[]),    // dead arg → no lever
            triggers(&["restart"], &[], &[], &[]),          // bare generic → no lever
            triggers(&[], &["/**"], &[], &[]),              // bare broad path → no lever
            triggers(&[], &["/etc/x.conf"], &[], &[]),      // specific path only → lever
        ];
        for t in &matrix {
            let denied = static_gate(t, &idx).is_some();
            let no_lever = live_levers(t, &idx).is_empty();
            assert_eq!(
                denied, no_lever,
                "static gate rescue must equal !live_levers.is_empty() for {t:?}"
            );
        }
    }

    #[test]
    fn bow_cosine_and_dedup_score_bounds() {
        // Identical single-word descriptions → cosine 1.0.
        let a = bag_of_words("checkpoint");
        let b = bag_of_words("checkpoint");
        assert!((bow_cosine(&a, &b) - 1.0).abs() < 1e-9);
        // Disjoint → 0.0.
        let c = bag_of_words("unrelated");
        assert_eq!(bow_cosine(&a, &c), 0.0);
        // Stopwords are dropped (an all-stopword description → empty bag → 0 cosine).
        assert!(bag_of_words("the a of on this box memory").is_empty());
    }

    #[test]
    fn infra_and_grammar_target_detection() {
        assert!(is_infra_basename(Path::new("/s/_grammar.toml")));
        assert!(is_infra_basename(Path::new("/s/MEMORY.md")));
        assert!(!is_infra_basename(Path::new("/s/gpu.md")));
    }
}
