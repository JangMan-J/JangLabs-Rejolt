//! The JSON catalog report, the artifact-pair binding (generation id +
//! sourceFingerprint), and the **single reader** (plan P4; D2, D14, D24, A2, §4).
//!
//! Two artifacts are written per rebuild: the flat index (TSV,
//! [`crate::index`]) and this JSON catalog report. Per A2(b) the report carries
//! **NO routing tables** — only write-side metadata: the memories (tags +
//! description) the dedup path needs, the [`RoutabilityReport`], the
//! `sourceFingerprint`, and the rendered vocab digest. Both artifacts carry an
//! **identical generation id + sourceFingerprint** (A2d).
//!
//! ## Generation id and fingerprint (A2d, §4)
//!
//! - `sourceFingerprint` hashes the grammar text alone (§4: staleness vs the
//!   routing vocabulary is mechanically detectable).
//! - The **generation id** hashes the schema version + grammar text + the sorted
//!   store file contents. It is deterministic on inputs, so a re-run on
//!   unchanged inputs reproduces it (idempotence, D2/P14); yet any input change
//!   yields a different id, so a torn rebuild — index (new id) written, crash
//!   before the report (old id) — leaves a **cross-artifact mismatch** the reader
//!   detects.
//!
//! The hash is FNV-1a/64 (deterministic, stable, zero-dependency). It is a
//! change-detector, not a security primitive; a legitimate input change differs
//! with overwhelming probability, which is all torn-pair detection needs.
//!
//! ## The single reader (§4, A2d)
//!
//! [`read_artifacts`] is the only reader. It **fails open**, always: a missing
//! pair, a malformed-but-parseable artifact, or a generation mismatch all yield
//! a non-`Consistent` outcome carrying an optional one-line advisory — never a
//! hard error. A generation mismatch is a stale pair (one advisory); a malformed
//! artifact loads as absent (`None`-equivalent), per §4's single-reader rule.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::index::{Index, IndexRecord};
use crate::tier::SCHEMA_VERSION;

// =============================================================================
// FNV-1a/64 — the deterministic change-detector behind gen id + fingerprint
// =============================================================================

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A small FNV-1a/64 accumulator that folds **length-prefixed** fields, so the
/// pre-image is injective — two distinct field sequences can never produce the
/// same byte stream regardless of the field contents.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(FNV_OFFSET)
    }
    fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }
    /// Fold one field as `len (u64 LE)` then its bytes. A length prefix (not a
    /// separator byte) is what keeps framing injective even when the content
    /// itself contains any byte — including NUL, which `read_to_string` accepts.
    fn field(&mut self, bytes: &[u8]) {
        self.update(&(bytes.len() as u64).to_le_bytes());
        self.update(bytes);
    }
    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

/// `sourceFingerprint` — hash of the grammar text alone (§4).
pub fn source_fingerprint(grammar_text: &str) -> String {
    let mut h = Fnv::new();
    h.update(grammar_text.as_bytes());
    h.hex()
}

/// The rebuild **generation id**: a deterministic hash of the schema version,
/// the build-config tag, the grammar text, and the sorted `(filename, content)`
/// of every store file scanned. Deterministic on inputs (idempotent re-runs)
/// yet input-sensitive (torn pairs detectable). `files` need not be pre-sorted
/// — this sorts them.
///
/// `config_tag` folds in every build-config value that shapes artifact bytes
/// (walk-back fix F12, 2026-07-04): without it, a rebuild that crashed between
/// the index and report writes AFTER a config change left a mixed-config pair
/// under EQUAL generation ids — a false-Consistent the A2(d) stale-pair
/// detector exists to catch.
pub fn generation_id(grammar_text: &str, files: &[(String, String)], config_tag: &str) -> String {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Fnv::new();
    h.update(&SCHEMA_VERSION.to_le_bytes()); // fixed-size, always first
    h.field(config_tag.as_bytes());
    h.field(grammar_text.as_bytes());
    h.update(&(sorted.len() as u64).to_le_bytes()); // frame the file count
    for (name, content) in sorted {
        h.field(name.as_bytes());
        h.field(content.as_bytes());
    }
    h.hex()
}

// =============================================================================
// The flat-index metadata header (carries gen id + fingerprint in the index)
// =============================================================================

/// The magic prefix of the index file's first line — a `#` comment, so it is not
/// a record (records start with a table name) and the loader skips it.
const HEADER_MAGIC: &str = "# rejolt-flat-index";

/// The metadata header the flat index carries on its first line, so the index —
/// not just the report — carries the generation id + fingerprint (A2d).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexHeader {
    /// The rebuild generation id (must equal the report's).
    pub generation: String,
    /// The grammar fingerprint (must equal the report's).
    pub source_fingerprint: String,
    /// The schema version this index was written under.
    pub schema_version: u32,
}

impl IndexHeader {
    /// Emit the header as one `#`-comment line.
    pub fn emit(&self) -> String {
        format!(
            "{HEADER_MAGIC}\tgeneration={}\tsourceFingerprint={}\tschemaVersion={}",
            self.generation, self.source_fingerprint, self.schema_version
        )
    }

    /// Parse the header line. Returns `None` if the line is not a well-formed
    /// header (→ the reader treats the index as malformed / absent).
    pub fn parse(line: &str) -> Option<IndexHeader> {
        let rest = line.strip_prefix(HEADER_MAGIC)?;
        let (mut generation, mut fingerprint, mut schema) = (None, None, None);
        for field in rest.split('\t').filter(|s| !s.is_empty()) {
            let (k, v) = field.split_once('=')?;
            match k {
                "generation" => generation = Some(v.to_string()),
                "sourceFingerprint" => fingerprint = Some(v.to_string()),
                "schemaVersion" => schema = v.parse::<u32>().ok(),
                _ => {} // tolerate unknown header fields (forward-compatible)
            }
        }
        Some(IndexHeader {
            generation: generation?,
            source_fingerprint: fingerprint?,
            schema_version: schema?,
        })
    }
}

// =============================================================================
// The JSON catalog report (write-side metadata only — NO routing tables, A2b)
// =============================================================================

/// The JSON catalog report. **Carries no routing tables** (A2b) — only write-side
/// metadata. Serialized with camelCase keys for jq-inspectability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogReport {
    /// The flat-index schema version this pair was written under.
    pub schema_version: u32,
    /// The rebuild generation id (must equal the index header's).
    pub generation: String,
    /// The grammar fingerprint (must equal the index header's).
    pub source_fingerprint: String,
    /// Every parsed memory with the fields the write-side dedup path needs
    /// (tags + description). Routing tables are deliberately absent.
    pub memories: Vec<MemorySummary>,
    /// Unroutable memories + build-time exclusions (advisory-only, D18).
    pub routability_report: RoutabilityReport,
    /// The vocabulary digest, rendered from the parsed grammar (D23).
    pub vocab_digest: String,
    /// Store `.md` files that failed to parse as frontmatter (skipped, advisory).
    pub malformed_files: Vec<String>,
}

/// One memory's write-side metadata (dedup input). No routing evidence here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySummary {
    /// The memory id (file stem).
    pub id: String,
    /// The memory file path.
    pub path: String,
    /// The memory's tags.
    pub tags: Vec<String>,
    /// The memory's description (raw; the dedup bag-of-words input).
    pub description: String,
}

/// The routability report (§4, D18): unroutable memories and build-time
/// exclusions. Advisory-only; never a build failure.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutabilityReport {
    /// How many parsed memories ended with zero routing rows.
    pub unroutable_count: usize,
    /// The ids of those memories, sorted.
    pub unroutable_ids: Vec<String>,
    /// Entries excluded at build because a routing-critical field held a
    /// tab/newline/CR (A2e).
    pub excluded_entries: Vec<ExcludedEntry>,
}

/// One routing entry excluded at build for control-char content (A2e).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedEntry {
    /// The memory the excluded entry belonged to.
    pub memory_id: String,
    /// The table it would have joined.
    pub table: String,
    /// Why it was excluded.
    pub reason: String,
}

impl CatalogReport {
    /// Serialize to pretty JSON (jq-inspectable), newline-terminated.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self)
            .expect("CatalogReport serializes (only owned std types)");
        s.push('\n');
        s
    }

    /// Parse from JSON. `None` on unparseable/malformed JSON → the single reader
    /// fails open (§4).
    pub fn from_json(text: &str) -> Option<CatalogReport> {
        serde_json::from_str(text).ok()
    }
}

// =============================================================================
// The single reader (§4, A2d) — fail-open, torn-pair aware
// =============================================================================

/// A fully loaded, generation-consistent artifact pair.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The routing index (records only; header consumed).
    pub index: Index,
    /// The index header (generation + fingerprint).
    pub header: IndexHeader,
    /// The catalog report.
    pub report: CatalogReport,
}

/// The outcome of the single reader. Every non-`Consistent` variant is a
/// **fail-open** result: the consumer proceeds as if there were no index
/// (recall → silence, write guard → static gate), surfacing `advisory()` if any.
#[derive(Debug, Clone)]
pub enum ArtifactRead {
    /// Both artifacts present, parseable, and generation-consistent.
    Consistent(Box<Loaded>),
    /// A stale / torn pair: the index and report disagree on the generation id.
    /// One advisory; fail open (A2d, RB4).
    Stale(String),
    /// One or both artifacts absent — normal before the first rebuild. Silent
    /// fail-open (no advisory).
    Missing,
    /// A present-but-malformed artifact. Loads as absent (§4); one advisory.
    Malformed(String),
}

impl ArtifactRead {
    /// The consistent pair, if any.
    pub fn loaded(&self) -> Option<&Loaded> {
        match self {
            ArtifactRead::Consistent(l) => Some(l),
            _ => None,
        }
    }

    /// The one-line advisory to surface (session-start / rebuild), if any.
    pub fn advisory(&self) -> Option<&str> {
        match self {
            ArtifactRead::Stale(s) | ArtifactRead::Malformed(s) => Some(s),
            _ => None,
        }
    }
}

/// The **single reader** (§4). Reads the flat index + catalog report, fails open
/// on every fault, and detects a generation mismatch as a stale pair.
pub fn read_artifacts(index_path: &Path, report_path: &Path) -> ArtifactRead {
    let (Ok(index_text), Ok(report_text)) = (
        std::fs::read_to_string(index_path),
        std::fs::read_to_string(report_path),
    ) else {
        // A missing artifact is normal pre-rebuild: fail open, silent.
        return ArtifactRead::Missing;
    };

    let mut lines = index_text.lines();
    let Some(header) = lines.next().and_then(IndexHeader::parse) else {
        return ArtifactRead::Malformed(
            "flat index has no valid metadata header; ignoring index (fail-open) — re-run `rejolt rebuild`".into(),
        );
    };

    let mut records = Vec::new();
    for line in lines {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match IndexRecord::parse(line) {
            Ok(r) => records.push(r),
            Err(e) => {
                return ArtifactRead::Malformed(format!(
                    "flat index has a malformed record ({e}); ignoring index (fail-open) — re-run `rejolt rebuild`"
                ));
            }
        }
    }

    let Some(report) = CatalogReport::from_json(&report_text) else {
        return ArtifactRead::Malformed(
            "catalog report is malformed JSON; ignoring index (fail-open) — re-run `rejolt rebuild`".into(),
        );
    };

    if header.generation != report.generation {
        return ArtifactRead::Stale(format!(
            "stale artifact pair: flat index generation {} != catalog report generation {}; \
             recall proceeds index-free (fail-open) — re-run `rejolt rebuild`",
            header.generation, report.generation
        ));
    }

    ArtifactRead::Consistent(Box::new(Loaded {
        index: Index::from_records(records),
        header,
        report,
    }))
}

// =============================================================================
// Atomic write (D14): write to a same-dir temp, then rename over the target
// =============================================================================

/// Write `contents` to `path` **durably and atomically** (D14): write a sibling
/// temp file, `fsync` it, `rename` it over the target, then `fsync` the parent
/// directory. So neither a process crash NOR power loss leaves a half-written
/// artifact — the target is either the old bytes or the new bytes, never a
/// partial or a zero-length file (which ext4 delayed-alloc can otherwise surface
/// after a rename without the temp fsync). The temp lives in the target's
/// directory so the rename stays on one filesystem (atomic). On any failure the
/// target is untouched and no partial target is created; a stray temp may remain
/// but is never read.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".into());
    // Unique-enough temp name (pid + a process-lifetime counter).
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp: PathBuf = dir.join(format!(".{file_name}.tmp.{}.{n}", std::process::id()));

    // Write + fsync the temp file: durable on disk BEFORE the rename.
    let write_then_sync = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        Ok(())
    };
    if let Err(e) = write_then_sync() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // fsync the parent directory so the rename itself survives power loss.
    // Best-effort: some filesystems reject directory fsync — a failure here does
    // not un-do the atomic rename, so it must not fail the write.
    if let Ok(dir_file) = std::fs::File::open(dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_and_generation_are_deterministic_and_input_sensitive() {
        let files = vec![("a.md".to_string(), "A".to_string())];
        let g1 = generation_id("gram=1", &files, "cfg=1");
        let g2 = generation_id("gram=1", &files, "cfg=1");
        assert_eq!(g1, g2, "same inputs → same generation (idempotence)");

        // A grammar change moves the generation.
        assert_ne!(g1, generation_id("gram=2", &files, "cfg=1"));
        // A store change moves the generation.
        let files2 = vec![("a.md".to_string(), "B".to_string())];
        assert_ne!(g1, generation_id("gram=1", &files2, "cfg=1"));
        // A build-config change moves the generation (F12: a crash between the
        // pair's two writes across a config change must read as a torn pair,
        // never a false-Consistent).
        assert_ne!(g1, generation_id("gram=1", &files, "cfg=2"));
        // File order does not matter (sorted internally).
        let files_ab = vec![
            ("a.md".to_string(), "A".to_string()),
            ("b.md".to_string(), "B".to_string()),
        ];
        let files_ba = vec![
            ("b.md".to_string(), "B".to_string()),
            ("a.md".to_string(), "A".to_string()),
        ];
        assert_eq!(
            generation_id("g", &files_ab, "c"),
            generation_id("g", &files_ba, "c")
        );

        assert_ne!(source_fingerprint("x"), source_fingerprint("y"));
    }

    #[test]
    fn header_round_trips_and_tolerates_unknown_fields() {
        let h = IndexHeader {
            generation: "deadbeef".into(),
            source_fingerprint: "cafef00d".into(),
            schema_version: SCHEMA_VERSION,
        };
        assert_eq!(IndexHeader::parse(&h.emit()), Some(h.clone()));
        // Unknown header fields are tolerated.
        let with_extra = format!("{}\tfutureField=1", h.emit());
        assert_eq!(IndexHeader::parse(&with_extra), Some(h));
        // A non-header line does not parse.
        assert_eq!(IndexHeader::parse("byCommand\trg\t..."), None);
    }

    #[test]
    fn report_round_trips_and_omits_routing_tables() {
        let report = CatalogReport {
            schema_version: SCHEMA_VERSION,
            generation: "g".into(),
            source_fingerprint: "f".into(),
            memories: vec![MemorySummary {
                id: "m".into(),
                path: "/s/m.md".into(),
                tags: vec!["t".into()],
                description: "d".into(),
            }],
            routability_report: RoutabilityReport::default(),
            vocab_digest: "# vocab\n".into(),
            malformed_files: vec![],
        };
        let json = report.to_json();
        assert_eq!(CatalogReport::from_json(&json), Some(report));
        // A2b: no routing tables anywhere in the report.
        for table in ["byCommand", "byPath", "byArg", "bySynonym", "triggerIndex"] {
            assert!(
                !json.contains(table),
                "report must carry no routing table `{table}`:\n{json}"
            );
        }
        assert!(CatalogReport::from_json("{ not json").is_none());
    }

    #[test]
    fn atomic_write_leaves_no_temp_and_no_partial_on_failure() {
        let dir = std::env::temp_dir().join(format!("rejolt-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("_flat_index.tsv");
        write_atomic(&target, "hello\n").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello\n");
        // No temp sibling left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "atomic write left a temp file");

        // Failure case: target dir does not exist → Err, no partial target.
        let bad = dir.join("nope").join("x.tsv");
        assert!(write_atomic(&bad, "data").is_err());
        assert!(
            !bad.exists(),
            "a failed atomic write must not create a partial target"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
