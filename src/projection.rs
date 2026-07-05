//! Collision projection and the unified narrowing / live-lever definition
//! (plan P10; D8, D4, A2, §1, §7).
//!
//! Projection answers "if this proposed trigger set were written, how many OTHER
//! memories would it co-fire with, and does it declare any lever that actually
//! narrows recall?" — so the write path (§6) can judge over-breadth.
//!
//! ## One walk, no second matcher (D4 / N1 / RB9)
//!
//! [`project`] walks the SAME [`crate::index::Index::walk`] recall uses — UNGATED
//! and UNSCORED. There is no second matcher: the co-fire set is exactly the walk's
//! raw hits, deduped to distinct memory ids. [`walk_query_for`] is the one place a
//! proposed trigger set becomes a [`WalkQuery`], so recall and projection reach the
//! index identically.
//!
//! ## Liveness = routability, defined ONCE (§1)
//!
//! [`live_levers`] is the single narrowing/live-lever definition §1 promises,
//! referenced by BOTH the write-guard static gate ([`crate::guard`]) and the
//! collision verdict here — so the two tiers can never disagree. A lever is LIVE
//! iff it would route inside the walk, by **index-key membership** (never co-fire
//! counts — the retired signal-inversion, Appendix A of CORE-SPEC):
//!
//! - **arg** → live iff its normalized form is a key in `byArg` **OR** `bySynonym`.
//! - **path** → live iff it is **specific** ([`crate::path_class::is_broad_path`]
//!   is `false`); needs no catalog membership.
//! - **synonym** → live iff its normalized form is a key in `bySynonym`.
//!
//! Commands are not levers (§1): a command is strong evidence but not a *narrowing*
//! lever, so it never appears in `live_levers`.

use std::collections::{BTreeMap, BTreeSet};

use crate::frontmatter::Triggers;
use crate::index::{Index, WalkQuery};
use crate::path_class::is_broad_path;

/// `collisionGuideFloor` (§10, default 8). The single corpus-breadth cutoff: the
/// SAME floor gates the BLOCK-degenerate deny and the GUIDE-broad advisory (§7).
/// Carried as a const here; WP-7 (P15) lifts it into `Config` as a §10 tunable.
/// **Asserted, not derived** (§10) — there is no per-corpus cutoff (N6).
pub const COLLISION_GUIDE_FLOOR: usize = 8;

// =============================================================================
// Live levers (§1) — the shared narrowing definition
// =============================================================================

/// The declared levers of a proposed trigger set that would actually **route** the
/// memory — the §1 live-lever set, computed by index-key membership independently
/// of the co-fire walk. Carries the raw lever strings (for guidance) split by axis.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveLevers {
    /// Proposed args that route (`byArg` OR `bySynonym`).
    pub args: Vec<String>,
    /// Proposed paths that are specific (not broad, §3.x).
    pub paths: Vec<String>,
    /// Proposed synonyms that route (`bySynonym`).
    pub synonyms: Vec<String>,
}

impl LiveLevers {
    /// Whether the proposed set declares **no** live lever. An empty live-lever set
    /// is the load-bearing half of the BLOCK-degenerate verdict (§7).
    pub fn is_empty(&self) -> bool {
        self.args.is_empty() && self.paths.is_empty() && self.synonyms.is_empty()
    }

    /// Every live lever string (args, then paths, then synonyms — the §7 order).
    pub fn all(&self) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(self.args.iter().cloned());
        out.extend(self.paths.iter().cloned());
        out.extend(self.synonyms.iter().cloned());
        out
    }
}

/// Compute the §1 live levers of `triggers` against `index` by index-key
/// membership (never hit counts). This ONE function drives both the static gate
/// (§6) and the collision verdict (§7). Blank tokens never count.
pub fn live_levers(triggers: &Triggers, index: &Index) -> LiveLevers {
    let nonblank = |s: &&String| !s.trim().is_empty();
    LiveLevers {
        // An arg routes via byArg OR bySynonym (mirrors the walk's dual arg route).
        args: triggers
            .args
            .iter()
            .filter(nonblank)
            .filter(|a| index.contains_arg_key(a) || index.contains_synonym_key(a))
            .cloned()
            .collect(),
        // A specific path always narrows — no catalog membership needed (§7).
        paths: triggers
            .paths
            .iter()
            .filter(nonblank)
            .filter(|p| !is_broad_path(p))
            .cloned()
            .collect(),
        // A synonym routes via bySynonym only.
        synonyms: triggers
            .synonyms
            .iter()
            .filter(nonblank)
            .filter(|s| index.contains_synonym_key(s))
            .cloned()
            .collect(),
    }
}

// =============================================================================
// The projection
// =============================================================================

/// The write-time collision verdict (§7). A pure read of the projection; because
/// the live-lever and static-gate definitions are unified (§1), the two tiers can
/// never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// `distinct_count <= collisionGuideFloor` — under the floor, silent.
    Pass,
    /// Breadth above the floor but a declared lever IS live. **Advisory** — never a
    /// hard block (a deliberately broad but routable memory).
    GuideBroad,
    /// Breadth **strictly greater than** the floor AND `live_levers` is empty — the
    /// degenerate case, the ONE collision verdict that hard-denies (§6 tier 4).
    BlockDegenerate,
}

/// The collision projection (§7 fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    /// The distinct co-firing memory ids, sorted — the OTHER memories the set matches.
    pub collisions: Vec<String>,
    /// Per-axis contribution: `trigger_type` → distinct co-firing memory count.
    /// **Not a sum** (§7) — the axis-resolved breadth.
    pub per_trigger: BTreeMap<String, usize>,
    /// Co-fire breadth (`= collisions.len()`) — how many other memories the set matches.
    pub distinct_count: usize,
    /// The §1 live levers of the PROPOSED set (computed independently of the co-fire
    /// walk). This is the load-bearing field the verdict reads.
    pub live_levers: LiveLevers,
    /// The verdict (PASS / GUIDE-broad / BLOCK-degenerate).
    pub verdict: Verdict,
}

impl Projection {
    /// The empty projection carrying `live_levers` (§7 fail-open). Any projection
    /// fault yields this, and the write then proceeds under the static gate only.
    pub fn empty(live_levers: LiveLevers) -> Projection {
        Projection {
            collisions: Vec::new(),
            per_trigger: BTreeMap::new(),
            distinct_count: 0,
            live_levers,
            verdict: Verdict::Pass,
        }
    }
}

/// Build the [`WalkQuery`] for a proposed trigger set — the ONE place a trigger set
/// becomes a walk query, so projection reaches the index exactly as recall does
/// (RB9). Args route via `byArg` OR `bySynonym` (so a proposed arg is queried in
/// both), synonyms via `bySynonym` only; paths get their home anchor expanded so
/// they can match the index's expanded byPath globs; commands go straight through
/// (the walk normalizes every exact-key token itself).
pub fn walk_query_for(triggers: &Triggers) -> WalkQuery {
    let mut synonyms = triggers.synonyms.clone();
    // A proposed arg co-fires via byArg OR bySynonym — query it in both buckets,
    // mirroring the walk's dual arg route (and recall's arg/synonym dual-bucketing).
    synonyms.extend(triggers.args.iter().cloned());
    WalkQuery {
        commands: triggers.commands.clone(),
        paths: triggers.paths.iter().map(|p| expand_home(p)).collect(),
        args: triggers.args.clone(),
        synonyms,
    }
}

/// Project a proposed trigger set against `index` (§7). Walks the ONE
/// [`Index::walk`] ungated/unscored, reports co-fire breadth + per-axis
/// contribution + the §1 live levers, and renders the verdict.
///
/// It is **new-file oriented**: the proposed memory is not in `index`, so every hit
/// is another memory. The caller (§6 collision tier) invokes it only for new files;
/// an existing-file consolidation never reaches here.
pub fn project(triggers: &Triggers, index: &Index) -> Projection {
    let query = walk_query_for(triggers);
    let hits = index.walk(&query);

    // Distinct co-firing memory ids (co-fire breadth) + per-axis contribution.
    let mut collisions: BTreeSet<String> = BTreeSet::new();
    let mut per_axis: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for hit in &hits {
        collisions.insert(hit.record.memory_id.clone());
        per_axis
            .entry(hit.record.trigger_type_str().to_string())
            .or_default()
            .insert(hit.record.memory_id.clone());
    }
    let per_trigger: BTreeMap<String, usize> =
        per_axis.into_iter().map(|(k, v)| (k, v.len())).collect();
    let collisions: Vec<String> = collisions.into_iter().collect();
    let distinct_count = collisions.len();

    let live = live_levers(triggers, index);
    let verdict = verdict_of(distinct_count, &live);

    Projection {
        collisions,
        per_trigger,
        distinct_count,
        live_levers: live,
        verdict,
    }
}

/// The §7 verdict from breadth + live levers. PASS iff `distinct_count <= floor`
/// (so the block/guide branches need breadth **strictly greater than** the floor —
/// the `>` is load-bearing); above the floor, a live lever downgrades BLOCK to the
/// advisory GUIDE-broad.
fn verdict_of(distinct_count: usize, live: &LiveLevers) -> Verdict {
    if distinct_count <= COLLISION_GUIDE_FLOOR {
        Verdict::Pass
    } else if live.is_empty() {
        Verdict::BlockDegenerate
    } else {
        Verdict::GuideBroad
    }
}

/// Expand a leading `~` / `~/` (and `$HOME` / `${HOME}`) to `$HOME` for a proposed
/// path used as a co-fire query. `HOME` unset → the anchor stays literal.
fn expand_home(pat: &str) -> String {
    let home = match std::env::var_os("HOME") {
        Some(h) => h.to_string_lossy().into_owned(),
        None => return pat.to_string(),
    };
    for anchor in ["${HOME}", "$HOME", "~"] {
        if pat == anchor {
            return home;
        }
        if let Some(rest) = pat.strip_prefix(anchor)
            && rest.starts_with('/')
        {
            return format!("{home}{rest}");
        }
    }
    pat.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier::{Axis, Source};

    fn rec(axis: Axis, pattern: &str, mid: &str) -> crate::index::IndexRecord {
        crate::index::IndexRecord {
            axis,
            pattern: pattern.to_string(),
            route_tag: mid.to_string(),
            source: Source::Memory,
            memory_id: mid.to_string(),
            mem_type: String::new(),
            last_reviewed: String::new(),
            decline_count: 0,
            tags: vec!["t".into()],
            path: format!("/store/{mid}.md"),
            snippet: "desc".into(),
        }
    }

    fn triggers(commands: &[&str], paths: &[&str], args: &[&str], synonyms: &[&str]) -> Triggers {
        Triggers {
            commands: commands.iter().map(|s| s.to_string()).collect(),
            paths: paths.iter().map(|s| s.to_string()).collect(),
            args: args.iter().map(|s| s.to_string()).collect(),
            synonyms: synonyms.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// An index whose `byCommand[foo]` routes to `distinct_count` distinct memories.
    fn wide_command_index(cmd: &str, n: usize) -> Index {
        let recs = (0..n)
            .map(|i| rec(Axis::Command, cmd, &format!("m{i}")))
            .collect();
        Index::from_records(recs)
    }

    #[test]
    fn live_levers_by_index_key_membership() {
        let idx = Index::from_records(vec![
            rec(Axis::Arg, "release", "a"),
            rec(Axis::Synonym, "grep", "b"),
        ]);
        // arg live via byArg; synonym live via bySynonym; a broad path never lives;
        // a specific path lives without any catalog membership.
        let live = live_levers(
            &triggers(&[], &["/**", "/etc/foo.conf"], &["release"], &["grep"]),
            &idx,
        );
        assert_eq!(live.args, vec!["release".to_string()]);
        assert_eq!(live.synonyms, vec!["grep".to_string()]);
        assert_eq!(live.paths, vec!["/etc/foo.conf".to_string()]);
        assert!(!live.is_empty());

        // A dead arg + a dead synonym + only broad paths → no live lever.
        let dead = live_levers(&triggers(&[], &["/**"], &["nope"], &["nope"]), &idx);
        assert!(dead.is_empty());
    }

    #[test]
    fn verdict_strict_greater_than_floor() {
        let empty_live = LiveLevers::default();
        let some_live = LiveLevers {
            paths: vec!["/etc/foo.conf".into()],
            ..Default::default()
        };
        // breadth == floor → PASS (strict `>` is load-bearing).
        assert_eq!(
            verdict_of(COLLISION_GUIDE_FLOOR, &empty_live),
            Verdict::Pass
        );
        // floor + 1 with no live lever → BLOCK-degenerate.
        assert_eq!(
            verdict_of(COLLISION_GUIDE_FLOOR + 1, &empty_live),
            Verdict::BlockDegenerate
        );
        // floor + 1 with a live lever → GUIDE-broad (advisory).
        assert_eq!(
            verdict_of(COLLISION_GUIDE_FLOOR + 1, &some_live),
            Verdict::GuideBroad
        );
    }

    #[test]
    fn project_counts_distinct_co_fires_and_renders_verdict() {
        // A generic command keyed to floor+1 memories, no live lever → BLOCK.
        let idx = wide_command_index("restart", COLLISION_GUIDE_FLOOR + 1);
        let p = project(&triggers(&["restart"], &[], &[], &[]), &idx);
        assert_eq!(p.distinct_count, COLLISION_GUIDE_FLOOR + 1);
        assert_eq!(
            p.per_trigger.get("command"),
            Some(&(COLLISION_GUIDE_FLOOR + 1))
        );
        assert!(p.live_levers.is_empty());
        assert_eq!(p.verdict, Verdict::BlockDegenerate);

        // Same breadth, but the set ALSO declares a specific path → the path is a
        // live lever → GUIDE-broad, not a block.
        let p2 = project(
            &triggers(&["restart"], &["/etc/specific.conf"], &[], &[]),
            &idx,
        );
        assert_eq!(p2.verdict, Verdict::GuideBroad);
    }

    #[test]
    fn empty_projection_carries_live_levers() {
        let idx = Index::default();
        let live = live_levers(&triggers(&[], &["/etc/x.conf"], &[], &[]), &idx);
        let p = Projection::empty(live);
        assert_eq!(p.distinct_count, 0);
        assert_eq!(p.verdict, Verdict::Pass);
        assert!(
            !p.live_levers.is_empty(),
            "empty projection still carries levers"
        );
    }
}
