//! Marks + telemetry — the ONE code path (plan P11 / WP-2b; D25, A7, D7, R7, §8).
//!
//! This is the foundational primitive WP-3 (recall fire-append), WP-5 (read
//! signal + writability probe), and WP-6 (curation window read) all build on.
//! D25 mandates **one code path** for mark-write / mark-check / telemetry-append,
//! shared by recall, read-signal, the session marker, and curation — so all of
//! those live on [`Telemetry`], and there is no second mark or telemetry writer
//! anywhere.
//!
//! ## Marks (D25, §8)
//!
//! A dedup mark is an **empty, mtime-only file** under an injectable runtime root
//! (default `${XDG_RUNTIME_DIR:-~/.cache}/rejolt/`), named `m_<sanitized-id>`
//! ([`mark_filename`]). tmpfs → per-boot self-cleaning is the intended deployment,
//! so there is no cleanup code. A mark is LIVE iff its mtime is within
//! `dedupe_ttl_seconds` of now — and that TTL is read **only** from
//! [`crate::config::Config`] (the D25 single-source fix; there is no second
//! hardcoded TTL in this file).
//!
//! ## The correlation invariant (A7's CORRECTED wording — the contract)
//!
//! A fresh mark's presence IS the fire↔read correlation — no timestamp joins. And
//! **mark persistence gates fire logging** ([`Telemetry::log_fire`]): the fire
//! path writes the dedup mark(s) FIRST and appends the fire record ONLY if ≥1 mark
//! persisted at write time. So an unwritable mark dir yields ZERO-FIRE (never
//! demoted, D7's zero-fire floor), never a fired-but-unread record. A later mark
//! wipe (reboot/suspend inside the TTL window) can orphan an already-logged fire —
//! a stated, accepted bias (A7 / RB8), NOT mechanically fixed here (persisting
//! mark metadata would reintroduce the very join D25 rejected).
//!
//! ## Rotation + the window reader (§8, R7, WR-04/WR-05)
//!
//! At `tel_max_bytes` the live file rotates to one `.1` generation (atomic
//! `rename`, ~2× total). The window reader reads `.1` FIRST then the live file
//! (WR-04), drops any fire OR read record with an unparseable ts **symmetrically**
//! (WR-05), and applies the effective window `min(telemetry_window_days,
//! rotation bound ≈ 2×tel_max_bytes span)` — exposing which bound was hit
//! (R7, [`WindowBound`]).
//!
//! ## Testability
//!
//! The mark-dir and telemetry-file locations are **injected** into
//! [`Telemetry::new`] — never hardcoded to the real `$XDG_RUNTIME_DIR` — so tests
//! point them at temp dirs. [`Telemetry::for_store`] wires the real defaults for
//! the production path.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::Config;

/// `O_NOFOLLOW` (from `libc`, so the value is correct for the target arch — it is
/// NOT the same integer on every Linux port). Opening a mark or the telemetry file
/// with this flag makes "do not follow a final symlink" atomic AT open(), closing
/// the check-then-open TOCTOU that a separate `symlink_metadata` probe leaves.
const O_NOFOLLOW: i32 = libc::O_NOFOLLOW;

/// The engine name — the runtime mark-dir namespace component (`.../rejolt/`).
const ENGINE_NAME: &str = "rejolt";
/// The live telemetry file (infra: underscore-prefixed, so the store scan skips it).
pub const TELEMETRY_FILENAME: &str = "_recall_telemetry.jsonl";
/// The `signal` value on a read-confirmation record.
const SIGNAL_READ: &str = "read";
/// The `signal` value on a session-start marker record.
const SIGNAL_SESSION: &str = "session";
/// Seconds per day — the window arithmetic unit (§8 rectangular day window).
const SECONDS_PER_DAY: i64 = 86_400;

// =============================================================================
// Record shapes (§8, serde) — ts is a unix timestamp (seconds)
// =============================================================================

/// One fired memory inside a [`FireRecord`] — `{id, tag, type, val}` (§8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FireMem {
    /// The fired memory id.
    pub id: String,
    /// The grammar route tag (or memory id) the fire cited.
    pub tag: String,
    /// The trigger axis (`command` / `path` / `arg` / `synonym`). `type` is a Rust
    /// keyword, so the field is `trigger_type` and serialized as `type`.
    #[serde(rename = "type")]
    pub trigger_type: String,
    /// The matched value.
    pub val: String,
}

/// A recall **fire** telemetry record: `{ts, qid, mems:[…], conf}` (§8). Appended
/// by [`Telemetry::log_fire`] once mark persistence is confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FireRecord {
    /// Unix timestamp (seconds).
    pub ts: i64,
    /// The query id (one per emission) — the fire-record discriminator (`qid`).
    pub qid: String,
    /// The fired memory set.
    pub mems: Vec<FireMem>,
    /// The confidence label.
    pub conf: String,
}

/// A **read** confirmation record: `{ts, id, signal:"read"}` (§8). Appended by
/// [`Telemetry::log_read`] only when the read targets a memory with a LIVE mark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRecord {
    /// Unix timestamp (seconds).
    pub ts: i64,
    /// The read memory id.
    pub id: String,
    /// Always `"read"` — the read-record discriminator.
    pub signal: String,
}

/// A session-start **marker** record: `{ts, signal:"session"}` (§8). Appended by
/// [`Telemetry::log_session`] — the fourth D25 consumer of the one append path.
/// WP-6 min-evidence (session-day) counting consumes these; WP-2b just writes them
/// through the shared path so the "one code path" contract is real.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Unix timestamp (seconds).
    pub ts: i64,
    /// Always `"session"`.
    pub signal: String,
}

// =============================================================================
// Outcomes (so the caller / tests can see which contract arm fired)
// =============================================================================

/// The result of [`Telemetry::log_fire`]. Every variant is non-blocking (recall is
/// never blocked by telemetry, §8) — they only report what was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireOutcome {
    /// ≥1 mark persisted AND the fire record was appended.
    Logged,
    /// The dedup mark(s) did not persist (e.g. an unwritable mark dir): the fire is
    /// NOT logged — ZERO-FIRE, never demoted (D7). The A7 correlation gate firing.
    ZeroFire,
    /// Marks persisted but the telemetry append itself faulted (EACCES/ENOSPC/a
    /// symlinked telemetry path): fail-open — the fire is simply unrecorded, recall
    /// proceeds regardless.
    AppendFailed,
}

/// The result of [`Telemetry::log_read`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    /// A live mark existed AND the read record was appended.
    Logged,
    /// No live mark for the memory ⇒ no read recorded (the mark IS the
    /// correlation; without one, a read is unobservable, D25).
    NoLiveMark,
    /// A live mark existed but the append faulted — fail-open (unrecorded).
    AppendFailed,
}

/// Which cap bounded the effective curation window (R7). Exposed by
/// [`Telemetry::read_window`] so curation (WP-6) knows whether it is looking at a
/// full 30 days or a rotation-shortened slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowBound {
    /// `telemetry_window_days` (the 30-day time window) was the binding cap.
    TimeCapped,
    /// The rotation bound (≈2×`tel_max_bytes` span) was tighter than the time
    /// window — telemetry was rotated out before the window's far edge.
    RotationCapped,
}

/// The windowed telemetry curation (WP-6) reads. Records are in **read order**
/// (`.1` generation first, then live — WR-04); they are NOT re-sorted.
#[derive(Debug, Clone)]
pub struct WindowedTelemetry {
    /// The in-window fire records, `.1`-first.
    pub fires: Vec<FireRecord>,
    /// The in-window read records, `.1`-first.
    pub reads: Vec<ReadRecord>,
    /// The in-window session-start markers, `.1`-first — the input WP-6 curation
    /// floor-2 (§8: ≥10 distinct session-days OR ≥30 days span) consumes. Surfaced
    /// here (not discarded) so that floor has a path without re-freezing this
    /// reader.
    pub sessions: Vec<SessionRecord>,
    /// Which cap bounded the window (R7).
    pub bound: WindowBound,
    /// How many fire / read / session records were dropped for an unparseable ts
    /// (WR-05 symmetric drop; a bad-ts session cannot be assigned a day either).
    pub dropped_bad_ts: usize,
}

// =============================================================================
// mark-filename mapping + runtime-dir/telemetry-path resolution
// =============================================================================

/// Map a memory id to its mark filename `m_<encoded-id>` (D25) — an **injective**
/// percent-encoding. Every UTF-8 byte outside the safe class `[A-Za-z0-9.-]`
/// (notably `/`, whitespace, control chars, AND `_` and `%` themselves) becomes
/// `%XX` (uppercase hex). The result is always a single safe filename with no path
/// separator, and — because `%` is itself escaped — the mapping is reversible, so
/// **distinct ids never share a mark file**.
///
/// Injectivity is a correlation-correctness requirement, not just tidiness (the
/// earlier collapse-to-`_` mapping was non-injective): the fire↔read correlation
/// is PER-MEMORY (D25). Two distinct legal store stems `gpu notes` and `gpu_notes`
/// (a space passes the rebuild control-char filter) must NOT hash to one mark, or a
/// read of one would be mis-credited to the other and let a fired memory be
/// recorded fired-but-unread (D7's zero-fire floor does not shield a memory that
/// has fires). Recall and read-signal both call this, so they always agree.
pub fn mark_filename(memory_id: &str) -> String {
    let mut s = String::with_capacity(memory_id.len() + 2);
    s.push_str("m_");
    for &b in memory_id.as_bytes() {
        if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' {
            s.push(b as char);
        } else {
            s.push('%');
            s.push(hex_upper_nibble(b >> 4));
            s.push(hex_upper_nibble(b & 0x0f));
        }
    }
    s
}

/// One uppercase-hex digit for a nibble (`0..=15`).
fn hex_upper_nibble(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

/// The default runtime mark-dir: `${XDG_RUNTIME_DIR:-~/.cache}/rejolt/`. Used by
/// [`Telemetry::for_store`] on the production path ONLY; tests inject a temp dir
/// via [`Telemetry::new`] and never touch this.
pub fn default_runtime_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from(".cache"));
    base.join(ENGINE_NAME)
}

/// The telemetry file path under a store: `<store>/_recall_telemetry.jsonl`.
pub fn telemetry_path(store_dir: &Path) -> PathBuf {
    store_dir.join(TELEMETRY_FILENAME)
}

/// The one-generation rotation path for a telemetry file: `<tel>.1`.
fn rotation_path(tel: &Path) -> PathBuf {
    let mut s = tel.as_os_str().to_os_string();
    s.push(".1");
    PathBuf::from(s)
}

/// Now, as a unix timestamp (seconds). Clock-before-epoch → 0 (never panics).
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// =============================================================================
// Telemetry — the one code path (marks + telemetry)
// =============================================================================

/// The single owner of mark-write, mark-check, and telemetry-append (D25). All of
/// recall (WP-3), read-signal (WP-5), the session marker, and curation (WP-6) go
/// through this one type. Locations are injected (testability); the TTL / rotation
/// / window tunables come from [`Config`] and nowhere else.
#[derive(Debug, Clone)]
pub struct Telemetry {
    /// The runtime mark-dir root (injectable). Marks are `m_*` files under here.
    runtime_dir: PathBuf,
    /// The live telemetry file (injectable). `.1` is its rotation generation.
    tel_path: PathBuf,
    /// The single source of TTL / rotation / window tunables (D25).
    config: Config,
}

impl Telemetry {
    /// Construct with **injected** locations (tests point these at temp dirs).
    pub fn new(runtime_dir: PathBuf, tel_path: PathBuf, config: Config) -> Self {
        Telemetry {
            runtime_dir,
            tel_path,
            config,
        }
    }

    /// The production wiring: the default runtime mark-dir + the telemetry file
    /// under `store_dir`. (Tests use [`Telemetry::new`] with temp dirs instead.)
    pub fn for_store(store_dir: &Path, config: Config) -> Self {
        Telemetry::new(default_runtime_dir(), telemetry_path(store_dir), config)
    }

    /// The runtime mark-dir root (diagnostics / advisory text).
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// The live telemetry file path (diagnostics / tests).
    pub fn telemetry_file(&self) -> &Path {
        &self.tel_path
    }

    /// The config this primitive carries (D25 single source). Recall reads its
    /// tier weights + confidence thresholds from here (WP-7), so the ranking
    /// magnitudes flow in through the same injected [`Config`] as the telemetry
    /// tunables — one source, no second hardcoded copy.
    pub fn config(&self) -> &Config {
        &self.config
    }

    // ---- marks -----------------------------------------------------------

    /// Is a memory's dedup mark LIVE (mtime within `dedupe_ttl_seconds` of now)?
    /// A missing mark, a symlinked mark path (refused, no-follow), or any stat
    /// fault ⇒ not live (fail-open). The single-id mark-check.
    pub fn is_live(&self, memory_id: &str) -> bool {
        let path = self.runtime_dir.join(mark_filename(memory_id));
        // No-symlink-follow: `symlink_metadata` does NOT resolve a final symlink;
        // we refuse a symlinked mark rather than reading through it (D25).
        match fs::symlink_metadata(&path) {
            Ok(meta) if !meta.file_type().is_symlink() => self.mark_is_fresh(&meta),
            _ => false,
        }
    }

    /// The set of memory ids (from `ids`) whose marks are LIVE. Marks are
    /// content-free empty files whose name is a lossy hash of the id, so liveness
    /// is queried against a KNOWN candidate set (recall/read/curation always hold
    /// the ids) rather than reversed out of the directory.
    pub fn live_marks(&self, ids: &[String]) -> BTreeSet<String> {
        ids.iter().filter(|id| self.is_live(id)).cloned().collect()
    }

    /// Is a mark fresh under the config TTL? This is the ONLY reader of the TTL —
    /// the D25 single-source point. A future mtime (clock skew) counts as fresh
    /// (fail toward suppressing a re-fire, never toward a spurious demotion).
    fn mark_is_fresh(&self, meta: &fs::Metadata) -> bool {
        let Ok(mtime) = meta.modified() else {
            return false;
        };
        match SystemTime::now().duration_since(mtime) {
            // `<=` (inclusive) honors D25's "within `dedupe_ttl_seconds`"; §8 phrases
            // it as "<15min". The inclusive boundary is the safe anti-deflation
            // direction: a mark exactly at the TTL still suppresses a re-fire and
            // still credits a read — never a spurious demotion (A7/RB8 bias floor).
            Ok(age) => age.as_secs() <= self.config.dedupe_ttl_seconds,
            Err(_) => true,
        }
    }

    /// Is the runtime mark-dir SAFE to write into? The single hardening gate BOTH
    /// the write path ([`Telemetry::write_mark`]) and the advisory
    /// ([`Telemetry::mark_dir_writable`]) share — so `log_fire` never silently
    /// writes into a dir the advisory calls inert. Creates the dir (0o700) then
    /// requires it to be: a real directory (not a symlink), owned by our euid, and
    /// NOT group- or world-writable. Every fault is fail-open (`false`).
    ///
    /// The owner + perms check matters when `XDG_RUNTIME_DIR` is unset (cron /
    /// containers / ssh-without-pam) and we fall back to `~/.cache/rejolt`, which a
    /// prior umask may have left 0o755 — a co-resident user could then read/plant
    /// marks. Such a dir is treated as unsafe (inert), not written into.
    fn runtime_dir_safe(&self) -> bool {
        // 0o700 on CREATE: a mark dir we create is private to us. An existing dir is
        // left as-is (we do not chmod someone else's dir) and vetted below instead.
        let _ = fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.runtime_dir);
        let Ok(meta) = fs::symlink_metadata(&self.runtime_dir) else {
            return false;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return false;
        }
        // Owned by us (synapse's `-O`); euid from libc so it is the real caller.
        let euid = unsafe { libc::geteuid() };
        if meta.uid() != euid {
            return false;
        }
        // Not group- or world-writable (a shared dir where marks could be tampered).
        (meta.mode() & 0o022) == 0
    }

    /// Atomically create/refresh a memory's empty mark file with mtime = now.
    /// Returns whether the mark persisted. Gated on [`Telemetry::runtime_dir_safe`]
    /// (matching the advisory) and opened with `O_NOFOLLOW`, so a swapped symlink at
    /// the mark path is refused atomically at open() rather than followed and its
    /// target truncated. `set_modified` is explicit because O_TRUNC on an
    /// already-empty file is not guaranteed to bump mtime — freshness is the point.
    fn write_mark(&self, memory_id: &str) -> bool {
        if !self.runtime_dir_safe() {
            return false;
        }
        let path = self.runtime_dir.join(mark_filename(memory_id));
        match OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(O_NOFOLLOW)
            .open(&path)
        {
            Ok(f) => f.set_modified(SystemTime::now()).is_ok(),
            Err(_) => false,
        }
    }

    /// The writability probe (A7): is the mark dir SAFE ([`Telemetry::runtime_dir_safe`]:
    /// owned by us, 0o700-ish, not a symlink) AND can we actually create a file in
    /// it (e.g. not a read-only fs)? Used by bootstrap/maintain to detect
    /// structurally-inert telemetry. Because the write path gates on the same
    /// `runtime_dir_safe`, a `false` here means `log_fire` also declines (ZeroFire) —
    /// the advisory and the write path never disagree.
    pub fn mark_dir_writable(&self) -> bool {
        if !self.runtime_dir_safe() {
            return false;
        }
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let probe = self
            .runtime_dir
            .join(format!(".probe.{}.{n}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(O_NOFOLLOW)
            .open(&probe)
        {
            Ok(_) => {
                let _ = fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        }
    }

    /// The A7 advisory: `Some(line)` when the runtime mark dir is not writable —
    /// telemetry is structurally inert (fires would be zero-fire forever, curation
    /// cannot correlate reads), so bootstrap/maintain emit exactly ONE advisory.
    /// `None` when writable. Recall itself is unaffected either way.
    pub fn inert_telemetry_advisory(&self) -> Option<String> {
        if self.mark_dir_writable() {
            None
        } else {
            Some(format!(
                "telemetry inert: runtime mark dir {} is not writable — self-curation \
                 cannot correlate reads (fires stay zero-fire, never demoted); recall is \
                 unaffected. Check {{XDG_RUNTIME_DIR}}/{ENGINE_NAME} ownership/permissions.",
                self.runtime_dir.display()
            ))
        }
    }

    // ---- telemetry append (the shared primitive) -------------------------

    /// Append one JSON record to the live telemetry file, rotating first if needed.
    /// FAIL-OPEN: a symlinked telemetry path is refused (`O_NOFOLLOW`) and every I/O
    /// fault is swallowed (returns `false`) — a telemetry fault never blocks recall
    /// (§8). This is the ONE append path all record kinds use.
    ///
    /// The record AND its trailing newline are written in ONE `write_all` over an
    /// `O_APPEND` fd. On Linux a single `write()` under `O_APPEND` is atomic against
    /// concurrent appenders, and a whole record is well under `PIPE_BUF`, so two
    /// parallel recall hooks (Claude Code batches parallel tool calls → parallel
    /// fires are ROUTINE) can never interleave at the newline into `{A}{B}\n\n`,
    /// which the reader would parse as invalid JSON and drop BOTH records silently.
    fn append_line(&self, line: &str) -> bool {
        self.maybe_rotate();
        let mut buf = Vec::with_capacity(line.len() + 1);
        buf.extend_from_slice(line.as_bytes());
        buf.push(b'\n');
        OpenOptions::new()
            .create(true)
            .append(true)
            .custom_flags(O_NOFOLLOW)
            .open(&self.tel_path)
            .and_then(|mut f| f.write_all(&buf))
            .is_ok()
    }

    /// Rotate the live telemetry file to its one `.1` generation when it has
    /// reached `tel_max_bytes` (§8). Atomic `rename` on one filesystem; best-effort
    /// and fail-open (a losing racer gets ENOENT, harmlessly). No live file yet ⇒
    /// nothing to rotate.
    fn maybe_rotate(&self) {
        let Ok(meta) = fs::metadata(&self.tel_path) else {
            return;
        };
        if meta.len() >= self.config.tel_max_bytes {
            let _ = fs::rename(&self.tel_path, rotation_path(&self.tel_path));
        }
    }

    // ---- the gated writers (recall / read-signal / session marker) -------

    /// Log a recall fire under the A7/D25 correlation gate — the ONE place the
    /// invariant is realized (WP-3 recall just calls this):
    ///
    /// 1. write the dedup mark for EACH memory the record credits (`record.mems`);
    /// 2. append `record` ONLY if ≥1 of those marks persisted at write time.
    ///
    /// The marked set is derived from `record.mems` — not a separate argument — so
    /// the invariant is structural: a memory can never be credited as fired in the
    /// record without its mark being the one written and gated on (a caller can no
    /// longer pass a `fired_ids` that diverges from `mems` and log a WRITE-time
    /// fired-but-unread). Empty `record.mems` credits nobody ⇒ [`FireOutcome::ZeroFire`]
    /// (nothing to log). An unwritable/unsafe mark dir persists no mark ⇒ ZeroFire:
    /// the fire is never logged, so every credited memory stays zero-fire (never
    /// demoted, D7) rather than fired-but-unread. The append itself is fail-open.
    pub fn log_fire(&self, record: &FireRecord) -> FireOutcome {
        if record.mems.is_empty() {
            return FireOutcome::ZeroFire;
        }
        let mut any_mark = false;
        for m in &record.mems {
            if self.write_mark(&m.id) {
                any_mark = true;
            }
        }
        if !any_mark {
            return FireOutcome::ZeroFire;
        }
        let Ok(line) = serde_json::to_string(record) else {
            return FireOutcome::AppendFailed;
        };
        if self.append_line(&line) {
            FireOutcome::Logged
        } else {
            FireOutcome::AppendFailed
        }
    }

    /// Log a read confirmation for `memory_id` — appended ONLY when a LIVE mark
    /// exists (the mark's presence IS the fire↔read correlation, no timestamp
    /// join; D25). WP-5 read-signal calls this. Fail-open on append fault.
    pub fn log_read(&self, memory_id: &str) -> ReadOutcome {
        if !self.is_live(memory_id) {
            return ReadOutcome::NoLiveMark;
        }
        let rec = ReadRecord {
            ts: now_unix(),
            id: memory_id.to_string(),
            signal: SIGNAL_READ.to_string(),
        };
        let Ok(line) = serde_json::to_string(&rec) else {
            return ReadOutcome::AppendFailed;
        };
        if self.append_line(&line) {
            ReadOutcome::Logged
        } else {
            ReadOutcome::AppendFailed
        }
    }

    /// Append a session-start marker `{ts, signal:"session"}` through the shared
    /// path (the fourth D25 consumer). Returns whether it persisted (fail-open).
    pub fn log_session(&self) -> bool {
        let rec = SessionRecord {
            ts: now_unix(),
            signal: SIGNAL_SESSION.to_string(),
        };
        match serde_json::to_string(&rec) {
            Ok(line) => self.append_line(&line),
            Err(_) => false,
        }
    }

    // ---- the window reader (curation consumes this) ----------------------

    /// Read the windowed telemetry curation (WP-6) consumes (§8, WR-04/WR-05, R7):
    ///
    /// - reads the `.1` generation FIRST, then live (WR-04) — order preserved, not
    ///   re-sorted, so pre-rotation reads are not stranded behind fresh fires;
    /// - drops any fire OR read record with an unparseable ts SYMMETRICALLY (WR-05);
    /// - applies the effective window `min(telemetry_window_days, rotation bound)`
    ///   and reports which bound was hit ([`WindowBound`], R7).
    pub fn read_window(&self) -> WindowedTelemetry {
        // Pass 1: gather every physical line, `.1` first (WR-04), with its byte
        // length (rotation measures file bytes) and its parsed Value + ts. Read
        // BYTES and decode each line lossily: one invalid-UTF-8 byte (torn write,
        // tampering) must corrupt only its own line, never fail the whole generation
        // the way `read_to_string` would (which would silently drop every record).
        let mut raw: Vec<RawLine> = Vec::new();
        for path in [rotation_path(&self.tel_path), self.tel_path.clone()] {
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            for raw_line in bytes.split(|&b| b == b'\n') {
                if raw_line.is_empty() {
                    continue;
                }
                let byte_len = raw_line.len() + 1; // + the newline separator
                let line = String::from_utf8_lossy(raw_line);
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let value = serde_json::from_str::<serde_json::Value>(trimmed).ok();
                let ts = value
                    .as_ref()
                    .and_then(|v| v.get("ts"))
                    .and_then(serde_json::Value::as_i64);
                raw.push(RawLine {
                    byte_len,
                    value,
                    ts,
                });
            }
        }

        // The rotation bound (R7): the ts of the oldest record still inside the
        // most-recent 2×tel_max_bytes of file bytes. `None` ⇒ rotation not binding.
        let budget = self.config.tel_max_bytes.saturating_mul(2);
        let rotation_cutoff = rotation_cutoff_ts(&raw, budget);

        let time_cutoff = now_unix().saturating_sub(
            (self.config.telemetry_window_days as i64).saturating_mul(SECONDS_PER_DAY),
        );

        // Effective window = the tighter (more recent) of the two cutoffs; the bound
        // names which one won (R7).
        let (effective_cutoff, bound) = match rotation_cutoff {
            Some(rc) if rc > time_cutoff => (rc, WindowBound::RotationCapped),
            _ => (time_cutoff, WindowBound::TimeCapped),
        };

        // Pass 2: classify + window-filter, preserving `.1`-first read order.
        let mut fires = Vec::new();
        let mut reads = Vec::new();
        let mut sessions = Vec::new();
        let mut dropped_bad_ts = 0usize;
        for r in &raw {
            let Some(v) = &r.value else {
                continue; // non-JSON line: skip (not a bad-ts record)
            };
            match classify(v) {
                Class::Fire(ts) => match ts {
                    None => dropped_bad_ts += 1, // WR-05: symmetric drop
                    Some(ts) if ts >= effective_cutoff => {
                        if let Ok(rec) = serde_json::from_value::<FireRecord>(v.clone()) {
                            fires.push(rec);
                        }
                    }
                    Some(_) => {} // parseable but out of window
                },
                Class::Read(ts) => match ts {
                    None => dropped_bad_ts += 1, // WR-05: symmetric drop
                    Some(ts) if ts >= effective_cutoff => {
                        if let Ok(rec) = serde_json::from_value::<ReadRecord>(v.clone()) {
                            reads.push(rec);
                        }
                    }
                    Some(_) => {}
                },
                Class::Session(ts) => match ts {
                    None => dropped_bad_ts += 1, // bad-ts session: cannot be dated
                    Some(ts) if ts >= effective_cutoff => {
                        if let Ok(rec) = serde_json::from_value::<SessionRecord>(v.clone()) {
                            sessions.push(rec);
                        }
                    }
                    Some(_) => {}
                },
                Class::Unknown => {} // not a fire/read/session record
            }
        }

        WindowedTelemetry {
            fires,
            reads,
            sessions,
            bound,
            dropped_bad_ts,
        }
    }
}

// =============================================================================
// window-reader internals
// =============================================================================

/// One physical telemetry line, as pass 1 collected it.
struct RawLine {
    /// The line's on-disk byte length (incl. its newline) — rotation accounting.
    byte_len: usize,
    /// The parsed JSON, or `None` for a non-JSON (malformed) line.
    value: Option<serde_json::Value>,
    /// The record's `ts` as an integer, or `None` if missing / non-integer.
    ts: Option<i64>,
}

/// How the window reader classifies one parsed line. The `Option<i64>` is the ts,
/// `None` meaning an unparseable ts (a WR-05 symmetric drop for fires AND reads,
/// and equally for session markers, which then cannot be assigned a day).
enum Class {
    Fire(Option<i64>),
    Read(Option<i64>),
    Session(Option<i64>),
    Unknown,
}

/// Classify a parsed telemetry record: a fire has a `qid`; a read has
/// `signal == "read"`; a session marker has `signal == "session"`; anything else is
/// Unknown.
fn classify(v: &serde_json::Value) -> Class {
    let ts = v.get("ts").and_then(serde_json::Value::as_i64);
    if v.get("qid").is_some() {
        Class::Fire(ts)
    } else if v.get("signal").and_then(serde_json::Value::as_str) == Some(SIGNAL_READ) {
        Class::Read(ts)
    } else if v.get("signal").and_then(serde_json::Value::as_str) == Some(SIGNAL_SESSION) {
        Class::Session(ts)
    } else {
        Class::Unknown
    }
}

/// The rotation-bound cutoff ts (R7): the oldest record ts still inside the
/// most-recent `budget` (= 2×`tel_max_bytes`) file bytes. `None` when the total is
/// within budget (rotation retains everything, so it never binds the window) or
/// when the retained slice carries no parseable ts.
fn rotation_cutoff_ts(raw: &[RawLine], budget: u64) -> Option<i64> {
    let total: u64 = raw.iter().map(|r| r.byte_len as u64).sum();
    if total <= budget {
        return None;
    }
    let mut acc: u64 = 0;
    let mut min_ts: Option<i64> = None;
    // Walk newest-first, accumulating bytes; the oldest ts reached before the budget
    // is exhausted is the boundary older records would have been rotated past.
    for r in raw.iter().rev() {
        acc = acc.saturating_add(r.byte_len as u64);
        if let Some(ts) = r.ts {
            min_ts = Some(min_ts.map_or(ts, |m| m.min(ts)));
        }
        if acc >= budget {
            break;
        }
    }
    min_ts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_filename_is_injective_percent_encoding() {
        // The safe class `[A-Za-z0-9.-]` survives verbatim; everything else — `/`,
        // space, control chars, AND `_`/`%` themselves — is `%XX`.
        assert_eq!(mark_filename("gpu-notes.md"), "m_gpu-notes.md");
        assert_eq!(mark_filename("a/b c"), "m_a%2Fb%20c"); // '/'=2F, ' '=20
        assert_eq!(mark_filename("we\tird\nname"), "m_we%09ird%0Aname"); // \t=09 \n=0A
        assert_eq!(mark_filename(""), "m_");
        // Injective where the old collapse-to-`_` mapping collided (the FIX-2 bug):
        // distinct ids MUST yield distinct filenames.
        assert_ne!(mark_filename("gpu notes"), mark_filename("gpu_notes"));
        assert_ne!(mark_filename("a/b"), mark_filename("a_b"));
        assert_ne!(mark_filename("a%20b"), mark_filename("a b")); // literal % is escaped
        // Never a path separator, always the `m_` prefix.
        for id in ["../escape", "a/b/c", "x"] {
            let f = mark_filename(id);
            assert!(f.starts_with("m_"));
            assert!(!f.contains('/'));
        }
        // Deterministic.
        assert_eq!(mark_filename("x/y"), mark_filename("x/y"));
    }

    #[test]
    fn default_runtime_dir_is_engine_namespaced() {
        assert!(default_runtime_dir().ends_with(ENGINE_NAME));
    }

    #[test]
    fn rotation_path_appends_dot_one() {
        assert_eq!(
            rotation_path(Path::new("/s/_recall_telemetry.jsonl")),
            PathBuf::from("/s/_recall_telemetry.jsonl.1")
        );
    }

    #[test]
    fn classify_distinguishes_fire_read_session() {
        let fire = serde_json::json!({"ts": 5, "qid": "q", "mems": [], "conf": "low"});
        let read = serde_json::json!({"ts": 5, "id": "m", "signal": "read"});
        let sess = serde_json::json!({"ts": 5, "signal": "session"});
        assert!(matches!(classify(&fire), Class::Fire(Some(5))));
        assert!(matches!(classify(&read), Class::Read(Some(5))));
        assert!(matches!(classify(&sess), Class::Session(Some(5))));
        // Bad ts on any record kind ⇒ ts None (the WR-05 drop signal).
        let bad_fire = serde_json::json!({"ts": "x", "qid": "q", "mems": [], "conf": "low"});
        let bad_read = serde_json::json!({"id": "m", "signal": "read"});
        let bad_sess = serde_json::json!({"signal": "session"});
        assert!(matches!(classify(&bad_fire), Class::Fire(None)));
        assert!(matches!(classify(&bad_read), Class::Read(None)));
        assert!(matches!(classify(&bad_sess), Class::Session(None)));
    }

    #[test]
    fn rotation_cutoff_none_when_within_budget() {
        let raw = vec![
            RawLine {
                byte_len: 10,
                value: None,
                ts: Some(100),
            },
            RawLine {
                byte_len: 10,
                value: None,
                ts: Some(200),
            },
        ];
        // total 20 <= budget 100 ⇒ rotation does not bind.
        assert_eq!(rotation_cutoff_ts(&raw, 100), None);
        // total 20 > budget 15 ⇒ binds; newest-first fills budget, boundary ts seen.
        assert!(rotation_cutoff_ts(&raw, 15).is_some());
    }
}
