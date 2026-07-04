//! The recall path — the pure engine (plan P6 / WP-3; D1, D3, D5, D19, D25, A7;
//! CORE-SPEC §5, §2.6, §10).
//!
//! `recall` turns a [`NormalizedOp`] into an advisory (or silence) by reading
//! ONLY the flat index through the one WP-2 walk. The hook wiring and the
//! `.surface-disabled` kill-switch are a later packet (WP-5); this module is the
//! engine WP-5 calls.
//!
//! ## The hard read-path invariant (N11 / D1 — index-only recall)
//!
//! `recall` **NEVER rebuilds**, **NEVER loads a memory body**, and **emits nothing
//! on silence**. It reads the flat index + catalog report via
//! [`crate::catalog::read_artifacts`] (the single reader), which fails open on a
//! missing / stale / malformed pair — and on any such fault recall surfaces
//! nothing rather than rebuilding. The only files it opens are the two build
//! artifacts; the `.md` bodies are never touched.
//!
//! ## The pipeline (§5)
//!
//! 1. **Extract** a query from the op: Bash command basenames + content args, tool
//!    target paths, and WebSearch/WebFetch/context7 keyword tokens. Command/arg/
//!    synonym tokens are normalized with [`crate::index::routing_key`] exactly as
//!    the build side keyed the index; paths are lexically canonicalized (§5.x) and
//!    stay raw for the byPath scan.
//! 2. **Load** the index (fail open → silence).
//! 3. **Walk** — the one ungated/unscored matcher ([`crate::index::Index::walk`]).
//! 4. **Gate** per memory (D5, frozen form): fire iff ≥1 strong-tier tuple OR ≥2
//!    tuples total, over distinct `(route_tag, trigger_type)` tuples. The
//!    GENERIC_VERBS stop-list drops generic command HITS *before* the tuple dedup,
//!    so a generic command can never shadow a specific one on the same route_tag.
//! 5. **Score** (§10 magnitudes, frozen forms): tier weights summed over distinct
//!    tuples, minus `5×stale` and `2×min(declineCount, 3)`; confidence vs the
//!    high/medium thresholds.
//! 6. **Dedup** per memory: suppress a candidate whose dedup mark is live (WP-2b
//!    [`Telemetry::is_live`]).
//! 7. **Rank + cap** (`maxResults = 3`), **render** with the §2.6 citation
//!    `{route_tag} <- {trigger_type}:{matched_value}`, and **fire** through
//!    [`Telemetry::log_fire`] — the one fire-logging path (D25/A7).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::catalog::read_artifacts;
use crate::index::{Hit, WalkQuery, routing_key};
use crate::normalize::{NormalizedOp, ToolOp, canonicalize_lexical, tokenize_bash};
use crate::rebuild::{index_path, report_path};
use crate::telemetry::{FireMem, FireOutcome, FireRecord, Telemetry};
use crate::tier::Tier;

// =============================================================================
// §10 tunables — forms frozen (§2), magnitudes are the §10 defaults
// =============================================================================
//
// These live as consts here because WP-2b's `Config` deliberately carries only the
// three marks/telemetry tunables; WP-7 (plan P15) is the packet that lifts
// `tierWeights` / `confidence*Threshold` into `Config` as config-overridable knobs.
// Until then recall uses the frozen §10 defaults directly — the *form* (§2 invariant)
// is what is frozen, and it lives in the gate/score functions below.

/// `TIER_WEIGHTS` (§10): strong / medium / weak.
const TIER_STRONG: i64 = 10;
const TIER_MEDIUM: i64 = 6;
const TIER_WEAK: i64 = 3;
/// Score penalties (§5, hardcoded): `-5×stale`, `-2×min(declineCount, 3)`.
const STALE_PENALTY: i64 = 5;
const DECLINE_PENALTY_PER: i64 = 2;
const DECLINE_CAP: i64 = 3;
/// Confidence thresholds (§10): high ≥ 10, medium ≥ 6.
const CONFIDENCE_HIGH: i64 = 10;
const CONFIDENCE_MEDIUM: i64 = 6;
/// Staleness horizon (§5): `lastReviewed` older than this many days is stale.
const STALE_DAYS: i64 = 180;
/// `maxResults` (§10): at most this many memories surface.
const MAX_RESULTS: usize = 3;

/// The GENERIC_VERBS stop-list (§5, D5): generic command basenames that do NOT
/// count as strong evidence. Source: synapse `memory_surface.py:1702-1704`
/// `GENERIC_VERBS` verbatim (service/pkg-manager subcommand verbs) plus `check`,
/// CORE-SPEC §5's named example ("generic verbs like restart/install/check").
/// A command tuple whose matched basename is in this set is dropped from a
/// memory's candidate tuple set entirely (precision over recall, D5) — so a memory
/// whose ONLY evidence is a generic-verb command never surfaces.
const GENERIC_VERBS: &[&str] = &[
    "restart", "start", "stop", "status", "enable", "disable", "reload", "list", "show", "info",
    "help", "version", "get", "set", "add", "install", "remove", "update", "upgrade", "check",
];

// =============================================================================
// Public result shapes (WP-5 turns an Advisory into `additionalContext`)
// =============================================================================

/// The outcome of [`recall`]. On silence there is NOTHING to render (N11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecallOutcome {
    /// No advisory: no evidence, gate not met, all candidates deduped, or the
    /// index was missing/stale/malformed (fail-open). Emits nothing.
    Silence,
    /// A recall advisory to surface.
    Advisory(Advisory),
}

impl RecallOutcome {
    /// The advisory, if any.
    pub fn advisory(&self) -> Option<&Advisory> {
        match self {
            RecallOutcome::Advisory(a) => Some(a),
            RecallOutcome::Silence => None,
        }
    }

    /// `true` iff this is silence.
    pub fn is_silent(&self) -> bool {
        matches!(self, RecallOutcome::Silence)
    }
}

/// A recall advisory: the surfaced memories, their citations, the rendered text
/// WP-5 wraps into `hookSpecificOutput.additionalContext`, and the fire outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advisory {
    /// The query id (the fire-record discriminator).
    pub query_id: String,
    /// The overall confidence label (the top result's label).
    pub confidence: String,
    /// The surfaced memories, ranked and capped at `maxResults`.
    pub memories: Vec<SurfacedMemory>,
    /// What [`Telemetry::log_fire`] did with the fire (fail-open; ZeroFire when the
    /// mark dir is unwritable, so the advisory still renders).
    pub fire: FireOutcome,
    /// The rendered advisory text.
    pub text: String,
}

/// One surfaced memory with its firing evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfacedMemory {
    /// The memory id.
    pub memory_id: String,
    /// The memory file path (display).
    pub path: String,
    /// The description snippet (already control-sanitized, escaped, and truncated
    /// to `maxDescriptionChars` at build).
    pub snippet: String,
    /// The recall score.
    pub score: i64,
    /// The per-memory confidence label.
    pub confidence: String,
    /// The firing evidence — one citation per distinct `(route_tag, trigger_type)`.
    pub citations: Vec<Citation>,
}

/// A diagnosable-fire citation (§2.6): `{route_tag} <- {trigger_type}:{matched_value}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    /// The grammar route tag (source `t`) or memory id (source `m`) — the populated
    /// `route_tag`/`trigger_type` axis columns, NOT the empty reserved `type` column.
    pub route_tag: String,
    /// The trigger axis (`command` / `path` / `arg` / `synonym`).
    pub trigger_type: String,
    /// The query value that matched.
    pub matched_value: String,
}

impl Citation {
    /// Render as the frozen §2.6 form `{route_tag} <- {trigger_type}:{matched_value}`.
    pub fn render(&self) -> String {
        format!(
            "{} <- {}:{}",
            self.route_tag, self.trigger_type, self.matched_value
        )
    }
}

// =============================================================================
// The recall entry (WP-5 consumes this: NormalizedOp + store → advisory | silence)
// =============================================================================

/// The recall path. Index-only (N11): reads the flat index via the single reader,
/// walks it once, gates + scores + dedups, and — on a fire — renders an advisory
/// and logs it through [`Telemetry::log_fire`]. Returns [`RecallOutcome::Silence`]
/// (nothing rendered, nothing logged) on no-evidence, a not-met gate, a fully
/// deduped candidate set, or any index fault (fail open, NEVER a rebuild).
///
/// `telemetry` is injected (WP-2b) so the dedup window and the fire append use the
/// real primitive; production callers pass `Telemetry::for_store(store, cfg)`.
pub fn recall(op: &NormalizedOp, store_dir: &Path, telemetry: &Telemetry) -> RecallOutcome {
    // 1. Extract the query. No tool op (SessionStart / Unclassifiable) or no
    //    routable token → silence.
    let Some(query) = build_query(op) else {
        return RecallOutcome::Silence;
    };

    // 2. Load the index — the SINGLE reader, fail open. Missing / Stale / Malformed
    //    → surface nothing, NEVER rebuild (N11/D1). `read_artifacts` only ever reads
    //    the two artifact files; it cannot create them.
    let read = read_artifacts(&index_path(store_dir), &report_path(store_dir));
    let Some(loaded) = read.loaded() else {
        return RecallOutcome::Silence;
    };

    // 3. Walk — the ONE matcher (D4). Ungated, unscored.
    let hits = loaded.index.walk(&query.walk);
    if hits.is_empty() {
        return RecallOutcome::Silence;
    }

    // 4 + 5. Gate + score per memory.
    let today = today_days();
    let mut candidates = gate_and_score(&hits, today);
    if candidates.is_empty() {
        return RecallOutcome::Silence;
    }

    // 6. Dedup window: suppress any candidate whose per-memory mark is live (D5/D25).
    candidates.retain(|c| !telemetry.is_live(&c.memory_id));
    if candidates.is_empty() {
        return RecallOutcome::Silence;
    }

    // 7. Rank (score desc, id asc for determinism) + cap at maxResults.
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.memory_id.cmp(&b.memory_id))
    });
    candidates.truncate(MAX_RESULTS);

    let confidence = confidence_label(candidates[0].score);
    let text = render_advisory(&confidence, &candidates);

    // 8. Fire telemetry — the ONE fire-logging path (D25/A7). The primitive derives
    //    the marked/gated set from `record.mems` and gates the append on mark
    //    persistence (ZeroFire when the mark dir is unwritable). Fail-open: recall
    //    proceeds regardless of the outcome.
    let fire_record = build_fire_record(&query.query_id, &confidence, &candidates);
    let fire = telemetry.log_fire(&fire_record);

    RecallOutcome::Advisory(Advisory {
        query_id: query.query_id,
        confidence,
        memories: candidates,
        fire,
        text,
    })
}

// =============================================================================
// 1. Query extraction (§5) — the WalkQuery + a query id
// =============================================================================

/// A built recall query: the [`WalkQuery`] for the one walk + the query id.
struct BuiltQuery {
    walk: WalkQuery,
    query_id: String,
}

/// Extract the query from a normalized op (§5). Returns `None` when there is no
/// tool op or no routable token — both of which mean silence (D3: observable
/// behavior only; no evidence → nothing to surface).
fn build_query(op: &NormalizedOp) -> Option<BuiltQuery> {
    let tool_op = match op {
        NormalizedOp::PreOp(t) | NormalizedOp::PostOp(t) => t,
        // SessionStart / Unclassifiable carry no operation to route on.
        NormalizedOp::SessionStart { .. } | NormalizedOp::Unclassifiable => return None,
    };

    let mut commands: Vec<String> = Vec::new();
    // `content` = the BASH argument tokens. §5's recall extracts "arg tokens"; the
    // index's byArg (medium) and bySynonym (weak) tables both hold that kind of
    // evidence, so — mirroring the synapse matcher, which looks an argument token up
    // in byArg AND bySynonym — recall feeds these to BOTH the `args` and `synonyms`
    // walk buckets. A token that keys a byArg pattern scores medium; one that keys a
    // bySynonym pattern scores weak. Web keywords are handled separately (below) and
    // are bySynonym-only — they never join `content`.
    let mut content: Vec<String> = Vec::new();
    let mut paths: Vec<String> = Vec::new();

    // Bash: command basenames + content args (the ONE tokenizer; the tool name is
    // NOT a command).
    if let Some(cmd) = &tool_op.command_text {
        let toks = tokenize_bash(cmd);
        commands.extend(toks.command_basenames);
        content.extend(toks.arg_tokens);
    }

    // Paths: the tool target + Bash-embedded paths, lexically canonicalized (§5.x)
    // against the op's cwd. Paths stay RAW (not routing_key-normalized) for the
    // byPath scan.
    let cwd = tool_op.cwd.as_deref();
    if let Some(tp) = &tool_op.target_path {
        paths.push(canonicalize_lexical(tp, cwd).to_string_lossy().into_owned());
    }
    for p in &tool_op.bash_embedded_paths {
        paths.push(canonicalize_lexical(p, cwd).to_string_lossy().into_owned());
    }

    // WebSearch / WebFetch / context7: keyword tokens are WEAK, bySynonym-ONLY
    // evidence — the synapse tiebreaker's `kind=="tag"` branch routes them through
    // bySynonym and NEVER byArg (memory_surface.py:2211-2229). Keeping them out of
    // `args`/byArg is what stops one web query from clearing the ≥2-tuple gate
    // (arg+synonym) or firing a Bash-argument-only memory at medium — a false fire.
    // So they go into `synonyms` only, never `args`.
    let web_content = keyword_tokens(tool_op);

    // Normalize command/content tokens with the shared build-side key normalizer,
    // dropping non-routable forms (`--bare`, `-p`, paths, mixed-noise), and dedup.
    let commands = normalize_dedup(&commands);
    let content = normalize_dedup(&content);
    let web_content = normalize_dedup(&web_content);
    let paths = dedup_raw(&paths);

    if commands.is_empty() && content.is_empty() && web_content.is_empty() && paths.is_empty() {
        return None; // no routable evidence → silence
    }

    // `synonyms` bucket = Bash argument tokens (dual-bucket parity with byArg) ∪ the
    // weak-only web keywords; `args` bucket = Bash argument tokens ONLY.
    let mut synonyms = content.clone();
    synonyms.extend(web_content);
    let synonyms = dedup_raw(&synonyms);

    let walk = WalkQuery {
        commands,
        paths,
        args: content,
        synonyms,
    };
    let query_id = compute_query_id(&tool_op.tool_name, &walk);
    Some(BuiltQuery { walk, query_id })
}

/// Extract keyword tokens from WebSearch (`query`), WebFetch (`url`), and the
/// context7 MCP tool (`libraryName` / `context7CompatibleLibraryID` / `libraryId`
/// / `query`). Non-matching tools yield nothing. Tokens are lowercase runs of
/// `[a-z0-9][a-z0-9-]*` (hyphenated tags survive intact).
fn keyword_tokens(op: &ToolOp) -> Vec<String> {
    let ti = &op.raw_tool_input;
    let mut out = Vec::new();
    let mut harvest = |field: &str| {
        if let Some(s) = ti.get(field).and_then(serde_json::Value::as_str) {
            out.extend(keyword_runs(s));
        }
    };
    match op.tool_name.as_str() {
        "WebSearch" => harvest("query"),
        "WebFetch" => harvest("url"),
        name if is_context7(name) => {
            for field in [
                "libraryName",
                "context7CompatibleLibraryID",
                "libraryId",
                "query",
            ] {
                harvest(field);
            }
        }
        _ => {}
    }
    out
}

/// The proven MCP-context7 matcher (Appendix B): an `mcp__…` tool whose name
/// mentions context7. Carried verbatim (R6); generalization is post-v1.
fn is_context7(tool_name: &str) -> bool {
    tool_name.starts_with("mcp__") && tool_name.contains("context7")
}

/// Lowercase `[a-z0-9][a-z0-9-]*` runs of `s` (regex-free), preserving hyphenated
/// tags like `plasma-compositor`.
fn keyword_runs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        let lc = c.to_ascii_lowercase();
        let is_body = lc.is_ascii_lowercase() || lc.is_ascii_digit() || lc == '-';
        if is_body && !(cur.is_empty() && lc == '-') {
            cur.push(lc);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Normalize each token with the shared build-side [`routing_key`], keep only the
/// routable ones, and dedup (order-preserving).
fn normalize_dedup(tokens: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for t in tokens {
        if let Some(k) = routing_key(t)
            && seen.insert(k.clone())
        {
            out.push(k);
        }
    }
    out
}

/// Dedup raw path strings (order-preserving); paths are NOT routing_key-normalized.
fn dedup_raw(paths: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for p in paths {
        if seen.insert(p.clone()) {
            out.push(p.clone());
        }
    }
    out
}

// =============================================================================
// 4 + 5. Gate + score (§5, §10)
// =============================================================================

/// One distinct firing tuple for a memory.
struct FiringTuple {
    route_tag: String,
    trigger_type: String,
    matched_value: String,
    tier: Tier,
}

/// Per-memory accumulation as the walk hits are grouped.
struct MemAccum {
    path: String,
    snippet: String,
    decline_count: i64,
    last_reviewed: String,
    tuples: Vec<FiringTuple>,
    seen: std::collections::BTreeSet<(String, String)>,
}

/// Group the walk hits by memory, apply the surface gate, and score the survivors.
/// Returns the firing memories (unranked).
fn gate_and_score(hits: &[Hit<'_>], today_days: i64) -> Vec<SurfacedMemory> {
    // Grouping preserves the walk's deterministic hit order via insertion order,
    // and `BTreeMap` keeps the memory set itself deterministic.
    let mut by_mem: BTreeMap<String, MemAccum> = BTreeMap::new();
    for hit in hits {
        let r = hit.record;
        // GENERIC_VERBS is a HIT-level filter on command-axis matches, applied
        // BEFORE the (route_tag, trigger_type) dedup. A generic command basename
        // must never become the representative of a (route_tag, command) tuple that
        // a NON-generic command also feeds — otherwise `restart && systemctl status`
        // (restart first) would drop the whole `svc <- command` tuple while
        // `systemctl status && restart` keeps it: identical evidence, opposite
        // outcome (a D5 precision/determinism break). Filtering at the hit level
        // makes a (route_tag, command) tuple survive iff ≥1 NON-generic command
        // matched, order-independently; arg/synonym axes are already non-strong so
        // the stop-list stays command-axis-only.
        if r.trigger_type_str() == "command" && GENERIC_VERBS.contains(&hit.matched_value.as_str())
        {
            continue;
        }
        let acc = by_mem
            .entry(r.memory_id.clone())
            .or_insert_with(|| MemAccum {
                path: r.path.clone(),
                snippet: r.snippet.clone(),
                decline_count: r.decline_count,
                last_reviewed: r.last_reviewed.clone(),
                tuples: Vec::new(),
                seen: std::collections::BTreeSet::new(),
            });
        let key = (r.route_tag.clone(), r.trigger_type_str().to_string());
        if acc.seen.insert(key) {
            acc.tuples.push(FiringTuple {
                route_tag: r.route_tag.clone(),
                trigger_type: r.trigger_type_str().to_string(),
                matched_value: hit.matched_value.clone(),
                tier: r.tier(),
            });
        }
    }

    let mut out = Vec::new();
    for (memory_id, acc) in by_mem {
        if !meets_surface_gate(&acc.tuples) {
            continue;
        }

        let score = score_tuples(
            &acc.tuples,
            acc.decline_count,
            &acc.last_reviewed,
            today_days,
        );
        let citations = acc
            .tuples
            .iter()
            .map(|t| Citation {
                route_tag: t.route_tag.clone(),
                trigger_type: t.trigger_type.clone(),
                matched_value: t.matched_value.clone(),
            })
            .collect();
        out.push(SurfacedMemory {
            memory_id,
            path: acc.path,
            snippet: acc.snippet,
            score,
            confidence: confidence_label(score),
            citations,
        });
    }
    out
}

/// The surface gate (§5, D5, frozen form): fire iff ≥1 strong-tier tuple OR ≥2
/// distinct tuples total. Applied to the post-GENERIC_VERBS tuple set.
fn meets_surface_gate(tuples: &[FiringTuple]) -> bool {
    if tuples.len() >= 2 {
        return true;
    }
    tuples.iter().any(|t| t.tier == Tier::Strong)
}

/// Score a memory (§5/§10): tier weights summed over distinct tuples, minus
/// `5×stale` and `2×min(declineCount, 3)`.
fn score_tuples(
    tuples: &[FiringTuple],
    decline_count: i64,
    last_reviewed: &str,
    today_days: i64,
) -> i64 {
    let mut score: i64 = tuples.iter().map(|t| tier_weight(t.tier)).sum();
    if is_stale(last_reviewed, today_days) {
        score -= STALE_PENALTY;
    }
    let decline = decline_count.clamp(0, DECLINE_CAP);
    score -= DECLINE_PENALTY_PER * decline;
    score
}

/// The §10 tier weight for a tier.
fn tier_weight(tier: Tier) -> i64 {
    match tier {
        Tier::Strong => TIER_STRONG,
        Tier::Medium => TIER_MEDIUM,
        Tier::Weak => TIER_WEAK,
    }
}

/// Map a score to a confidence label (§10): high ≥ 10, medium ≥ 6, else low.
fn confidence_label(score: i64) -> String {
    if score >= CONFIDENCE_HIGH {
        "high".into()
    } else if score >= CONFIDENCE_MEDIUM {
        "medium".into()
    } else {
        "low".into()
    }
}

// =============================================================================
// 7. Render + 8. fire-record assembly
// =============================================================================

/// Render the advisory text (§5). WP-5 wraps this into `additionalContext`.
fn render_advisory(confidence: &str, memories: &[SurfacedMemory]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Possible memory match (confidence: {confidence}).\n\n"
    ));
    for (i, m) in memories.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, m.snippet));
        let cites: Vec<String> = m.citations.iter().map(Citation::render).collect();
        out.push_str(&format!("   {}\n", cites.join("; ")));
    }
    out
}

/// Build the [`FireRecord`] for [`Telemetry::log_fire`]. Each surfaced memory
/// contributes one [`FireMem`] whose `{tag, type, val}` is its strongest citation
/// (highest tier, ties broken by citation order). The primitive derives the marked
/// set from `record.mems`, so this list IS the fired set.
fn build_fire_record(query_id: &str, confidence: &str, memories: &[SurfacedMemory]) -> FireRecord {
    let mems = memories
        .iter()
        .map(|m| {
            let rep = representative_citation(m);
            FireMem {
                id: m.memory_id.clone(),
                tag: rep.route_tag.clone(),
                trigger_type: rep.trigger_type.clone(),
                val: rep.matched_value.clone(),
            }
        })
        .collect();
    FireRecord {
        ts: now_unix(),
        qid: query_id.to_string(),
        mems,
        conf: confidence.to_string(),
    }
}

/// The strongest citation of a memory (highest tier by trigger_type), for the
/// telemetry `{tag, type, val}`. A surfaced memory always has ≥1 citation.
fn representative_citation(m: &SurfacedMemory) -> &Citation {
    m.citations
        .iter()
        .max_by_key(|c| trigger_type_rank(&c.trigger_type))
        .expect("a surfaced memory has at least one citation")
}

/// Rank a trigger type by tier (strong 2, medium 1, weak 0) for representative
/// selection.
fn trigger_type_rank(trigger_type: &str) -> u8 {
    match trigger_type {
        "command" | "path" => 2,
        "arg" => 1,
        _ => 0,
    }
}

// =============================================================================
// query id + time helpers
// =============================================================================

/// A deterministic query id (the fire-record discriminator): FNV-1a/64 over the
/// tool name + the sorted query buckets, `memq_`-prefixed. Stable across runs
/// (fixed seed) so equal queries share a qid, distinct ones (with overwhelming
/// probability) differ.
fn compute_query_id(tool_name: &str, walk: &WalkQuery) -> String {
    let mut h = Fnv::new();
    h.field(tool_name.as_bytes());
    for bucket in [&walk.commands, &walk.paths, &walk.args] {
        let mut sorted = bucket.clone();
        sorted.sort();
        h.update(&(sorted.len() as u64).to_le_bytes());
        for v in sorted {
            h.field(v.as_bytes());
        }
    }
    format!("memq_{:016x}", h.0)
}

/// A minimal length-prefixed FNV-1a/64 (deterministic, zero-dependency) for the
/// query id. Not a security primitive — a change detector, like the catalog's.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn field(&mut self, bytes: &[u8]) {
        self.update(&(bytes.len() as u64).to_le_bytes());
        self.update(bytes);
    }
}

/// Now, as a unix timestamp (seconds). Clock-before-epoch → 0 (never panics).
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Today, as whole days since the unix epoch.
fn today_days() -> i64 {
    now_unix().div_euclid(86_400)
}

/// Is `last_reviewed` stale (§5)? Parses the leading `YYYY-MM-DD` and returns
/// `true` iff it is more than [`STALE_DAYS`] days before `today_days`. An empty or
/// unparseable value is NOT stale (parity with synapse `_is_stale`).
fn is_stale(last_reviewed: &str, today_days: i64) -> bool {
    let Some((y, m, d)) = parse_ymd(last_reviewed) else {
        return false;
    };
    let reviewed = civil_to_days(y, m, d);
    today_days - reviewed > STALE_DAYS
}

/// Parse a leading `YYYY-MM-DD` date (the first 10 chars). `None` on any shape
/// fault.
fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let head: String = s.chars().take(10).collect();
    let mut parts = head.split('-');
    let y = parts.next()?.parse::<i64>().ok()?;
    let m = parts.next()?.parse::<i64>().ok()?;
    let d = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

/// Days from the civil date `y-m-d` to the unix epoch (Howard Hinnant's algorithm).
/// Pure integer arithmetic — no date crate.
fn civil_to_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_gate_matrix() {
        // GOOD: one strong tuple fires; two tuples fire.
        assert!(meets_surface_gate(&[tuple("command", Tier::Strong, "rg")]));
        assert!(meets_surface_gate(&[
            tuple("arg", Tier::Medium, "release"),
            tuple("synonym", Tier::Weak, "vram"),
        ]));
        // BAD: no evidence is silent; one weak tuple is silent.
        assert!(!meets_surface_gate(&[]));
        assert!(!meets_surface_gate(&[tuple("synonym", Tier::Weak, "vram")]));
        // One medium tuple alone is also silent (not strong, only one total).
        assert!(!meets_surface_gate(&[tuple(
            "arg",
            Tier::Medium,
            "release"
        )]));
    }

    #[test]
    fn score_tuples_forms_are_frozen() {
        // strong(10) + medium(6) = 16, no penalties.
        assert_eq!(
            score_tuples(
                &[
                    tuple("command", Tier::Strong, "rg"),
                    tuple("arg", Tier::Medium, "x")
                ],
                0,
                "",
                20_000,
            ),
            16
        );
        // -2×min(declineCount,3): declineCount 5 clamps to 3 → -6.
        assert_eq!(
            score_tuples(&[tuple("command", Tier::Strong, "rg")], 5, "", 20_000),
            10 - 6
        );
        // -5×stale: a review > 180 days before today.
        let today = civil_to_days(2026, 7, 4);
        assert_eq!(
            score_tuples(
                &[tuple("command", Tier::Strong, "rg")],
                0,
                "2020-01-01",
                today
            ),
            10 - 5,
        );
        // A recent review is not stale.
        assert_eq!(
            score_tuples(
                &[tuple("command", Tier::Strong, "rg")],
                0,
                "2026-06-01",
                today
            ),
            10,
        );
    }

    #[test]
    fn confidence_thresholds() {
        assert_eq!(confidence_label(10), "high");
        assert_eq!(confidence_label(6), "medium");
        assert_eq!(confidence_label(5), "low");
    }

    #[test]
    fn citation_renders_frozen_form() {
        let c = Citation {
            route_tag: "gpu".into(),
            trigger_type: "command".into(),
            matched_value: "nvidia-smi".into(),
        };
        assert_eq!(c.render(), "gpu <- command:nvidia-smi");
    }

    #[test]
    fn civil_to_days_matches_known_epoch() {
        assert_eq!(civil_to_days(1970, 1, 1), 0);
        assert_eq!(civil_to_days(1970, 1, 2), 1);
        assert_eq!(civil_to_days(2000, 1, 1), 10_957);
    }

    #[test]
    fn parse_ymd_good_and_bad() {
        assert_eq!(parse_ymd("2026-07-04"), Some((2026, 7, 4)));
        assert_eq!(parse_ymd("2026-07-04T12:00:00Z"), Some((2026, 7, 4)));
        assert_eq!(parse_ymd(""), None);
        assert_eq!(parse_ymd("not-a-date"), None);
        assert_eq!(parse_ymd("2026-13-01"), None); // month out of range
    }

    #[test]
    fn keyword_runs_splits_and_preserves_hyphens() {
        assert_eq!(
            keyword_runs("Plasma Compositor v6"),
            vec!["plasma", "compositor", "v6"]
        );
        assert_eq!(keyword_runs("plasma-compositor"), vec!["plasma-compositor"]);
        assert_eq!(keyword_runs("/react/next.js"), vec!["react", "next", "js"]);
    }

    fn tuple(trigger_type: &str, tier: Tier, val: &str) -> FiringTuple {
        FiringTuple {
            route_tag: "t".into(),
            trigger_type: trigger_type.into(),
            matched_value: val.into(),
            tier,
        }
    }
}
