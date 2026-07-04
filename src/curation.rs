//! Self-curation: the maintenance pass + seat governance (plan P12 / WP-6; D7,
//! D25, A7; CORE-SPEC §8).
//!
//! Two independent entry points, each with its own CLI + Appendix D contract:
//!
//! - [`maintain`] — the telemetry-driven `declineCount` pass: the ≥50-record
//!   trigger (recheck-under-lock, WR-02), claim-before-mutate (WR-01), the
//!   `O_EXCL` lock with stale-lock rename-to-corpse reclaim, and the three
//!   floors (zero-fire, minimum-evidence, and — inside [`seats`] — the seat
//!   dual-gate).
//! - [`seats`] — `MEMORY.md` router-seat governance: parses the seat list,
//!   probes each seat through LIVE [`crate::recall::recall`], and emits the
//!   `PENDING-SEAT-CHANGES` block (replace-not-stack; non-block content
//!   untouched).
//!
//! Both **never delete or rewrite a memory body** (D7): the only mutation is
//! `declineCount` in frontmatter, written via `frontmatter::parse → mutate →
//! generate → atomic write`, with the body after the closing fence spliced back
//! in byte-for-byte from the original file — `generate` is never asked to
//! reproduce the body, only the frontmatter block.
//!
//! ## Ground truth (D15: reference, not constraint)
//!
//! The live `synapse` engine (`../synapse/lib/memory_surface.py`, functions
//! `maintenance`/`seats`/`_parse_seat_stems`/`_write_pending_block`/
//! `_acquire_maintenance_lock`, and the seat-probe derivation in
//! `tests/memory_surface/seat_probes.py::_derive_payload`) is read here as
//! **citable evidence of de-facto behavior, nothing more** (D15): the lock
//! reclaim protocol, the claim-before-mutate ordering, the `## Always-relevant
//! entries` seat-link convention, and the commands-then-paths probe-derivation
//! priority are all carried over because nothing in the frozen ledger overrides
//! them and CORE-SPEC §8 is itself distilled from this same behavior. Two
//! specific points are **not** carried over verbatim, by deliberate choice under
//! the frozen contract (not a ground-truth divergence needing escalation):
//!
//! 1. The live engine's seat DEMOTE dual-gate uses a bare `fire_count >= 1`;
//!    this reseed's frozen D7/§8/§10 all agree the threshold is
//!    `seatPromoteMinFires` (default 5) — the ledger is unambiguous and
//!    self-consistent here, so it — not the old hardcode — is the contract.
//! 2. The live engine's `maintenance()` calls `seats()` internally at the end of
//!    every non-shadow pass (`CUR-05`); this reseed's plan/Appendix D wire
//!    `maintain` and `seats` as two independently-invocable CLI surfaces with no
//!    stated auto-chain, so [`maintain`] does not call [`seats`] — WP-5's
//!    session-start wiring decides whether/when to call both.
//!
//! ## The soft time budget (`timeout 2 || true`)
//!
//! CORE-SPEC §8 describes the maintenance pass running under a `timeout 2`
//! process wrapper in the old shell-hook adapter. That is host-integration
//! plumbing (WP-5/WP-8's concern: how `rejolt maintain` gets invoked from
//! session-start), not something this library function can honor internally —
//! spawning a watchdog thread to abort a pure, local, single-pass-over-the-store
//! computation mid-flight would risk a torn write for no benefit. This module's
//! contribution to the budget is doing bounded, single-pass, non-blocking local
//! I/O only (no network, no unbounded loop) so that an outer `timeout` wrapper,
//! if the host adapter applies one, has real headroom.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::bootstrap::MEMORY_ROUTER_FILENAME;
use crate::catalog::write_atomic;
use crate::config::Config;
use crate::frontmatter::{self, Frontmatter, Triggers};
use crate::normalize::{NormalizedOp, ToolOp};
use crate::rebuild::{MemoryFacts, scan_store};
use crate::recall::{RecallOutcome, recall};
use crate::telemetry::{FireRecord, ReadRecord, Telemetry, WindowBound, WindowedTelemetry};

// =============================================================================
// §10 consts (NOT config — the §10 table marks these `const`)
// =============================================================================

/// The maintenance-state sidecar (infra: underscore-prefixed, store scan skips
/// it). Records the LIVE telemetry file's line count at the last pass (WR-01).
pub const MAINTENANCE_STATE_FILENAME: &str = "_maintenance_state.json";
/// `_MAINT_LOCK_STALE_SECS` (§10, const): a lock older than this is a corpse from
/// a killed pass (the pass runs well under this; anything older was abandoned).
pub const MAINT_LOCK_STALE_SECS: u64 = 300;
/// The `maintenance` trigger (§10, const): the pass runs only once the LIVE
/// telemetry file has grown by at least this many records since the last claim.
pub const MAINTENANCE_TRIGGER_RECORDS: u64 = 50;

fn maintenance_state_path(store: &Path) -> PathBuf {
    store.join(MAINTENANCE_STATE_FILENAME)
}

fn lock_path(store: &Path) -> PathBuf {
    let mut s = maintenance_state_path(store).into_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const SECONDS_PER_DAY: i64 = 86_400;

// =============================================================================
// Maintenance state (WR-01 claim) — `_maintenance_state.json`
// =============================================================================

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaintenanceState {
    last_pass_line: u64,
    #[serde(default)]
    last_pass_ts: i64,
}

/// Read the claimed state; a missing/malformed file reads as "no prior pass"
/// (`last_pass_line: 0`) — fail-open, never an error (D6).
fn read_state(store: &Path) -> MaintenanceState {
    let Ok(text) = fs::read_to_string(maintenance_state_path(store)) else {
        return MaintenanceState::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Claim the current LIVE-file line count atomically (WR-01). Fail-open: a write
/// fault is swallowed (the next pass will simply re-see the same delta).
fn write_state(store: &Path, cur_lines: u64) {
    let state = MaintenanceState {
        last_pass_line: cur_lines,
        last_pass_ts: now_unix(),
    };
    if let Ok(json) = serde_json::to_string(&state) {
        let _ = write_atomic(&maintenance_state_path(store), &json);
    }
}

/// Physical line count of the LIVE telemetry file only (NOT the `.1` rotation
/// generation) — mirrors the ground-truth trigger accounting, which measures
/// growth of the live file since the last claim, not the whole (possibly
/// rotated) window. A missing/unreadable file counts as zero.
fn live_telemetry_line_count(tel: &Telemetry) -> u64 {
    match fs::read(tel.telemetry_file()) {
        Ok(bytes) => bytes
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .count() as u64,
        Err(_) => 0,
    }
}

/// The ≥50-new-record trigger, with the rotation-reset rule: if the live file is
/// currently SMALLER than the last claimed count (a rotation happened since),
/// every current line counts as new rather than underflowing.
fn threshold_met(cur_lines: u64, last_pass_line: u64) -> bool {
    let new = if cur_lines < last_pass_line {
        cur_lines
    } else {
        cur_lines - last_pass_line
    };
    new >= MAINTENANCE_TRIGGER_RECORDS
}

// =============================================================================
// The O_EXCL lock + stale-lock reclaim (WR-02; NEVER stat→unlink→create)
// =============================================================================

/// Acquire the maintenance lock (`O_CREAT|O_EXCL`). `Some(path)` on success;
/// `None` when another pass holds a FRESH lock (busy — the caller no-ops) or on
/// any I/O fault (fail-open by SKIPPING, never by running unlocked). A lock
/// older than [`MAINT_LOCK_STALE_SECS`] is a corpse from a killed pass and is
/// reclaimed by an ATOMIC `rename` to a per-pid corpse name — never
/// stat→unlink→create, which would let two reclaimers interleave and let one
/// unlink the OTHER's fresh lock.
fn acquire_lock(store: &Path) -> Option<PathBuf> {
    let lp = lock_path(store);
    for _ in 0..2 {
        // one retry after a stale-lock reclaim
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&lp)
        {
            Ok(_) => return Some(lp),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let age_secs = match fs::symlink_metadata(&lp).and_then(|m| m.modified()) {
                    Ok(mtime) => SystemTime::now()
                        .duration_since(mtime)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    // Lock vanished between open() and stat() — retry create.
                    Err(_) => continue,
                };
                if age_secs <= MAINT_LOCK_STALE_SECS {
                    return None; // fresh lock: a concurrent pass is running
                }
                // Stale: reclaim via atomic rename-to-corpse.
                let corpse = corpse_path(&lp);
                if fs::rename(&lp, &corpse).is_err() {
                    return None; // another reclaimer won the race first — defer
                }
                let _ = fs::remove_file(&corpse);
                // loop retries create — the lock path is now clear
            }
            Err(_) => return None, // unwritable store etc. — mutations would fail anyway
        }
    }
    None
}

fn corpse_path(lock: &Path) -> PathBuf {
    let mut s = lock.as_os_str().to_os_string();
    s.push(format!(".reclaim.{}", std::process::id()));
    PathBuf::from(s)
}

fn release_lock(lock: &Path) {
    let _ = fs::remove_file(lock);
}

// =============================================================================
// Windowed evidence stats (the minimum-evidence floor, gates the WHOLE pass)
// =============================================================================

/// `(distinct session-days, span-days)` over the windowed telemetry (A7: the
/// `min(30d, rotation bound)` window `read_window` already applied). Distinct
/// session-days come from `sessions` (WR-03 parity: a busy day of resumes must
/// not inflate the count); span is `now - earliest ts among fires/reads/sessions
/// in-window`. Both are naturally bounded by the window itself.
fn evidence_stats(window: &WindowedTelemetry) -> (u64, f64) {
    let mut days: BTreeSet<i64> = BTreeSet::new();
    for s in &window.sessions {
        days.insert(s.ts.div_euclid(SECONDS_PER_DAY));
    }
    let earliest = window
        .fires
        .iter()
        .map(|f| f.ts)
        .chain(window.reads.iter().map(|r| r.ts))
        .chain(window.sessions.iter().map(|s| s.ts))
        .min();
    let span_days = match earliest {
        Some(e) => ((now_unix() - e) as f64 / SECONDS_PER_DAY as f64).max(0.0),
        None => 0.0,
    };
    (days.len() as u64, span_days)
}

fn count_fires_by_id(fires: &[FireRecord]) -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    for f in fires {
        for mem in &f.mems {
            *m.entry(mem.id.clone()).or_insert(0) += 1;
        }
    }
    m
}

fn count_reads_by_id(reads: &[ReadRecord]) -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    for r in reads {
        *m.entry(r.id.clone()).or_insert(0) += 1;
    }
    m
}

// =============================================================================
// Body-preserving frontmatter mutation (D7: declineCount ONLY, body untouched)
// =============================================================================

/// The byte offset where the body begins (immediately after the closing `---`
/// fence's line, including its terminator) — mirrors `frontmatter::tokenize`'s
/// exact fence rule (line 1 is exactly `---`, ignoring one trailing `\r`; the
/// next such line closes the block) so the split point can never disagree with
/// what [`frontmatter::parse`] itself considered the fence.
fn body_start_offset(input: &str) -> Option<usize> {
    fn is_fence_line(line_with_terminator: &str) -> bool {
        let no_nl = line_with_terminator
            .strip_suffix('\n')
            .unwrap_or(line_with_terminator);
        let no_cr = no_nl.strip_suffix('\r').unwrap_or(no_nl);
        no_cr == "---"
    }
    let mut it = input.split_inclusive('\n');
    let first = it.next()?;
    if !is_fence_line(first) {
        return None;
    }
    let mut idx = first.len();
    for line in it {
        idx += line.len();
        if is_fence_line(line) {
            return Some(idx);
        }
    }
    None
}

/// Set `decline_count` to `new_count` and re-render, splicing the ORIGINAL body
/// bytes (after the closing fence) back in untouched. `None` only if `original`
/// somehow lacks a closing fence (cannot happen for a memory `scan_store` already
/// parsed — fail-open: the caller skips the write rather than guessing).
fn set_decline_count(original: &str, fm: &Frontmatter, new_count: i64) -> Option<String> {
    let body_start = body_start_offset(original)?;
    let body = &original[body_start..];
    let mut mutated = fm.clone();
    mutated.metadata.decline_count = Some(new_count);
    let new_block = frontmatter::generate(&mutated);
    Some(format!("{new_block}{body}"))
}

fn apply_decline(mem: &MemoryFacts, new_count: i64) -> std::io::Result<()> {
    match set_decline_count(&mem.content, &mem.fm, new_count) {
        Some(new_content) => write_atomic(Path::new(&mem.path), &new_content),
        None => Ok(()), // fail-open: nothing to splice against, skip rather than guess
    }
}

// =============================================================================
// maintain() — the declineCount pass
// =============================================================================

/// The outcome of one [`maintain`] call. Every variant is a NORMAL, non-error
/// result — a pass that correctly decided not to mutate (below trigger, lock
/// held, evidence thin) is success, not failure (D6/D7 fail-open posture).
#[derive(Debug, Clone, PartialEq)]
pub enum MaintainOutcome {
    /// The pre-lock fast check found telemetry growth below the trigger; no lock
    /// was even attempted. (Bypassed when `force` is set.)
    BelowTrigger,
    /// Another pass holds a FRESH lock; this pass no-ops (WR-02 mutual exclusion).
    LockHeld,
    /// WR-02: re-verified UNDER the lock and the trigger was no longer met (a
    /// losing racer whose pre-lock check saw a stale total) — no-op, no mutation.
    ThresholdUnmetUnderLock,
    /// The minimum-evidence floor blocked ALL mutation this pass (a global gate,
    /// not per-memory). State was still claimed (WR-01) so an evidence-starved
    /// store doesn't retrigger the same insufficient pass every session once the
    /// record trigger is met.
    InsufficientEvidence { session_days: u64, span_days: f64 },
    /// The pass ran: floors applied, `declineCount` mutations written.
    Ran {
        promoted: Vec<String>,
        demoted: Vec<String>,
        zero_fire: Vec<String>,
        bound: WindowBound,
    },
    /// A genuine I/O fault reading the store (e.g. `scan_store` failed). The only
    /// variant a direct CLI should treat as a failed check.
    Io(String),
}

/// The self-curation maintenance pass (P12; D7, D25, A7; §8). Runs the ≥50-record
/// trigger (recheck-under-lock, WR-02), claims state before any mutation (WR-01),
/// then scores each memory over the windowed telemetry (`read_window`, A7),
/// applying the zero-fire floor and the minimum-evidence floor before writing any
/// `declineCount`. `force` bypasses BOTH trigger checks (pre-lock and under-lock)
/// but NOT the lock itself (mutual exclusion always applies) nor the
/// minimum-evidence floor (a floor, never bypassable).
pub fn maintain(store: &Path, cfg: &Config, force: bool) -> MaintainOutcome {
    let tel = Telemetry::for_store(store, cfg.clone());

    if !force {
        let cur = live_telemetry_line_count(&tel);
        let last = read_state(store).last_pass_line;
        if !threshold_met(cur, last) {
            return MaintainOutcome::BelowTrigger;
        }
    }

    let Some(lock) = acquire_lock(store) else {
        return MaintainOutcome::LockHeld;
    };
    let outcome = run_locked_pass(store, cfg, &tel, force);
    release_lock(&lock);
    outcome
}

/// Everything that happens WHILE the lock is held: the WR-02 recheck, the WR-01
/// claim, the evidence floor, and (if evidence holds) the scoring + mutation
/// loop with the zero-fire floor.
fn run_locked_pass(store: &Path, cfg: &Config, tel: &Telemetry, force: bool) -> MaintainOutcome {
    if !force {
        let cur = live_telemetry_line_count(tel);
        let last = read_state(store).last_pass_line;
        if !threshold_met(cur, last) {
            return MaintainOutcome::ThresholdUnmetUnderLock;
        }
    }

    // WR-01 claim-before-mutate: claimed here, before ANY declineCount write —
    // regardless of whether the evidence floor below ends up blocking mutation.
    let cur_final = live_telemetry_line_count(tel);
    write_state(store, cur_final);

    let window = tel.read_window();
    let (session_days, span_days) = evidence_stats(&window);
    let evidence_ok =
        session_days >= cfg.min_evidence_sessions || span_days >= cfg.min_evidence_days as f64;
    if !evidence_ok {
        return MaintainOutcome::InsufficientEvidence {
            session_days,
            span_days,
        };
    }

    let fires_by_id = count_fires_by_id(&window.fires);
    let reads_by_id = count_reads_by_id(&window.reads);

    let (memories, _malformed) = match scan_store(store) {
        Ok(r) => r,
        Err(e) => return MaintainOutcome::Io(e.to_string()),
    };

    let mut promoted = Vec::new();
    let mut demoted = Vec::new();
    let mut zero_fire = Vec::new();

    for mem in &memories {
        let fire_count = fires_by_id.get(&mem.id).copied().unwrap_or(0);
        // Floor 1 (D-43/zero-fire): NEVER demoted. Precedes rate computation
        // (division by fire_count would panic/NaN-drift otherwise).
        if fire_count == 0 {
            zero_fire.push(mem.id.clone());
            continue;
        }
        let read_count = reads_by_id.get(&mem.id).copied().unwrap_or(0);
        let rate = read_count as f64 / fire_count as f64;
        if rate >= cfg.promote_threshold {
            if apply_decline(mem, 0).is_ok() {
                promoted.push(mem.id.clone());
            }
        } else if rate <= cfg.demote_threshold {
            let cur = mem.fm.metadata.decline_count.unwrap_or(0);
            if apply_decline(mem, cur + 1).is_ok() {
                demoted.push(mem.id.clone());
            }
        }
    }

    MaintainOutcome::Ran {
        promoted,
        demoted,
        zero_fire,
        bound: window.bound,
    }
}

// =============================================================================
// Seats — MEMORY.md router-seat governance
// =============================================================================

/// The heading `_parse_seat_stems` scans under (the live-engine seat-link
/// convention, D15 reference evidence: nothing in the frozen ledger names a
/// different one, and the bootstrap seed `# Memory Router\n` naturally carries
/// no such section, so a fresh store starts with zero seats).
const SEAT_SECTION_HEADING: &str = "## Always-relevant";
const PENDING_BLOCK_MARKER: &str = "<!-- PENDING-SEAT-CHANGES";

/// Parse router seat stems from `MEMORY.md` text: markdown-link targets
/// (`](stem.md)`, no slash — store-relative only) inside the
/// `## Always-relevant entries` section, up to the next `## ` heading.
pub fn parse_seat_stems(memory_router_text: &str) -> Vec<String> {
    let mut stems = Vec::new();
    let mut in_section = false;
    for line in memory_router_text.lines() {
        let stripped = line.trim();
        if stripped.starts_with(SEAT_SECTION_HEADING) {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        if stripped.starts_with("## ") {
            break;
        }
        if let Some(stem) = extract_seat_link(stripped) {
            stems.push(stem);
        }
    }
    stems
}

/// Extract a seat stem from a `](stem.md)` markdown-link target: scans every
/// `](` in the line (robust to nested brackets in link titles) and accepts the
/// first whose parenthesised target has no `/` and ends in `.md`.
fn extract_seat_link(line: &str) -> Option<String> {
    for (idx, _) in line.match_indices("](") {
        let rest = &line[idx + 2..];
        let Some(close) = rest.find(')') else {
            continue;
        };
        let target = &rest[..close];
        if target.contains('/') {
            continue;
        }
        if let Some(stem) = target.strip_suffix(".md")
            && !stem.is_empty()
        {
            return Some(stem.to_string());
        }
    }
    None
}

/// Strip any existing `<!-- PENDING-SEAT-CHANGES … -->` block (plus any
/// newlines immediately following it) from `text`. A block with no closing
/// `-->` anywhere in the remaining text is left untouched (no match — the
/// defensive, non-destructive direction).
fn strip_pending_block(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(start) = rest.find(PENDING_BLOCK_MARKER) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let from_marker = &rest[start..];
        let Some(end_rel) = from_marker.find("-->") else {
            out.push_str(from_marker);
            break;
        };
        let after_close = &from_marker[end_rel + 3..];
        rest = after_close.trim_start_matches('\n');
    }
    out
}

/// Write (or remove) the `PENDING-SEAT-CHANGES` block at the top of
/// `memory_router_path` (replace-not-stack): strips any existing block, then —
/// if `proposals` is non-empty — prepends a fresh one. The rest of the file
/// (`stripped`) is written back BYTE-IDENTICAL to what it was minus the old
/// block; an empty `proposals` with no existing block is a true no-op (no write
/// at all, so mtime/atomicity are never disturbed for nothing).
fn write_pending_block(memory_router_path: &Path, proposals: &[String]) -> std::io::Result<()> {
    let current = fs::read_to_string(memory_router_path)?;
    let stripped = strip_pending_block(&current);

    if proposals.is_empty() {
        if stripped != current {
            write_atomic(memory_router_path, &stripped)?;
        }
        return Ok(());
    }

    let today = today_iso();
    let mut block = format!(
        "<!-- PENDING-SEAT-CHANGES (automated, {today}) — review and delete this block to approve:\n"
    );
    for p in proposals {
        block.push_str("  ");
        block.push_str(p);
        block.push('\n');
    }
    block.push_str("-->\n");
    write_atomic(memory_router_path, &format!("{block}{stripped}"))
}

/// `YYYY-MM-DD` for today (UTC), pure integer arithmetic (no date crate) —
/// mirrors [`crate::recall`]'s civil-date math, run in reverse (days since epoch
/// → y/m/d) rather than duplicated as a dependency.
fn today_iso() -> String {
    let days = now_unix().div_euclid(SECONDS_PER_DAY);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant's `civil_from_days`: days-since-epoch → `(year, month, day)`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// One seat's probe result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatProbe {
    pub stem: String,
    /// Whether a probe payload derived from the seat's OWN frontmatter triggers
    /// surfaces it through LIVE `recall` (condition (a) of the seat dual-gate).
    pub covered: bool,
    /// In-window telemetry fire count for this seat (condition (b)).
    pub fire_count: u64,
}

/// The outcome of one [`seats`] call.
#[derive(Debug, Clone, PartialEq)]
pub enum SeatsOutcome {
    /// The minimum-evidence floor (same global gate as [`maintain`]) blocked
    /// seat governance; any stale pending block was still removed when
    /// `propose` was set (never leave a proposal that predates enough evidence).
    InsufficientEvidence { session_days: u64, span_days: f64 },
    /// Ran: every seat's probe + the demote proposals the dual-gate cleared.
    /// `written` is `true` iff `propose` was set AND `MEMORY.md` exists.
    Ran {
        demote: Vec<String>,
        probes: Vec<SeatProbe>,
        written: bool,
    },
}

/// Seat governance (P12; D7/§8 seat dual-gate). Parses `MEMORY.md`'s seat list,
/// probes each seat through LIVE recall using a payload derived from the seat's
/// OWN declared triggers (commands, then paths — the live-engine probe-runner's
/// derivation priority, D15 reference evidence; a seat with only args/synonyms,
/// or none at all, has `covered: false`, never a mechanical demotion), and
/// proposes a demotion only when BOTH the probe is covered AND telemetry shows
/// `seatPromoteMinFires` in-window fires. `propose` controls whether the
/// `PENDING-SEAT-CHANGES` block is actually written; `false` computes and
/// reports without touching `MEMORY.md`.
pub fn seats(store: &Path, cfg: &Config, propose: bool) -> std::io::Result<SeatsOutcome> {
    let router_path = store.join(MEMORY_ROUTER_FILENAME);
    let router_text = fs::read_to_string(&router_path).unwrap_or_default();
    let seat_stems = parse_seat_stems(&router_text);

    let tel = Telemetry::for_store(store, cfg.clone());
    let window = tel.read_window();
    let (session_days, span_days) = evidence_stats(&window);
    let evidence_ok =
        session_days >= cfg.min_evidence_sessions || span_days >= cfg.min_evidence_days as f64;
    if !evidence_ok {
        if propose && router_path.exists() {
            write_pending_block(&router_path, &[])?; // remove a stale block, if any
        }
        return Ok(SeatsOutcome::InsufficientEvidence {
            session_days,
            span_days,
        });
    }

    let fires_by_id = count_fires_by_id(&window.fires);

    let mut demote = Vec::new();
    let mut probes = Vec::new();
    for stem in &seat_stems {
        let fire_count = fires_by_id.get(stem).copied().unwrap_or(0);
        let covered = probe_seat(store, stem, cfg);
        probes.push(SeatProbe {
            stem: stem.clone(),
            covered,
            fire_count,
        });
        // The seat dual-gate (D7/§8): BOTH legs, never either alone.
        if covered && fire_count >= cfg.seat_promote_min_fires {
            demote.push(stem.clone());
        }
    }

    let mut written = false;
    if propose && router_path.exists() {
        let proposals: Vec<String> = demote
            .iter()
            .map(|s| format!("DEMOTE: {s}.md — fired {}x in window", fires_by_id[s]))
            .collect();
        write_pending_block(&router_path, &proposals)?;
        written = true;
    }

    Ok(SeatsOutcome::Ran {
        demote,
        probes,
        written,
    })
}

/// Does a probe payload derived from `stem`'s OWN frontmatter triggers surface
/// it through LIVE recall? Uses a throwaway, per-call-isolated [`Telemetry`] (a
/// fresh temp dir every call) so a probe never touches the real dedup marks or
/// store telemetry — matching the live-engine probe runner's isolated-XDG
/// discipline, but simpler here since each call already gets a pristine dir
/// (no cross-seat mark accumulation to guard against).
fn probe_seat(store: &Path, stem: &str, cfg: &Config) -> bool {
    let mem_path = store.join(format!("{stem}.md"));
    let Ok(raw) = fs::read_to_string(&mem_path) else {
        return false; // missing memory — fail-safe: not covered
    };
    let Ok(fm) = frontmatter::parse(&raw) else {
        return false;
    };
    let Some(triggers) = &fm.metadata.triggers else {
        return false; // no triggers block at all → nothing to derive a probe from
    };
    let Some(op) = derive_probe_op(triggers) else {
        return false; // args/synonyms-only (or empty) → not mechanically derivable
    };
    let probe_tel = isolated_probe_telemetry(cfg.clone());
    let normalized = NormalizedOp::PreOp(op);
    match recall(&normalized, store, &probe_tel) {
        RecallOutcome::Advisory(a) => a.memories.iter().any(|m| m.memory_id == stem),
        RecallOutcome::Silence => false,
    }
}

/// Derive one probe [`ToolOp`] from a memory's `triggers:` block (the live-engine
/// probe runner's priority, D15 reference evidence): first a declared command
/// (`{cmd} --help`, strong tier — a single hit clears the surface gate alone),
/// else a declared path (glob concretized to a representative file, strong
/// tier). Args/synonyms are weak/medium only (never alone sufficient to clear
/// the surface gate) and are not attempted — a seat whose ONLY evidence is
/// args/synonyms correctly probes as not-covered, matching what `recall` would
/// actually do with that evidence in isolation.
fn derive_probe_op(triggers: &Triggers) -> Option<ToolOp> {
    if let Some(cmd) = triggers
        .commands
        .iter()
        .map(|c| c.trim())
        .find(|c| !c.is_empty())
    {
        return Some(ToolOp {
            tool_name: "Bash".to_string(),
            command_text: Some(format!("{cmd} --help")),
            cwd: Some(PathBuf::from("/tmp")),
            ..Default::default()
        });
    }
    if let Some(pat) = triggers
        .paths
        .iter()
        .map(|p| p.trim())
        .find(|p| !p.is_empty())
    {
        return Some(ToolOp {
            tool_name: "Read".to_string(),
            target_path: Some(PathBuf::from(concretize_path_glob(pat))),
            cwd: Some(PathBuf::from("/tmp")),
            ..Default::default()
        });
    }
    None
}

/// Instantiate a glob pattern into a concrete representative path: `/**` and
/// bare `**` become a literal probe filename, and a leading `~` expands to
/// `$HOME` (falling back to the literal pattern if `$HOME` is unset).
fn concretize_path_glob(pat: &str) -> String {
    let s = pat
        .replace("/**", "/test-file.txt")
        .replace("**", "test-file.txt");
    if let Some(rest) = s.strip_prefix('~')
        && let Some(home) = std::env::var_os("HOME")
    {
        return format!("{}{}", PathBuf::from(home).display(), rest);
    }
    s
}

/// A throwaway [`Telemetry`] for a seat probe: an isolated temp mark-dir + temp
/// telemetry file, unique per call, so a probe never writes into the real dedup
/// marks or store telemetry (mirrors `cli::probe_telemetry` for `search`).
fn isolated_probe_telemetry(cfg: Config) -> Telemetry {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("rejolt-seat-probe-{}-{n}", std::process::id()));
    Telemetry::new(base.join("rt"), base.join("tel.jsonl"), cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering as O};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, O::Relaxed);
        let dir = std::env::temp_dir().join(format!("rejolt-wp6-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    // -------------------------------------------------------------------
    // Body-preserving mutation (D7)
    // -------------------------------------------------------------------

    #[test]
    fn body_start_offset_finds_the_closing_fence() {
        let src = "---\nmetadata:\n  tags: [t]\n---\nBODY LINE 1\nBODY LINE 2\n";
        let start = body_start_offset(src).unwrap();
        assert_eq!(&src[start..], "BODY LINE 1\nBODY LINE 2\n");
    }

    #[test]
    fn body_start_offset_none_without_closing_fence() {
        assert!(body_start_offset("---\nmetadata:\n  tags: [t]\n").is_none());
        assert!(body_start_offset("not frontmatter at all\n").is_none());
    }

    #[test]
    fn set_decline_count_preserves_body_byte_identical() {
        let src = "---\nname: x\nmetadata:\n  tags: [t]\n  declineCount: 0\n---\nBODY\nunchanged\n";
        let fm = frontmatter::parse(src).unwrap();
        let out = set_decline_count(src, &fm, 3).unwrap();
        let body_start = body_start_offset(&out).unwrap();
        assert_eq!(&out[body_start..], "BODY\nunchanged\n");
        let reparsed = frontmatter::parse(&out).unwrap();
        assert_eq!(reparsed.metadata.decline_count, Some(3));
    }

    // -------------------------------------------------------------------
    // Threshold + rotation-reset rule
    // -------------------------------------------------------------------

    #[test]
    fn threshold_met_matrix() {
        assert!(!threshold_met(49, 0)); // GOOD (below): 49 new < 50
        assert!(threshold_met(50, 0)); // GOOD (at): 50 new >= 50
        assert!(threshold_met(120, 50)); // 70 new >= 50
        assert!(!threshold_met(120, 100)); // 20 new < 50
        // Rotation-reset: current < last claimed -> treat all current as new.
        assert!(threshold_met(60, 1_000_000));
        assert!(!threshold_met(10, 1_000_000));
    }

    // -------------------------------------------------------------------
    // Lock: fresh busy / stale reclaim / release
    // -------------------------------------------------------------------

    #[test]
    fn lock_fresh_is_busy_stale_is_reclaimed() {
        let dir = unique_dir("lock");
        // GOOD: no lock present -> acquires.
        let lock = acquire_lock(&dir).expect("first acquire succeeds");
        assert!(lock.exists());

        // BAD (busy): a second acquire while the first is still held -> None.
        assert!(
            acquire_lock(&dir).is_none(),
            "a fresh lock must block a second acquire"
        );
        release_lock(&lock);
        assert!(!lock.exists(), "release removes the lock file");

        // Re-acquire after release succeeds.
        let lock2 = acquire_lock(&dir).expect("re-acquire after release succeeds");

        // Backdate the lock past the stale threshold and confirm reclaim.
        let stale_time =
            SystemTime::now() - std::time::Duration::from_secs(MAINT_LOCK_STALE_SECS + 5);
        let f = fs::File::options().write(true).open(&lock2).unwrap();
        f.set_modified(stale_time).unwrap();

        let lock3 = acquire_lock(&dir).expect("a stale lock is reclaimed, not left busy");
        assert_eq!(lock3, lock2, "same lock path, reclaimed in place");
        // No corpse file left behind after reclaim.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".reclaim."))
            .collect();
        assert!(leftovers.is_empty(), "reclaim must not leave a corpse file");
        release_lock(&lock3);
    }

    // -------------------------------------------------------------------
    // Seat-link parsing + pending-block replace-not-stack
    // -------------------------------------------------------------------

    #[test]
    fn parse_seat_stems_scans_only_the_always_relevant_section() {
        let text = "# Memory Router\n\n## Always-relevant entries\n\n- [GPU notes](gpu-notes.md)\n- [[Misfire] weird title](weird.md)\n\n## Other section\n\n- [not-a-seat](elsewhere.md)\n";
        let stems = parse_seat_stems(text);
        assert_eq!(stems, vec!["gpu-notes".to_string(), "weird".to_string()]);
    }

    #[test]
    fn parse_seat_stems_rejects_slash_bearing_targets() {
        let text = "## Always-relevant entries\n\n- [nested](sub/dir.md)\n- [ok](fine.md)\n";
        assert_eq!(parse_seat_stems(text), vec!["fine".to_string()]);
    }

    #[test]
    fn parse_seat_stems_empty_without_the_section() {
        // The bootstrap seed is exactly this: no section -> zero seats.
        assert!(parse_seat_stems("# Memory Router\n").is_empty());
    }

    #[test]
    fn pending_block_replaces_not_stacks_and_preserves_non_block_content() {
        let dir = unique_dir("pending");
        let router = dir.join("MEMORY.md");
        let original = "# Memory Router\n\n## Always-relevant entries\n\n- [x](x.md)\n";
        fs::write(&router, original).unwrap();

        write_pending_block(&router, &["DEMOTE: x.md — reason A".to_string()]).unwrap();
        let after_first = fs::read_to_string(&router).unwrap();
        assert!(after_first.starts_with(PENDING_BLOCK_MARKER));
        assert_eq!(after_first.matches(PENDING_BLOCK_MARKER).count(), 1);
        assert!(
            after_first.ends_with(original),
            "non-block content byte-identical"
        );

        // A second run with a DIFFERENT proposal must REPLACE, never stack.
        write_pending_block(&router, &["DEMOTE: x.md — reason B".to_string()]).unwrap();
        let after_second = fs::read_to_string(&router).unwrap();
        assert_eq!(
            after_second.matches(PENDING_BLOCK_MARKER).count(),
            1,
            "re-run must replace, not stack"
        );
        assert!(after_second.contains("reason B"));
        assert!(!after_second.contains("reason A"));
        assert!(
            after_second.ends_with(original),
            "non-block content still byte-identical"
        );

        // Emptying the proposals removes the block entirely, restoring the original.
        write_pending_block(&router, &[]).unwrap();
        let after_clear = fs::read_to_string(&router).unwrap();
        assert_eq!(after_clear, original);
    }

    #[test]
    fn concretize_path_glob_instantiates_double_star_and_tilde() {
        assert_eq!(
            concretize_path_glob("~/.config/gpu/**"),
            format!(
                "{}/.config/gpu/test-file.txt",
                PathBuf::from(std::env::var_os("HOME").unwrap()).display()
            )
        );
        assert_eq!(concretize_path_glob("/proj/**"), "/proj/test-file.txt");
    }

    #[test]
    fn today_iso_matches_known_civil_dates() {
        // Cross-check against recall.rs's forward civil_to_days at a few pins.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
    }

    // -------------------------------------------------------------------
    // evidence_stats: distinct session-days from `sessions`, span from the
    // earliest ts among fires/reads/sessions (A7 window semantics).
    // -------------------------------------------------------------------

    fn fire(ts: i64, id: &str) -> FireRecord {
        FireRecord {
            ts,
            qid: format!("q{ts}"),
            mems: vec![crate::telemetry::FireMem {
                id: id.to_string(),
                tag: id.to_string(),
                trigger_type: "command".to_string(),
                val: "x".to_string(),
            }],
            conf: "high".to_string(),
        }
    }

    fn session(ts: i64) -> crate::telemetry::SessionRecord {
        crate::telemetry::SessionRecord {
            ts,
            signal: "session".to_string(),
        }
    }

    #[test]
    fn evidence_stats_counts_distinct_session_days_and_earliest_span() {
        let now = now_unix();
        let day = SECONDS_PER_DAY;
        // GOOD: three sessions on three distinct days -> 3 distinct days.
        let window = WindowedTelemetry {
            fires: vec![fire(now - 5 * day, "m")],
            reads: vec![],
            sessions: vec![session(now), session(now - day), session(now - 2 * day)],
            bound: WindowBound::TimeCapped,
            dropped_bad_ts: 0,
        };
        let (days, span) = evidence_stats(&window);
        assert_eq!(days, 3);
        // Span is bounded by the EARLIEST record of any kind (the fire at -5d),
        // not just sessions.
        assert!((span - 5.0).abs() < 0.1, "span was {span}");

        // BAD (contrast): two sessions on the SAME calendar day must count once,
        // not twice (WR-03 parity — a busy day of resumes must not inflate it).
        let same_day = WindowedTelemetry {
            fires: vec![],
            reads: vec![],
            sessions: vec![session(now), session(now + 1)],
            bound: WindowBound::TimeCapped,
            dropped_bad_ts: 0,
        };
        let (days2, _) = evidence_stats(&same_day);
        assert_eq!(days2, 1, "same calendar day must dedupe to one");

        // Empty window -> zero days, zero span.
        let empty = WindowedTelemetry {
            fires: vec![],
            reads: vec![],
            sessions: vec![],
            bound: WindowBound::TimeCapped,
            dropped_bad_ts: 0,
        };
        assert_eq!(evidence_stats(&empty), (0, 0.0));
    }

    #[test]
    fn count_fires_and_reads_by_id_tally_per_memory() {
        let fires = vec![fire(1, "a"), fire(2, "a"), fire(3, "b")];
        let by_id = count_fires_by_id(&fires);
        assert_eq!(by_id.get("a"), Some(&2));
        assert_eq!(by_id.get("b"), Some(&1));
        assert_eq!(by_id.get("c"), None);

        let reads = vec![
            ReadRecord {
                ts: 1,
                id: "a".to_string(),
                signal: "read".to_string(),
            },
            ReadRecord {
                ts: 2,
                id: "a".to_string(),
                signal: "read".to_string(),
            },
        ];
        let reads_by_id = count_reads_by_id(&reads);
        assert_eq!(reads_by_id.get("a"), Some(&2));
        assert_eq!(reads_by_id.get("b"), None);
    }

    // -------------------------------------------------------------------
    // WR-02 recheck-under-lock + WR-01 claim-before-mutate ordering
    // -------------------------------------------------------------------

    #[test]
    fn wr02_recheck_under_lock_catches_a_racing_claim() {
        // Simulate the race WR-02 exists to close: the pre-lock (outer) check saw
        // the trigger met, but by the time this pass is UNDER the lock, another
        // racer already claimed the same telemetry growth (advanced
        // `_maintenance_state.json` to the current line count). `run_locked_pass`
        // re-verifies the trigger itself (the "under lock" recheck) — private
        // access, exercised directly here rather than via real threads.
        let dir = unique_dir("wr02");
        let cfg = Config::default();
        let tel_path = dir.join(crate::telemetry::TELEMETRY_FILENAME);
        // 60 lines live -> the OUTER check (against last_pass_line=0) would see
        // 60 new records, comfortably over the 50 trigger.
        let lines: String = (0..60).map(|i| format!("{{\"ts\":{i}}}\n")).collect();
        fs::write(&tel_path, lines).unwrap();
        let tel = Telemetry::new(dir.join("rt"), tel_path.clone(), cfg.clone());

        // The racer already claimed: last_pass_line == current line count, so the
        // UNDER-LOCK recheck sees zero new records.
        write_state(&dir, live_telemetry_line_count(&tel));

        let outcome = run_locked_pass(&dir, &cfg, &tel, false);
        assert_eq!(outcome, MaintainOutcome::ThresholdUnmetUnderLock);
    }

    #[test]
    fn claim_before_mutate_advances_state_even_on_insufficient_evidence() {
        // WR-01: state must be claimed (last_pass_line advanced to the current
        // count) BEFORE mutation is even considered — including the branch where
        // the minimum-evidence floor ends up blocking every mutation, so an
        // evidence-starved store doesn't retrigger every session once the
        // record count crosses 50.
        let dir = unique_dir("wr01");
        let cfg = Config::default();
        let tel_path = dir.join(crate::telemetry::TELEMETRY_FILENAME);
        let lines: String = (0..60).map(|i| format!("{{\"ts\":{i}}}\n")).collect();
        fs::write(&tel_path, lines).unwrap();
        let tel = Telemetry::new(dir.join("rt"), tel_path.clone(), cfg.clone());

        assert_eq!(read_state(&dir).last_pass_line, 0, "no prior claim yet");
        let outcome = run_locked_pass(&dir, &cfg, &tel, false);
        assert!(matches!(
            outcome,
            MaintainOutcome::InsufficientEvidence { .. }
        ));
        let cur = live_telemetry_line_count(&tel);
        assert_eq!(
            read_state(&dir).last_pass_line,
            cur,
            "WR-01: state claimed even though evidence blocked mutation"
        );
    }

    #[test]
    fn force_bypasses_trigger_but_not_evidence_or_lock() {
        // `force` skips BOTH the pre-lock and under-lock trigger checks (no
        // telemetry growth needed at all here) but the minimum-evidence floor
        // still applies — a floor is never bypassable.
        let dir = unique_dir("force");
        let cfg = Config::default();
        let outcome = maintain(&dir, &cfg, true);
        assert!(
            matches!(outcome, MaintainOutcome::InsufficientEvidence { .. }),
            "force skips the trigger, not the evidence floor: {outcome:?}"
        );
    }
}
