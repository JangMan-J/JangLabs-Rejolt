//! The flat recall index — the **sole** routing structure — and the **one**
//! walk over it (plan P5; D4, D24, A2, A3, D15; §5, §7).
//!
//! This module carries the second (and last) bespoke *read* surface in the
//! system (A3(d)); it is kept trivial by A2(e)'s no-escaping rule: one record is
//! one physical line, columns are tab-separated, and load splits on `\t` with NO
//! unescaping. There is no escaping layer because routing-critical fields that
//! would need one are excluded at build time (see [`crate::rebuild`]).
//!
//! ## The 13-column record (Appendix A, EXACT)
//!
//! `table, pattern, route_tag, source, memory_id, trigger_type, tier, type,
//! lastReviewed, declineCount, tags, path, snippet`
//!
//! `table` / `trigger_type` / `tier` are all derived from one [`Axis`]
//! ([`crate::tier`]); the load path keys off `table` and regenerates the other
//! two, so a record can never carry an inconsistent tier.
//!
//! ## The one walk (D4 / N1 — the crux)
//!
//! [`Index::walk`] is the single, **ungated and unscored** matcher. Recall
//! (WP-3) and collision projection (WP-4) both consume it; there is no second
//! matcher. It returns the raw [`Hit`]s so a consumer can gate / score / count
//! in one pass. Liveness (WP-4) does NOT re-walk: it reads the index-key
//! membership accessors ([`Index::contains_arg_key`],
//! [`Index::contains_synonym_key`], [`Index::arg_or_synonym_key_set`]) computed
//! independently of the co-fire walk.
//!
//! ## Key normalization (Appendix A)
//!
//! byCommand / byArg / bySynonym keys are normalized at build AND at query
//! (strip + lowercase, then the [`crate::tag::is_tag`] conformance filter) — so a
//! query token routes iff it matches a key the *same* normalization produced.
//! This fixes synapse's raw-key vs normalized-lookup asymmetry. **byPath is
//! EXEMPT**: its raw glob pattern is preserved (case- and slash-bearing) and
//! matched by a `/**`-prefix scan or fnmatch, never by exact key.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::path_class::is_broad_path;
use crate::tag::is_tag;
use crate::tier::{Axis, COLUMN_COUNT, Source, Tier};

// =============================================================================
// Query normalization — shared by build and walk
// =============================================================================

/// Normalize an exact-key routing token (command / arg / synonym) exactly as the
/// build and the walk both must: trim surrounding whitespace, lowercase, then
/// require [`is_tag`] conformance. Returns the normalized key, or `None` for a
/// token that cannot be a routing key (`--bare`, `-p`, mixed-noise, empty). A
/// `None` here is precisely synapse's `_norm(...) is None` — not live, not
/// matched. **Not applied to paths** (byPath is exempt).
pub fn routing_key(token: &str) -> Option<String> {
    let n = token.trim().to_lowercase();
    if is_tag(&n) { Some(n) } else { None }
}

// =============================================================================
// IndexRecord: one physical line, 13 columns, no escaping layer
// =============================================================================

/// One flat-index record — one physical line. Field order matches Appendix A;
/// `table`/`trigger_type`/`tier` derive from [`IndexRecord::axis`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndexRecord {
    /// The routing axis — renders the `table`, `trigger_type`, and `tier`
    /// columns.
    pub axis: Axis,
    /// Column 2 `pattern`: the routing key. Normalized (strip/lowercase) for
    /// command/arg/synonym; the **raw glob**, verbatim, for path.
    pub pattern: String,
    /// Column 3 `route_tag`: grammar tag name (source `t`) or memory id
    /// (source `m`).
    pub route_tag: String,
    /// Column 4 `source`: `t` grammar-tag route, `m` per-memory trigger.
    pub source: Source,
    /// Column 5 `memory_id`: the memory this row routes to.
    pub memory_id: String,
    /// Column 8 `type`: the memory's classification field. The reseed frontmatter
    /// dialect (D21) has no `type` key, so this is empty for reseed-authored
    /// memories; the column is carried per the frozen schema and consumed by
    /// recall ranking when present.
    pub mem_type: String,
    /// Column 9 `lastReviewed`: ranking metadata (opaque; empty if absent).
    pub last_reviewed: String,
    /// Column 10 `declineCount`: ranking metadata (0 if absent).
    pub decline_count: i64,
    /// Column 11 `tags`: the memory's tags, joined with `,` on emit
    /// (`is_tag` forbids commas, so the join is lossless).
    pub tags: Vec<String>,
    /// Column 12 `path`: the memory file's path (display / citation).
    pub path: String,
    /// Column 13 `snippet`: the description, control-sanitized, entity-escaped,
    /// and truncated at build (see [`crate::rebuild`]).
    pub snippet: String,
}

impl IndexRecord {
    /// The `table` column token.
    pub fn table_str(&self) -> &'static str {
        self.axis.table_str()
    }
    /// The `trigger_type` column token.
    pub fn trigger_type_str(&self) -> &'static str {
        self.axis.trigger_type_str()
    }
    /// The precomputed [`Tier`] (from the type→tier map).
    pub fn tier(&self) -> Tier {
        self.axis.tier()
    }

    /// Emit this record as one physical line: 13 tab-separated columns, NO
    /// escaping. The producer guarantees — via build-time exclusion (routing +
    /// filename-derived fields) and sanitization (display fields) — that no
    /// column contains a tab, newline, or CR. The `debug_assert!` here is the
    /// belt-and-suspenders backstop: a future column addition that skips that
    /// discipline trips it in tests rather than silently splitting a line.
    pub fn emit(&self) -> String {
        let cols = [
            self.table_str().to_string(),
            self.pattern.clone(),
            self.route_tag.clone(),
            self.source.as_str().to_string(),
            self.memory_id.clone(),
            self.trigger_type_str().to_string(),
            self.tier().as_str().to_string(),
            self.mem_type.clone(),
            self.last_reviewed.clone(),
            self.decline_count.to_string(),
            self.tags.join(","),
            self.path.clone(),
            self.snippet.clone(),
        ];
        debug_assert!(
            cols.iter().all(|c| !c.contains(['\t', '\n', '\r'])),
            "IndexRecord::emit: a column holds a control char (would break \
             one-record-per-line): {cols:?}"
        );
        cols.join("\t")
    }

    /// Parse one physical line into a record — split on `\t` with **no
    /// unescaping**. Keys off the `table` column for the axis and regenerates
    /// `trigger_type`/`tier`; the on-disk `trigger_type`/`tier` columns are the
    /// producer's output and are not re-trusted. Returns [`RecordError`] on a
    /// malformed line so the single reader can fail open to `None` (§4).
    pub fn parse(line: &str) -> Result<IndexRecord, RecordError> {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != COLUMN_COUNT {
            return Err(RecordError::ColumnCount(cols.len()));
        }
        let axis =
            Axis::from_table_str(cols[0]).ok_or_else(|| RecordError::Table(cols[0].into()))?;
        let source =
            Source::from_token(cols[3]).ok_or_else(|| RecordError::Source(cols[3].into()))?;
        let decline_count = cols[9]
            .parse::<i64>()
            .map_err(|_| RecordError::DeclineCount(cols[9].into()))?;
        let tags = if cols[10].is_empty() {
            Vec::new()
        } else {
            cols[10].split(',').map(str::to_string).collect()
        };
        Ok(IndexRecord {
            axis,
            pattern: cols[1].to_string(),
            route_tag: cols[2].to_string(),
            source,
            memory_id: cols[4].to_string(),
            // cols[5] trigger_type and cols[6] tier are derived, not re-trusted.
            mem_type: cols[7].to_string(),
            last_reviewed: cols[8].to_string(),
            decline_count,
            tags,
            path: cols[11].to_string(),
            snippet: cols[12].to_string(),
        })
    }
}

/// A malformed flat-index line. Every variant means the single reader fails open
/// to "no index" (§4), never a hard error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// Not exactly [`COLUMN_COUNT`] tab-separated columns.
    ColumnCount(usize),
    /// The `table` column is not one of the four table names.
    Table(String),
    /// The `source` column is neither `t` nor `m`.
    Source(String),
    /// The `declineCount` column is not an integer.
    DeclineCount(String),
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordError::ColumnCount(n) => {
                write!(f, "expected {COLUMN_COUNT} columns, found {n}")
            }
            RecordError::Table(s) => write!(f, "unknown table `{s}`"),
            RecordError::Source(s) => write!(f, "unknown source `{s}` (expected t|m)"),
            RecordError::DeclineCount(s) => write!(f, "non-integer declineCount `{s}`"),
        }
    }
}

impl std::error::Error for RecordError {}

/// Emit records as index lines, one per line, terminated by newlines. Callers
/// prepend the metadata header (see [`crate::catalog`]).
pub fn emit_records(records: &[IndexRecord]) -> String {
    let mut out = String::new();
    for r in records {
        out.push_str(&r.emit());
        out.push('\n');
    }
    out
}

// =============================================================================
// The Index and the one walk
// =============================================================================

/// The four routing tables, loaded from the flat index. This is the only
/// structure the walk reads (A2a).
#[derive(Debug, Clone, Default)]
pub struct Index {
    by_command: BTreeMap<String, Vec<IndexRecord>>,
    by_arg: BTreeMap<String, Vec<IndexRecord>>,
    by_synonym: BTreeMap<String, Vec<IndexRecord>>,
    /// byPath is keyed by the RAW glob pattern (exempt from normalization); the
    /// walk scans these, it never does an exact-key lookup here.
    by_path: BTreeMap<String, Vec<IndexRecord>>,
}

/// A query for the [`Index::walk`], built identically by recall (from a
/// normalized host op) and by projection (from a proposed trigger set) — that
/// identity is what makes recall ≡ projection through one walk (RB9, D4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalkQuery {
    /// Command basenames (normalized on lookup the same way build normalized the
    /// key).
    pub commands: Vec<String>,
    /// Canonical (absolute) query paths, scanned against raw byPath globs.
    pub paths: Vec<String>,
    /// Argument tokens (normalized on lookup).
    pub args: Vec<String>,
    /// Synonym tokens (normalized on lookup).
    pub synonyms: Vec<String>,
}

/// One raw hit from the walk — ungated, unscored. Carries a borrow of the whole
/// matched [`IndexRecord`] (so the consumer gets table / memory_id / route_tag /
/// source / trigger_type / tier plus every ranking + display column) and the
/// query value that produced the match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit<'a> {
    /// The matched index row.
    pub record: &'a IndexRecord,
    /// The query value that matched: the normalized key for command/arg/synonym,
    /// or the query path for byPath.
    pub matched_value: String,
}

impl Index {
    /// Build an [`Index`] from parsed records, bucketing each by its axis and
    /// pattern. The pattern is already normalized (command/arg/synonym) or raw
    /// (path) — this does not re-normalize.
    pub fn from_records(records: Vec<IndexRecord>) -> Index {
        let mut idx = Index::default();
        for r in records {
            let table = match r.axis {
                Axis::Command => &mut idx.by_command,
                Axis::Arg => &mut idx.by_arg,
                Axis::Synonym => &mut idx.by_synonym,
                Axis::Path => &mut idx.by_path,
            };
            table.entry(r.pattern.clone()).or_default().push(r);
        }
        idx
    }

    /// The **one walk** (D4). Ungated, unscored. Returns every raw hit for the
    /// query across all four tables:
    ///
    /// - byCommand / byArg / bySynonym: exact-key after normalizing the query
    ///   token the same way the key was normalized at build.
    /// - byPath: a scan of the raw globs (`/**`-prefix containment, else
    ///   fnmatch), never an exact-key lookup.
    ///
    /// Hits are returned in a deterministic order (table order, then pattern,
    /// then the row's own order) so both consumers see the identical set.
    pub fn walk(&self, q: &WalkQuery) -> Vec<Hit<'_>> {
        let mut hits = Vec::new();
        exact_hits(&self.by_command, &q.commands, &mut hits);
        // byPath sits between command and arg in table order (Appendix A).
        for (glob, recs) in &self.by_path {
            if let Some(matched) = path_scan_match(glob, &q.paths) {
                for r in recs {
                    hits.push(Hit {
                        record: r,
                        matched_value: matched.clone(),
                    });
                }
            }
        }
        exact_hits(&self.by_arg, &q.args, &mut hits);
        exact_hits(&self.by_synonym, &q.synonyms, &mut hits);
        hits
    }

    // --- index-key membership (WP-4 liveness; computed WITHOUT the co-fire walk) ---

    /// Is `token` (after the shared normalization) a key in `byArg`?
    pub fn contains_arg_key(&self, token: &str) -> bool {
        routing_key(token).is_some_and(|k| self.by_arg.contains_key(&k))
    }

    /// Is `token` (after the shared normalization) a key in `bySynonym`?
    pub fn contains_synonym_key(&self, token: &str) -> bool {
        routing_key(token).is_some_and(|k| self.by_synonym.contains_key(&k))
    }

    /// The union of the `byArg` and `bySynonym` key sets. WP-4 liveness composes
    /// from this: an **arg** lever is live iff its normalized form is in
    /// `byArg` OR `bySynonym`; a **synonym** lever is live iff its normalized
    /// form is in `bySynonym` (§7). This is the retired-signal-inversion fix:
    /// liveness is key membership, never co-fire counts (Appendix A of CORE-SPEC).
    pub fn arg_or_synonym_key_set(&self) -> BTreeSet<String> {
        self.by_arg
            .keys()
            .chain(self.by_synonym.keys())
            .cloned()
            .collect()
    }

    /// The set of memory ids that have at least one routing row. The §11 drift
    /// guardrail ([`crate::rebuild::drift_guardrail`]) reads this to find
    /// trigger-bearing memories that route nowhere, without re-walking.
    pub fn routed_memory_ids(&self) -> BTreeSet<String> {
        [
            &self.by_command,
            &self.by_path,
            &self.by_arg,
            &self.by_synonym,
        ]
        .iter()
        .flat_map(|t| t.values())
        .flatten()
        .map(|r| r.memory_id.clone())
        .collect()
    }

    /// Total record count across all tables (diagnostics / tests).
    pub fn len(&self) -> usize {
        [
            &self.by_command,
            &self.by_path,
            &self.by_arg,
            &self.by_synonym,
        ]
        .iter()
        .map(|t| t.values().map(Vec::len).sum::<usize>())
        .sum()
    }

    /// Whether the index holds zero records.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Look up each query token (normalized) in an exact-key table, appending hits.
fn exact_hits<'a>(
    table: &'a BTreeMap<String, Vec<IndexRecord>>,
    tokens: &[String],
    hits: &mut Vec<Hit<'a>>,
) {
    for tok in tokens {
        let Some(key) = routing_key(tok) else {
            continue;
        };
        if let Some(recs) = table.get(&key) {
            for r in recs {
                hits.push(Hit {
                    record: r,
                    matched_value: key.clone(),
                });
            }
        }
    }
}

// =============================================================================
// byPath scan: /**-prefix containment, else fnmatch (Appendix A)
// =============================================================================

/// Match a raw byPath glob against the query paths, returning the first query
/// path that matches. Mirrors Appendix A / the frozen ground-truth matcher
/// (synapse `memory_surface.py:1765-1771`, `project_triggers` pitfall 5): a
/// §3.x-BROAD glob never routes at all, then expand a leading `~`, then
///
/// 1. a **trailing** `/**` is literal-prefix containment
///    (`ap == prefix || ap.startswith(prefix + "/")`);
/// 2. any OTHER `**` (bare, or mid-pattern) is BROAD — it never routes (§3.x
///    classes `**`, `**/*.md`, `~/**/settings.json` as broad); return `None`;
/// 3. otherwise an fnmatch scan.
///
/// `**` is thus sanctioned ONLY as a trailing `/**` behind a CONCRETE prefix.
/// The up-front [`is_broad_path`] gate closes the anchor-only class the two
/// branches below miss on their own: `/**` and `~/**` are trailing-`/**` forms
/// whose prefix is an anchor (empty / `$HOME`), so branch 1 alone would contain
/// EVERY path under them; `/*` / `~/*` reach fnmatch where `*` crosses `/`
/// (Python-fnmatch parity) and likewise match everything. §3.x classifies all
/// of these broad, and this gate is the SAME classifier liveness reads
/// (`projection::live_levers`) — match-time sanctioning and the live-lever
/// definition can never disagree on what a broad path is (D5 precision;
/// deviates from the synapse matcher, which shares the anchor-only hole —
/// D15: synapse is reference, not constraint; walk-back fix F3, 2026-07-04).
fn path_scan_match(glob: &str, paths: &[String]) -> Option<String> {
    if is_broad_path(glob) {
        return None; // §3.x broad → non-routing, regardless of glob shape
    }
    let expanded = expand_tilde(glob);
    if let Some(prefix) = expanded.strip_suffix("/**") {
        // Literal prefix containment: `== prefix` OR under `prefix + "/"` — never
        // a bare `starts_with(prefix)` (which would match `~/.configFOO` for
        // prefix `~/.config`).
        let with_slash = format!("{prefix}/");
        paths
            .iter()
            .find(|p| p.as_str() == prefix || p.starts_with(&with_slash))
            .cloned()
    } else if expanded.contains("**") {
        // Broad, non-routing (§3.x). Do NOT fall through to fnmatch — a mid/bare
        // `**` there would match every path (the false-fire this branch prevents).
        None
    } else {
        let pat: Vec<char> = expanded.chars().collect();
        paths
            .iter()
            .find(|p| fnmatch(&pat, &p.chars().collect::<Vec<char>>()))
            .cloned()
    }
}

/// Expand a leading `~` / `~/` to `$HOME`. If `HOME` is unset the tilde is left
/// literal (deterministic, no panic). Only the anchor is expanded — the rest of
/// the glob is untouched.
fn expand_tilde(pat: &str) -> String {
    let home = match std::env::var_os("HOME") {
        Some(h) => h.to_string_lossy().into_owned(),
        None => return pat.to_string(),
    };
    if pat == "~" {
        home
    } else if let Some(rest) = pat.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        pat.to_string()
    }
}

/// Python-`fnmatch` semantics over char slices: `*` matches any run (INCLUDING
/// `/`), `?` matches any single char, `[...]` is a character class (a leading
/// `!` negates — `^` is a LITERAL class member, matching `fnmatch.fnmatchcase`,
/// NOT a negator; `a-z` ranges), everything else is literal. This is the
/// "fnmatch scan" Appendix A names — deliberately NOT a path-aware glob crate
/// (those treat `/` specially, which would diverge from the frozen semantics).
fn fnmatch(pat: &[char], txt: &[char]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    // Backtrack point for the most recent `*`.
    let mut star: Option<(usize, usize)> = None;
    while t < txt.len() {
        if p < pat.len() && pat[p] == '*' {
            star = Some((p, t));
            p += 1;
            continue;
        }
        if p < pat.len()
            && let Some(next_p) = match_one(pat, p, txt[t])
        {
            p = next_p;
            t += 1;
            continue;
        }
        // Mismatch (or ran out of pattern): backtrack to the last `*`, if any.
        match star {
            Some((sp, st)) => {
                p = sp + 1;
                t = st + 1;
                star = Some((sp, st + 1));
            }
            None => return false,
        }
    }
    // Text consumed: any pattern remainder must be all `*`.
    while p < pat.len() && pat[p] == '*' {
        p += 1;
    }
    p == pat.len()
}

/// Match a single non-`*` pattern unit at `pat[p]` against char `c`. Returns the
/// pattern index just past the unit on a match, else `None`. Handles `?`,
/// `[...]` classes, and literals. An unterminated `[` is treated as a literal
/// `[` (matching Python fnmatch).
fn match_one(pat: &[char], p: usize, c: char) -> Option<usize> {
    match pat[p] {
        '?' => Some(p + 1),
        '[' => match match_class(pat, p, c) {
            Some((matched, next_p)) => matched.then_some(next_p),
            // Unterminated '[' → literal '['.
            None => (c == '[').then_some(p + 1),
        },
        lit => (lit == c).then_some(p + 1),
    }
}

/// Match a `[...]` character class starting at `pat[start]` (which is `[`)
/// against `c`. Returns `(matched, index-just-past-`]`)`, or `None` if the class
/// is unterminated (no closing `]`).
fn match_class(pat: &[char], start: usize, c: char) -> Option<(bool, usize)> {
    let mut i = start + 1;
    // ONLY `!` negates (Python `fnmatch`); `^` is a literal class member.
    let negate = pat.get(i) == Some(&'!');
    if negate {
        i += 1;
    }
    let mut matched = false;
    let mut first = true;
    while i < pat.len() {
        // A `]` as the very first class char is a literal, not the terminator.
        if pat[i] == ']' && !first {
            return Some((matched != negate, i + 1));
        }
        first = false;
        // Range `a-z`: current char, `-`, and a following non-`]` char.
        if i + 2 < pat.len() && pat[i + 1] == '-' && pat[i + 2] != ']' {
            if pat[i] <= c && c <= pat[i + 2] {
                matched = true;
            }
            i += 3;
        } else {
            if pat[i] == c {
                matched = true;
            }
            i += 1;
        }
    }
    None // unterminated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier::Axis;

    fn rec(axis: Axis, pattern: &str, source: Source, mid: &str) -> IndexRecord {
        IndexRecord {
            axis,
            pattern: pattern.to_string(),
            route_tag: mid.to_string(),
            source,
            memory_id: mid.to_string(),
            mem_type: String::new(),
            last_reviewed: String::new(),
            decline_count: 0,
            tags: vec!["t".into()],
            path: format!("/store/{mid}.md"),
            snippet: "desc".into(),
        }
    }

    #[test]
    fn record_emit_parse_round_trip() {
        let r = IndexRecord {
            axis: Axis::Command,
            pattern: "nvidia-smi".into(),
            route_tag: "gpu-tools".into(),
            source: Source::Tag,
            memory_id: "gpu".into(),
            mem_type: String::new(),
            last_reviewed: "2026-07-04".into(),
            decline_count: 3,
            tags: vec!["gpu".into(), "vram".into()],
            path: "/store/gpu.md".into(),
            snippet: "GPU &amp; VRAM".into(),
        };
        let line = r.emit();
        assert_eq!(line.split('\t').count(), COLUMN_COUNT);
        assert_eq!(IndexRecord::parse(&line).unwrap(), r);
    }

    #[test]
    fn record_parse_rejects_malformed() {
        assert!(matches!(
            IndexRecord::parse("only\tthree\tcols"),
            Err(RecordError::ColumnCount(3))
        ));
        let mut cols = vec![
            "byNope", "p", "rt", "t", "m", "command", "strong", "", "", "0", "", "", "",
        ];
        assert!(matches!(
            IndexRecord::parse(&cols.join("\t")),
            Err(RecordError::Table(_))
        ));
        cols[0] = "byCommand";
        cols[3] = "x";
        assert!(matches!(
            IndexRecord::parse(&cols.join("\t")),
            Err(RecordError::Source(_))
        ));
        cols[3] = "t";
        cols[9] = "NaN";
        assert!(matches!(
            IndexRecord::parse(&cols.join("\t")),
            Err(RecordError::DeclineCount(_))
        ));
    }

    #[test]
    fn routing_key_normalization() {
        assert_eq!(routing_key("  RipGrep  ").as_deref(), Some("ripgrep"));
        assert_eq!(routing_key("nvidia-smi").as_deref(), Some("nvidia-smi"));
        // Non-routing forms normalize to None (synapse `_norm` → None).
        assert_eq!(routing_key("--no-cache"), None);
        assert_eq!(routing_key("-p"), None);
        assert_eq!(routing_key(""), None);
    }

    #[test]
    fn walk_exact_key_command_and_arg() {
        let idx = Index::from_records(vec![
            rec(Axis::Command, "nvidia-smi", Source::Tag, "gpu"),
            rec(Axis::Arg, "no-cache", Source::Memory, "cargo"),
        ]);
        let q = WalkQuery {
            commands: vec!["NVIDIA-SMI".into()], // normalizes to nvidia-smi
            args: vec!["no-cache".into()],
            ..Default::default()
        };
        let hits = idx.walk(&q);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.record.memory_id == "gpu"));
        assert!(hits.iter().any(|h| h.record.memory_id == "cargo"));
    }

    #[test]
    fn walk_bypath_prefix_and_fnmatch_fire_but_broad_double_star_never() {
        let idx = Index::from_records(vec![
            rec(Axis::Path, "/etc/foo/**", Source::Tag, "prefix"), // trailing /** → fires
            rec(Axis::Path, "/etc/*.conf", Source::Memory, "fnmatch"), // plain fnmatch → fires
            rec(Axis::Path, "**/*.md", Source::Memory, "broad-mid"), // mid ** → broad, never
            rec(Axis::Path, "**", Source::Memory, "broad-bare"),   // bare ** → broad, never
            rec(
                Axis::Path,
                "~/**/settings.json",
                Source::Memory,
                "broad-lead",
            ), // lead ** → broad
        ]);
        let q = WalkQuery {
            paths: vec![
                "/etc/foo/bar.conf".into(),
                "/etc/a.conf".into(),
                "/repo/docs/readme.md".into(),
                "/home/u/.config/settings.json".into(),
            ],
            ..Default::default()
        };
        let fired: Vec<&str> = idx
            .walk(&q)
            .iter()
            .map(|h| h.record.memory_id.as_str())
            .collect();
        assert!(fired.contains(&"prefix"), "trailing /** must fire");
        assert!(fired.contains(&"fnmatch"), "plain fnmatch must fire");
        assert!(
            !fired.contains(&"broad-mid"),
            "**/*.md is broad — must NOT fire"
        );
        assert!(
            !fired.contains(&"broad-bare"),
            "** is broad — must NOT fire"
        );
        assert!(
            !fired.contains(&"broad-lead"),
            "~/**/settings.json is broad — must NOT fire"
        );

        // Prefix boundary: `/etc/foo/**` must NOT match `/etc/foobar` (needs `/`).
        let q2 = WalkQuery {
            paths: vec!["/etc/foobar".into()],
            ..Default::default()
        };
        assert!(idx.walk(&q2).iter().all(|h| h.record.memory_id != "prefix"));
    }

    #[test]
    fn walk_bypath_anchor_only_globs_never_fire() {
        // Walk-back fix F3 (2026-07-04): the anchor-only broad class — `/**` and
        // `~/**` are trailing-`/**` forms whose PREFIX is an anchor, so the
        // containment branch alone would match every path; `/*` and `~/*` reach
        // fnmatch where `*` crosses `/`. §3.x classifies all four broad; the
        // walk must agree with the liveness classifier and never route them.
        let idx = Index::from_records(vec![
            rec(Axis::Path, "/**", Source::Tag, "root-recursive"),
            rec(Axis::Path, "~/**", Source::Tag, "home-recursive"),
            rec(Axis::Path, "/*", Source::Memory, "root-star"),
            rec(Axis::Path, "~/*", Source::Memory, "home-star"),
            // GOOD contrast: a concrete prefix behind the trailing /** still fires.
            rec(Axis::Path, "~/.config/gpu/**", Source::Tag, "concrete"),
        ]);
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/u".into());
        let q = WalkQuery {
            paths: vec![
                "/etc/anything".into(),
                format!("{home}/some/file.txt"),
                format!("{home}/.config/gpu/nv.conf"),
            ],
            ..Default::default()
        };
        let fired: Vec<&str> = idx
            .walk(&q)
            .iter()
            .map(|h| h.record.memory_id.as_str())
            .collect();
        for broad in ["root-recursive", "home-recursive", "root-star", "home-star"] {
            assert!(
                !fired.contains(&broad),
                "{broad} is an anchor-only broad glob — must NOT fire"
            );
        }
        assert!(
            fired.contains(&"concrete"),
            "a concrete-prefix trailing /** must still fire"
        );
    }

    #[test]
    fn membership_accessors() {
        let idx = Index::from_records(vec![
            rec(Axis::Arg, "release", Source::Memory, "a"),
            rec(Axis::Synonym, "grep", Source::Tag, "rg"),
        ]);
        assert!(idx.contains_arg_key("RELEASE"));
        assert!(!idx.contains_arg_key("grep"));
        assert!(idx.contains_synonym_key("grep"));
        assert!(!idx.contains_synonym_key("--bare"));
        let keys = idx.arg_or_synonym_key_set();
        assert!(keys.contains("release") && keys.contains("grep"));
    }

    #[test]
    fn fnmatch_semantics() {
        let m = |p: &str, t: &str| {
            fnmatch(
                &p.chars().collect::<Vec<_>>(),
                &t.chars().collect::<Vec<_>>(),
            )
        };
        assert!(m("*.md", "readme.md"));
        assert!(m("*.md", "/a/b/c.md")); // `*` crosses '/', unlike a path-glob crate
        assert!(!m("*.md", "readme.txt"));
        assert!(m("a?c", "abc"));
        assert!(!m("a?c", "ac"));
        assert!(m("[a-c]x", "bx"));
        assert!(!m("[a-c]x", "dx"));
        assert!(m("[!a-c]x", "dx"));
        assert!(m("*.tmp", "build.tmp"));
        // Class negation parity with Python `fnmatch`: ONLY `!` negates; `^` is a
        // literal member.
        assert!(
            !m("[^a]", "b"),
            "`^` is literal: {{^,a}} does not contain b"
        );
        assert!(m("[^a]", "a"), "`^` is literal: `a` is a member");
        assert!(m("[^a]", "^"), "`^` is literal: `^` is a member");
        assert!(m("[!a]", "b"), "`!` negates: b != a");
        assert!(!m("[!a]", "a"), "`!` negates: a == a");
    }

    #[test]
    fn bypath_double_star_prefix_is_literal() {
        // `~/.config/{nvim,vim}/**` -> tilde expands, `/**` suffix -> literal
        // prefix containment (braces are literal, matching synapse).
        let home = std::env::var("HOME").unwrap();
        let idx = Index::from_records(vec![rec(
            Axis::Path,
            "~/.config/{nvim,vim}/**",
            Source::Memory,
            "cfg",
        )]);
        let q = WalkQuery {
            paths: vec![format!("{home}/.config/{{nvim,vim}}/init.lua")],
            ..Default::default()
        };
        assert_eq!(idx.walk(&q).len(), 1);
    }
}
