//! `rebuild`: scan the store → the two artifacts, atomically, plus the §11 drift
//! guardrail (plan P4; D2, D14, D24, A2, D10, D5, D8, §4, §11).
//!
//! `rebuild` scans the store (memory `.md` files + grammar), folds grammar-tag
//! evidence (source `t`, `route_tag` = tag name) AND per-memory
//! `metadata.triggers` (source `m`, `route_tag` = memory id) into the four flat
//! tables — pre-flattened one row per `(table, pattern, memory_id)` (A2c) — and
//! writes the flat index ([`crate::index`]) then the catalog report
//! ([`crate::catalog`]).
//!
//! ## Ordering + atomicity (D14, A2d)
//!
//! The **index is written first, the report last**, each via
//! [`crate::catalog::write_atomic`] (write-temp-then-rename). Both carry an
//! identical generation id + `sourceFingerprint`. A crash between the two writes
//! leaves the report stale relative to the index; because the generation id is
//! input-derived, the two disagree and the single reader detects the stale pair
//! (fail-open advisory).
//!
//! ## Ranking vs routing (D10)
//!
//! `rebuild` regenerates the whole index unconditionally; the D10 partition lives
//! at the *caller*: the WP-5 post-op refresh must invoke `rebuild` only on a
//! **routing-affecting** store write (a trigger/tag/grammar change), and must NOT
//! rebuild for a **ranking-only** write (`lastReviewed` / `declineCount`), since
//! those columns are re-read from the existing index without a rebuild. The
//! tripwire if a field ever becomes both is [`drift_guardrail`]'s partition
//! assertion (§4, §11).
//!
//! ## Control-char policy (A2e)
//!
//! A routing-critical field (a command/arg/synonym key, or a path glob)
//! containing a tab/newline/CR excludes that ENTRY from the index and lists it in
//! the routability report — never a build failure. The display `snippet` is
//! sanitized (control chars → space) and truncated at build.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use crate::catalog::{
    CatalogReport, ExcludedEntry, IndexHeader, MemorySummary, RoutabilityReport, generation_id,
    source_fingerprint, write_atomic,
};
use crate::frontmatter::{self, Frontmatter};
use crate::grammar::{self, Entry, Grammar, GrammarError, render_digest};
use crate::index::{Index, IndexRecord, emit_records, routing_key};
use crate::tier::{Axis, SCHEMA_VERSION, Source, routing_ranking_partition_holds};

/// The flat-index artifact filename (infra: underscore-prefixed, so the scan
/// excludes it).
pub const INDEX_FILENAME: &str = "_flat_index.tsv";
/// The catalog report artifact filename (infra).
pub const REPORT_FILENAME: &str = "_memory_catalog.json";

/// Build-time knobs. `max_description_chars` mirrors the §10 config default.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    /// The snippet truncation length (config `maxDescriptionChars`, default 220).
    pub max_description_chars: usize,
}

impl Default for BuildConfig {
    fn default() -> Self {
        BuildConfig {
            max_description_chars: 220,
        }
    }
}

// =============================================================================
// Store scan
// =============================================================================

/// One scanned, parsed memory.
#[derive(Debug, Clone)]
pub struct MemoryFacts {
    /// The memory id (file stem — filename minus `.md`).
    pub id: String,
    /// The file name (for generation hashing + malformed reporting).
    pub filename: String,
    /// The file path (the `path` display column + report).
    pub path: String,
    /// The raw file content (a generation-hash input).
    pub content: String,
    /// The parsed frontmatter (WP-1 parser; reused, never re-derived).
    pub fm: Frontmatter,
}

/// A store `.md` file is infra (excluded from the memory corpus) iff it is
/// underscore-prefixed or is `MEMORY.md` (§3). `pub(crate)` so WP-5's post-op
/// dispatch ([`crate::hook`]) can classify a written target as a memory file the
/// SAME way the store scan does — one definition, no drift between "what
/// `rebuild` scans" and "what the post-op refresh/read-signal treats as a
/// memory".
pub(crate) fn is_infra(name: &str) -> bool {
    name.starts_with('_') || name == "MEMORY.md"
}

/// The result of a store scan: the parsed memories plus the `(filename, content)`
/// of files that failed to parse (skipped, recorded advisory-only).
pub type ScanResult = (Vec<MemoryFacts>, Vec<(String, String)>);

/// Scan the store directory (non-recursively) for memory `.md` files. Returns the
/// parsed memories and the `(filename, content)` of files that failed to parse
/// (skipped — a malformed memory contributes no routes; recorded advisory-only).
pub fn scan_store(store_dir: &Path) -> std::io::Result<ScanResult> {
    let mut entries: Vec<_> = std::fs::read_dir(store_dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    let mut memories = Vec::new();
    let mut malformed = Vec::new();
    for e in entries {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") || is_infra(&name) {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        match frontmatter::parse(&content) {
            Ok(fm) => memories.push(MemoryFacts {
                id: name.strip_suffix(".md").unwrap_or(&name).to_string(),
                filename: name,
                path: path.to_string_lossy().into_owned(),
                content,
                fm,
            }),
            Err(_) => malformed.push((name, content)),
        }
    }
    Ok((memories, malformed))
}

// =============================================================================
// Build artifacts
// =============================================================================

/// The two in-memory artifacts plus build byproducts, before writing.
#[derive(Debug, Clone)]
pub struct Artifacts {
    /// The sorted, deduped flat-index records.
    pub records: Vec<IndexRecord>,
    /// The full flat-index file text (metadata header + record lines).
    pub index_text: String,
    /// The catalog report.
    pub report: CatalogReport,
    /// The catalog report JSON text.
    pub report_text: String,
    /// The generation id both artifacts carry.
    pub generation: String,
    /// The grammar fingerprint both artifacts carry.
    pub source_fingerprint: String,
    /// The §11 drift guardrail advisories (fail-open; never blocks).
    pub drift: DriftReport,
}

/// A memory's per-row display/ranking bundle, computed once per memory (the same
/// values ride every row that memory produces).
struct MemDisplay {
    id: String,
    last_reviewed: String,
    decline_count: i64,
    tags: Vec<String>,
    path: String,
    snippet: String,
}

fn mem_display(m: &MemoryFacts, cfg: &BuildConfig) -> MemDisplay {
    MemDisplay {
        id: m.id.clone(),
        // `lastReviewed` is an opaque scalar (a `\t`/`\n`/`\r` from a double-quote
        // escape would split the line): sanitize it to spaces, like the snippet.
        last_reviewed: sanitize_line_field(m.fm.metadata.last_reviewed.as_deref().unwrap_or("")),
        decline_count: m.fm.metadata.decline_count.unwrap_or(0),
        tags: m.fm.metadata.tags.clone(),
        path: m.path.clone(),
        snippet: build_snippet(
            m.fm.description.as_deref().unwrap_or(""),
            cfg.max_description_chars,
        ),
    }
}

/// The outcome of turning one evidence value into a row.
enum RowOutcome {
    Record(Box<IndexRecord>),
    Excluded(ExcludedEntry),
    Skip,
}

/// Turn one evidence value on `axis` into a row, applying the build-time policy:
/// control-char exclusion (A2e), then — for the three exact-key tables — the
/// shared normalization; byPath is exempt (raw glob preserved, A3).
fn make_row(
    axis: Axis,
    raw_value: &str,
    source: Source,
    route_tag: &str,
    disp: &MemDisplay,
) -> RowOutcome {
    // Control-char policy (A2e): a routing-critical field with tab/newline/CR is
    // EXCLUDED (never a build failure) and reported.
    if raw_value.contains(['\t', '\n', '\r']) {
        return RowOutcome::Excluded(ExcludedEntry {
            memory_id: disp.id.clone(),
            table: axis.table_str().to_string(),
            reason: format!(
                "control char (tab/newline/CR) in {} pattern",
                axis.trigger_type_str()
            ),
        });
    }
    let pattern = if axis.is_path() {
        // byPath EXEMPT from normalization: preserve the raw glob verbatim
        // (case- and slash-bearing). Only surrounding whitespace / blanks drop.
        let t = raw_value.trim();
        if t.is_empty() {
            return RowOutcome::Skip;
        }
        t.to_string()
    } else {
        // byCommand/byArg/bySynonym: normalize the key the SAME way the walk
        // normalizes the query. A non-routing form (`--bare`, `-p`, empty)
        // normalizes to None and is silently dropped — it can never match.
        match routing_key(raw_value) {
            Some(k) => k,
            None => return RowOutcome::Skip,
        }
    };
    RowOutcome::Record(Box::new(IndexRecord {
        axis,
        pattern,
        route_tag: route_tag.to_string(),
        source,
        memory_id: disp.id.clone(),
        mem_type: String::new(), // no `type` key in the reseed frontmatter dialect
        last_reviewed: disp.last_reviewed.clone(),
        decline_count: disp.decline_count,
        tags: disp.tags.clone(),
        path: disp.path.clone(),
        snippet: disp.snippet.clone(),
    }))
}

fn push_axis_rows(
    records: &mut Vec<IndexRecord>,
    excluded: &mut Vec<ExcludedEntry>,
    axis: Axis,
    values: &[String],
    source: Source,
    route_tag: &str,
    disp: &MemDisplay,
) {
    for v in values {
        match make_row(axis, v, source, route_tag, disp) {
            RowOutcome::Record(r) => records.push(*r),
            RowOutcome::Excluded(e) => excluded.push(e),
            RowOutcome::Skip => {}
        }
    }
}

fn all_entries(g: &Grammar) -> Vec<(&str, &Entry)> {
    let mut v = Vec::new();
    for map in [&g.domain, &g.tool, &g.pattern] {
        for (k, e) in map {
            v.push((k.as_str(), e));
        }
    }
    v
}

fn grammar_has_tag(g: &Grammar, tag: &str) -> bool {
    g.domain.contains_key(tag) || g.tool.contains_key(tag) || g.pattern.contains_key(tag)
}

/// Build both artifacts in memory. Pure: no filesystem writes (the caller writes,
/// index-first). The grammar is assumed already valid (the caller validated it).
pub fn build_artifacts(
    memories: &[MemoryFacts],
    malformed: &[(String, String)],
    grammar: &Grammar,
    grammar_text: &str,
    cfg: &BuildConfig,
) -> Artifacts {
    let displays: Vec<MemDisplay> = memories.iter().map(|m| mem_display(m, cfg)).collect();

    let mut excluded: Vec<ExcludedEntry> = Vec::new();

    // A2e generalized: `memory_id` (col 5), `route_tag` for source=m (col 3), and
    // `path` (col 12) are ALL derived from the filename. If the filename stem or
    // path carries a `\t`/`\n`/`\r` (Linux-legal), those columns would split the
    // emitted line and — via the single reader's `ColumnCount` → `Malformed` —
    // discard the ENTIRE index, disabling recall store-wide from one memory.
    // Exclude such a memory's rows wholesale and report it (never emit it).
    let mut hostile_named: BTreeSet<&str> = BTreeSet::new();
    for m in memories {
        if has_line_control(&m.id) || has_line_control(&m.path) {
            hostile_named.insert(m.id.as_str());
            excluded.push(ExcludedEntry {
                memory_id: m.id.clone(),
                table: "(all)".to_string(),
                reason:
                    "control char (tab/newline/CR) in memory filename/path — whole memory excluded"
                        .to_string(),
            });
        }
    }

    // tag -> member displays (grammar-known tags only; unknown tags don't route;
    // hostile-named memories are excluded from routing entirely).
    let mut tag_members: BTreeMap<&str, Vec<&MemDisplay>> = BTreeMap::new();
    for (m, d) in memories.iter().zip(&displays) {
        if hostile_named.contains(m.id.as_str()) {
            continue;
        }
        for tag in &m.fm.metadata.tags {
            if grammar_has_tag(grammar, tag) {
                tag_members.entry(tag.as_str()).or_default().push(d);
            }
        }
    }

    let mut records: Vec<IndexRecord> = Vec::new();

    // Grammar-tag routes (source `t`): pre-flattened one row per member memory.
    for (tag_name, entry) in all_entries(grammar) {
        let Some(members) = tag_members.get(tag_name) else {
            continue; // a grammar tag no memory carries routes to nobody
        };
        for d in members {
            push_axis_rows(
                &mut records,
                &mut excluded,
                Axis::Command,
                &entry.commands,
                Source::Tag,
                tag_name,
                d,
            );
            push_axis_rows(
                &mut records,
                &mut excluded,
                Axis::Path,
                &entry.paths,
                Source::Tag,
                tag_name,
                d,
            );
            push_axis_rows(
                &mut records,
                &mut excluded,
                Axis::Arg,
                &entry.args,
                Source::Tag,
                tag_name,
                d,
            );
            push_axis_rows(
                &mut records,
                &mut excluded,
                Axis::Synonym,
                &entry.synonyms,
                Source::Tag,
                tag_name,
                d,
            );
        }
    }

    // Per-memory triggers (source `m`): route_tag = memory id.
    for (m, d) in memories.iter().zip(&displays) {
        if hostile_named.contains(m.id.as_str()) {
            continue;
        }
        if let Some(tr) = &m.fm.metadata.triggers {
            push_axis_rows(
                &mut records,
                &mut excluded,
                Axis::Command,
                &tr.commands,
                Source::Memory,
                &d.id,
                d,
            );
            push_axis_rows(
                &mut records,
                &mut excluded,
                Axis::Path,
                &tr.paths,
                Source::Memory,
                &d.id,
                d,
            );
            push_axis_rows(
                &mut records,
                &mut excluded,
                Axis::Arg,
                &tr.args,
                Source::Memory,
                &d.id,
                d,
            );
            push_axis_rows(
                &mut records,
                &mut excluded,
                Axis::Synonym,
                &tr.synonyms,
                Source::Memory,
                &d.id,
                d,
            );
        }
    }

    // Deterministic order + exact-duplicate removal (idempotence, grep-friendly
    // grouping by table then pattern).
    records.sort();
    records.dedup();

    // Routability (D18): parsed memories with zero routing rows, sorted.
    let routed: BTreeSet<&str> = records.iter().map(|r| r.memory_id.as_str()).collect();
    let mut unroutable_ids: Vec<String> = memories
        .iter()
        .map(|m| m.id.clone())
        .filter(|id| !routed.contains(id.as_str()))
        .collect();
    unroutable_ids.sort();

    // Generation id + fingerprint from ALL inputs (grammar + every scanned file).
    let mut all_files: Vec<(String, String)> = memories
        .iter()
        .map(|m| (m.filename.clone(), m.content.clone()))
        .collect();
    all_files.extend(malformed.iter().cloned());
    let generation = generation_id(grammar_text, &all_files);
    let fingerprint = source_fingerprint(grammar_text);

    let report = CatalogReport {
        schema_version: SCHEMA_VERSION,
        generation: generation.clone(),
        source_fingerprint: fingerprint.clone(),
        memories: memories
            .iter()
            .map(|m| MemorySummary {
                id: m.id.clone(),
                path: m.path.clone(),
                tags: m.fm.metadata.tags.clone(),
                description: m.fm.description.clone().unwrap_or_default(),
            })
            .collect(),
        routability_report: RoutabilityReport {
            unroutable_count: unroutable_ids.len(),
            unroutable_ids,
            excluded_entries: excluded,
        },
        vocab_digest: render_digest(grammar),
        malformed_files: malformed.iter().map(|(name, _)| name.clone()).collect(),
    };

    let header = IndexHeader {
        generation: generation.clone(),
        source_fingerprint: fingerprint.clone(),
        schema_version: SCHEMA_VERSION,
    };
    let index_text = format!("{}\n{}", header.emit(), emit_records(&records));

    // §11 drift guardrail against the index we just built (fail-open).
    let index = Index::from_records(records.clone());
    let drift = drift_guardrail(memories, &index);

    let report_text = report.to_json();
    Artifacts {
        records,
        index_text,
        report,
        report_text,
        generation,
        source_fingerprint: fingerprint,
        drift,
    }
}

/// Whether a string carries a line-splitting control char (`\t` / `\n` / `\r`) —
/// the three that break the one-record-per-line / 13-column invariant.
fn has_line_control(s: &str) -> bool {
    s.contains(['\t', '\n', '\r'])
}

/// Replace line-splitting control chars (`\t` / `\n` / `\r`) with spaces, keeping
/// a display scalar on one physical line. Used for `lastReviewed` (the snippet
/// uses the broader `is_control` sanitizer).
fn sanitize_line_field(s: &str) -> String {
    s.chars()
        .map(|c| {
            if matches!(c, '\t' | '\n' | '\r') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// The `snippet` column build (A2e; synapse `_esc` + truncate). Control chars →
/// space (one-line-safe), then entity-escape (`&` `<` `>`), then truncate to
/// `maxd` without cutting a half-written entity, appending `…` if cut.
fn build_snippet(desc: &str, maxd: usize) -> String {
    let sanitized: String = desc
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let escaped = sanitized
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    truncate_escaped(&escaped, maxd)
}

fn truncate_escaped(e: &str, maxd: usize) -> String {
    let chars: Vec<char> = e.chars().collect();
    if chars.len() <= maxd {
        return e.to_string();
    }
    let mut cut: Vec<char> = chars[..maxd.saturating_sub(1)].to_vec();
    // Don't cut a half-written entity: if the tail holds a `&` with no following
    // `;`, back up to that `&`.
    if let Some(amp) = cut.iter().rposition(|&c| c == '&')
        && !cut[amp..].contains(&';')
    {
        cut.truncate(amp);
    }
    let mut s: String = cut.into_iter().collect();
    s = s.trim_end().to_string();
    s.push('…');
    s
}

// =============================================================================
// The §11 drift guardrail (fail-open advisory; run at rebuild + session-start)
// =============================================================================

/// The §11 drift guardrail result: zero or more advisory lines. It NEVER blocks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriftReport {
    /// One line per violated assertion.
    pub advisories: Vec<String>,
}

impl DriftReport {
    /// Whether the guardrail found no drift.
    pub fn is_clean(&self) -> bool {
        self.advisories.is_empty()
    }
}

/// The §11 drift guardrail — a fail-open advisory pass over the index data,
/// reusable at session-start (WP-5). It surfaces that a point-in-time assumption
/// has drifted; it is a guardrail, not a gate, and never blocks.
///
/// Three assertions:
/// 1. no existing trigger-bearing memory is a bare-degenerate-only set (would be
///    denied by the static gate today);
/// 2. no curated memory would be BLOCK-degenerate under the current verdict;
/// 3. the D10 routing-vs-ranking partition holds.
///
/// Both assertions are now backed by the real WP-4 predicates (still fail-open,
/// advisory — this is a guardrail, never a gate). Assertion 1
/// ([`static_gate_would_deny`]) unions the sound conservative arm (a
/// trigger-bearing memory that routes nowhere) with the real two-arm static gate
/// ([`crate::guard::static_gate_denies`]) over the declared trigger set. Assertion
/// 2 ([`would_block_degenerate`]) runs the real collision projection
/// ([`crate::projection::project`]) and checks the BLOCK-degenerate verdict,
/// excluding the memory's own rows so the breadth mirrors the new-file verdict the
/// write guard would render.
pub fn drift_guardrail(memories: &[MemoryFacts], index: &Index) -> DriftReport {
    let mut advisories = Vec::new();
    let routed = index.routed_memory_ids();

    // Assertion 1.
    for m in memories {
        if is_trigger_bearing(&m.fm) && static_gate_would_deny(m, index, &routed) {
            advisories.push(format!(
                "drift[static-gate]: trigger-bearing memory `{}` has a degenerate declared \
                 trigger set (routes nowhere, or only generic commands / broad paths with no live \
                 lever) — would be denied by the static gate",
                m.id
            ));
        }
    }

    // Assertion 2 (WP-4 classifier stub; fails open).
    for m in memories {
        if would_block_degenerate(m, index) {
            advisories.push(format!(
                "drift[block-degenerate]: curated memory `{}` is BLOCK-degenerate under the current verdict",
                m.id
            ));
        }
    }

    // Assertion 3: the D10 routing/ranking partition tripwire.
    if !routing_ranking_partition_holds() {
        advisories.push(
            "drift[partition]: the D10 routing-vs-ranking column partition is violated \
             (a field is both routing- and ranking-affecting) — a ranking-only write could now \
             silently skip a rebuild it needs"
                .to_string(),
        );
    }

    DriftReport { advisories }
}

fn is_trigger_bearing(fm: &Frontmatter) -> bool {
    fm.metadata.triggers.as_ref().is_some_and(|t| {
        !t.commands.is_empty()
            || !t.paths.is_empty()
            || !t.args.is_empty()
            || !t.synonyms.is_empty()
    })
}

/// Would the static degenerate gate deny this memory's trigger set today? The union
/// of two sound arms:
/// 1. the conservative arm — a memory with declared triggers but **no live route in
///    the index** (every declared trigger normalized away or was excluded, so it
///    routes nowhere); and
/// 2. the real static gate ([`crate::guard::static_gate_denies`]) over the declared
///    trigger set — flagging routable-but-degenerate sets (only broad paths via
///    `is_broad_path` §3.x, or only generic commands via `GENERIC_VERBS`) with no
///    narrowing lever, exactly as the write guard would.
///
/// Advisory-only (§11): this never blocks a rebuild.
fn static_gate_would_deny(m: &MemoryFacts, index: &Index, routed: &BTreeSet<String>) -> bool {
    if !routed.contains(&m.id) {
        return true; // routes nowhere — the sound conservative arm
    }
    m.fm.metadata
        .triggers
        .as_ref()
        .is_some_and(|t| crate::guard::static_gate_denies(t, index))
}

/// Would this memory be BLOCK-degenerate under the current collision verdict (§7)?
/// Runs the real projection ([`crate::projection::project`]) over the memory's
/// declared trigger set and checks the strict-`>`-floor + empty-live-levers verdict,
/// **excluding the memory's own rows** (it is already in the index at rebuild time)
/// so the breadth mirrors the new-file verdict the write guard would render.
/// Advisory-only (§11): never a block.
fn would_block_degenerate(m: &MemoryFacts, index: &Index) -> bool {
    let Some(triggers) = m.fm.metadata.triggers.as_ref() else {
        return false; // no declared trigger set → nothing to project
    };
    let proj = crate::projection::project(triggers, index);
    let breadth = proj.collisions.iter().filter(|id| **id != m.id).count();
    breadth > crate::projection::COLLISION_GUIDE_FLOOR && proj.live_levers.is_empty()
}

// =============================================================================
// rebuild orchestration
// =============================================================================

/// A rebuild's summary (the direct-CLI + session-start surfaces consume this).
#[derive(Debug, Clone)]
pub struct RebuildOutcome {
    /// The generation id both artifacts were written with.
    pub generation: String,
    /// Count of unroutable memories.
    pub unroutable_count: usize,
    /// Ids of unroutable memories, sorted.
    pub unroutable_ids: Vec<String>,
    /// Count of build-time excluded (control-char) entries.
    pub excluded_count: usize,
    /// The §11 drift advisories (fail-open).
    pub drift_advisories: Vec<String>,
}

/// A rebuild failure. `rebuild` is a direct-CLI operation (loud, fail-closed on
/// missing deps, D12); the CLI layer (WP-7) maps these to exit codes.
#[derive(Debug)]
pub enum RebuildError {
    /// A filesystem error reading the store/grammar or writing an artifact.
    Io(std::io::Error),
    /// The grammar failed to load/validate (config/taxonomy, exit-2 class).
    Grammar(GrammarError),
}

impl fmt::Display for RebuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RebuildError::Io(e) => write!(f, "rebuild I/O error: {e}"),
            RebuildError::Grammar(e) => write!(f, "rebuild grammar error: {e}"),
        }
    }
}

impl std::error::Error for RebuildError {}

/// Rebuild both artifacts from the store + grammar and write them, index-first
/// (D14, A2d). The grammar is loaded and validated here (a bad grammar is a hard
/// error — the loud direct-CLI posture).
pub fn rebuild(
    store_dir: &Path,
    grammar_path: &Path,
    cfg: &BuildConfig,
) -> Result<RebuildOutcome, RebuildError> {
    let grammar_text = std::fs::read_to_string(grammar_path).map_err(RebuildError::Io)?;
    let grammar = grammar::parse_and_validate(&grammar_text).map_err(RebuildError::Grammar)?;
    let (memories, malformed) = scan_store(store_dir).map_err(RebuildError::Io)?;
    let artifacts = build_artifacts(&memories, &malformed, &grammar, &grammar_text, cfg);

    // Index FIRST, report LAST (A2d), each atomic (D14).
    write_atomic(&store_dir.join(INDEX_FILENAME), &artifacts.index_text)
        .map_err(RebuildError::Io)?;
    write_atomic(&store_dir.join(REPORT_FILENAME), &artifacts.report_text)
        .map_err(RebuildError::Io)?;

    Ok(RebuildOutcome {
        generation: artifacts.generation,
        unroutable_count: artifacts.report.routability_report.unroutable_count,
        unroutable_ids: artifacts.report.routability_report.unroutable_ids.clone(),
        excluded_count: artifacts.report.routability_report.excluded_entries.len(),
        drift_advisories: artifacts.drift.advisories,
    })
}

/// Convenience: the standard artifact paths under a store.
pub fn index_path(store_dir: &Path) -> std::path::PathBuf {
    store_dir.join(INDEX_FILENAME)
}
/// Convenience: the standard report path under a store.
pub fn report_path(store_dir: &Path) -> std::path::PathBuf {
    store_dir.join(REPORT_FILENAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_sanitizes_escapes_and_truncates() {
        // Control chars → space; entities escaped.
        assert_eq!(build_snippet("a\tb\nc", 220), "a b c");
        assert_eq!(
            build_snippet("x & y < z > w", 220),
            "x &amp; y &lt; z &gt; w"
        );
        // Truncation appends an ellipsis and stays within bound.
        let long = "a".repeat(500);
        let s = build_snippet(&long, 220);
        assert!(s.chars().count() <= 220);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn snippet_does_not_cut_a_half_entity() {
        // Force the cut to land right after an `&amp;`-ish boundary.
        let desc = format!("{}&more", "z".repeat(216));
        let s = build_snippet(&desc, 220);
        // The escaped `&more` becomes `&more` (no entity), but the guard backs off
        // any dangling `&` with no `;`.
        assert!(!s.trim_end_matches('…').ends_with('&'));
    }

    #[test]
    fn infra_files_are_excluded() {
        assert!(is_infra("_grammar.md"));
        assert!(is_infra("_flat_index.tsv"));
        assert!(is_infra("MEMORY.md"));
        assert!(!is_infra("gpu-notes.md"));
    }
}
