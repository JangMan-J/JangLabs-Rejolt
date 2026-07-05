//! Conformance for self-curation (WP-6 / P12; D7, D25, A7; CORE-SPEC §8). Covers
//! the WP-6 gate: the three floors (zero-fire, minimum-evidence, seat dual-gate)
//! with known-good/known-bad contrast per floor, the never-rewrites-a-body
//! contract (D7), and the `PENDING-SEAT-CHANGES` replace-not-stack seat
//! governance surface. Concurrency (the lock + WR-01/WR-02 recheck) is unit-
//! tested inside `src/curation.rs` (it needs private access to simulate a
//! race); this file drives only the public `maintain`/`seats` API end-to-end
//! over real on-disk stores.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rejolt::bootstrap::EMPTY_GRAMMAR_SEED;
use rejolt::config::Config;
use rejolt::curation::{self, MaintainOutcome, SeatsOutcome};
use rejolt::rebuild::{BuildConfig, rebuild};
use rejolt::telemetry::TELEMETRY_FILENAME;

// =============================================================================
// Helpers
// =============================================================================

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rejolt-wp6-e2e-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const DAY: i64 = 86_400;

/// Write a minimal memory file: `metadata.tags` + an optional `triggers:` +
/// `declineCount`, plus a body so the never-rewrites-body assertion has
/// something to check.
fn write_memory(store: &Path, id: &str, triggers_toml_like: &str, decline_count: Option<i64>) {
    let mut s = String::from("---\nmetadata:\n  tags: [seat]\n");
    if !triggers_toml_like.is_empty() {
        s.push_str("  triggers:\n");
        s.push_str(triggers_toml_like);
    }
    if let Some(dc) = decline_count {
        s.push_str(&format!("  declineCount: {dc}\n"));
    }
    s.push_str("---\n");
    s.push_str(&format!("BODY for {id} — must never change.\n"));
    fs::write(store.join(format!("{id}.md")), s).expect("write memory");
}

fn read_memory(store: &Path, id: &str) -> String {
    fs::read_to_string(store.join(format!("{id}.md"))).expect("read memory")
}

fn body_of(content: &str) -> &str {
    let close = content
        .match_indices("---")
        .nth(1)
        .expect("closing fence")
        .0;
    content[close + 3..].trim_start_matches('\n')
}

/// One fire telemetry record crediting `id` at `ts`.
fn fire_line(ts: i64, id: &str) -> String {
    format!(
        "{{\"ts\":{ts},\"qid\":\"q{ts}-{id}\",\"mems\":[{{\"id\":\"{id}\",\"tag\":\"{id}\",\"type\":\"command\",\"val\":\"x\"}}],\"conf\":\"high\"}}\n"
    )
}

fn read_line(ts: i64, id: &str) -> String {
    format!("{{\"ts\":{ts},\"id\":\"{id}\",\"signal\":\"read\"}}\n")
}

fn session_line(ts: i64) -> String {
    format!("{{\"ts\":{ts},\"signal\":\"session\"}}\n")
}

fn append_telemetry(store: &Path, lines: &[String]) {
    let path = store.join(TELEMETRY_FILENAME);
    let mut existing = fs::read_to_string(&path).unwrap_or_default();
    for l in lines {
        existing.push_str(l);
    }
    fs::write(path, existing).unwrap();
}

/// 10 session markers on 10 distinct recent days — satisfies the
/// `minEvidenceSessions=10` leg of the minimum-evidence floor.
fn sufficient_evidence_sessions(store: &Path) {
    let now = now_unix();
    let lines: Vec<String> = (0..10).map(|i| session_line(now - i * DAY)).collect();
    append_telemetry(store, &lines);
}

// =============================================================================
// Floor 1 + Floor 2: zero-fire never demotes; a real low-read-rate memory does,
// but ONLY once the minimum-evidence floor is met.
// =============================================================================

#[test]
fn zero_fire_floor_never_demotes_while_low_read_rate_does_once_evidence_is_sufficient() {
    let store = unique_dir("floors");
    let cfg = Config::default();

    // GOOD (zero-fire floor holds): a memory with NO fires at all in the window.
    write_memory(&store, "zero-fire-mem", "", Some(0));
    let zero_fire_original = read_memory(&store, "zero-fire-mem");

    // BAD contrast (the floor is doing real work, not "nothing ever demotes"):
    // a memory that DOES fire, with a read rate at/under the 0.05 demote floor.
    write_memory(&store, "low-read-mem", "", Some(0));

    let now = now_unix();
    // low-read-mem fires 4 times, read back 0 times -> rate 0.0 <= 0.05 -> demote.
    let mut lines: Vec<String> = (0..4).map(|i| fire_line(now - i, "low-read-mem")).collect();
    // zero-fire-mem gets NO fire lines at all.
    // A read-only record for zero-fire-mem (no matching fire) must not matter.
    lines.push(read_line(now, "zero-fire-mem"));
    append_telemetry(&store, &lines);

    // First: with INSUFFICIENT evidence, `maintain` must mutate NOTHING at all
    // (not even the below-floor memory), and report InsufficientEvidence.
    let outcome = curation::maintain(&store, &cfg, true);
    assert!(
        matches!(outcome, MaintainOutcome::InsufficientEvidence { .. }),
        "expected insufficient evidence before any session markers: {outcome:?}"
    );
    assert_eq!(
        read_memory(&store, "low-read-mem"),
        {
            let mut s = String::from("---\nmetadata:\n  tags: [seat]\n  declineCount: 0\n---\n");
            s.push_str("BODY for low-read-mem — must never change.\n");
            s
        },
        "no mutation while evidence is insufficient"
    );

    // Now supply sufficient evidence and re-run (force=true bypasses the
    // record-count trigger, never the evidence floor).
    sufficient_evidence_sessions(&store);
    let outcome = curation::maintain(&store, &cfg, true);
    let (promoted, demoted, zero_fire) = match outcome {
        MaintainOutcome::Ran {
            promoted,
            demoted,
            zero_fire,
            ..
        } => (promoted, demoted, zero_fire),
        other => panic!("expected Ran once evidence is sufficient: {other:?}"),
    };
    assert!(promoted.is_empty());
    assert!(
        zero_fire.contains(&"zero-fire-mem".to_string()),
        "zero-fire memory must be reported zero-fire: {zero_fire:?}"
    );
    assert!(
        !demoted.contains(&"zero-fire-mem".to_string()),
        "D-43: a zero-fire memory must NEVER be demoted"
    );
    assert!(
        demoted.contains(&"low-read-mem".to_string()),
        "a real fired-but-unread memory MUST be demoted: {demoted:?}"
    );

    // D7: declineCount actually incremented for the demoted memory...
    let mutated = read_memory(&store, "low-read-mem");
    assert!(mutated.contains("declineCount: 1"));
    // ...and the zero-fire memory's frontmatter is untouched (declineCount still 0).
    let zero_fire_after = read_memory(&store, "zero-fire-mem");
    assert_eq!(
        zero_fire_after, zero_fire_original,
        "D7: a zero-fire memory's file must be byte-identical (never touched)"
    );
    // D7 (never-rewrites): the BODY of the mutated memory is byte-identical.
    assert_eq!(
        body_of(&mutated),
        "BODY for low-read-mem — must never change.\n"
    );
}

// =============================================================================
// FIX 2 — the minimum-evidence "≥30 days span" leg reads UNWINDOWED telemetry,
// not the (always ≤30d) windowed slice, so a long-observed-but-few-session-days
// store can still clear the floor via span alone.
// =============================================================================

#[test]
fn min_evidence_span_leg_fires_from_unwindowed_telemetry_not_the_30d_window() {
    let store = unique_dir("span-leg");
    let cfg = Config::default();
    let now = now_unix();

    // A real fired-but-unread memory: 4 recent fires (inside any window), 0
    // reads -> rate 0.0 <= demoteThreshold(0.05) -> demote-eligible once the
    // evidence floor clears.
    write_memory(&store, "stale-mem", "", Some(0));
    let mut lines: Vec<String> = (0..4).map(|i| fire_line(now - i, "stale-mem")).collect();

    // Only 3 distinct session-days -> WELL under minEvidenceSessions(10). One of
    // them is 40 days old (oldest record overall) -> span ~40d >= minEvidenceDays
    // (30) -> the span leg alone must clear the OR-guard, even though the
    // windowed reader (`min(30d, rotation bound)`) would never see that day-40
    // marker or let span exceed ~30 by construction.
    lines.push(session_line(now));
    lines.push(session_line(now - DAY));
    lines.push(session_line(now - 40 * DAY));
    append_telemetry(&store, &lines);

    let outcome = curation::maintain(&store, &cfg, true);
    let demoted = match outcome {
        MaintainOutcome::Ran { demoted, .. } => demoted,
        other => panic!("expected the span leg to clear the evidence floor and Ran: {other:?}"),
    };
    assert!(
        demoted.contains(&"stale-mem".to_string()),
        "the fired-but-unread memory must be demoted once the span leg passes: {demoted:?}"
    );
    let mutated = read_memory(&store, "stale-mem");
    assert!(mutated.contains("declineCount: 1"));

    // Contrast (still blocks): <10 session-days AND <30d span -> InsufficientEvidence.
    let blocked_store = unique_dir("span-leg-blocked");
    write_memory(&blocked_store, "stale-mem-2", "", Some(0));
    let mut blocked_lines: Vec<String> =
        (0..4).map(|i| fire_line(now - i, "stale-mem-2")).collect();
    // 3 distinct session-days, all recent -> span only ~2 days, well under 30.
    blocked_lines.push(session_line(now));
    blocked_lines.push(session_line(now - DAY));
    blocked_lines.push(session_line(now - 2 * DAY));
    append_telemetry(&blocked_store, &blocked_lines);

    let blocked_outcome = curation::maintain(&blocked_store, &cfg, true);
    assert!(
        matches!(
            blocked_outcome,
            MaintainOutcome::InsufficientEvidence { .. }
        ),
        "< 10 session-days AND < 30d span must still block: {blocked_outcome:?}"
    );
}

// =============================================================================
// Seat dual-gate: covered+high-fire demotes; covered+low-fire and
// high-fire+uncovered both do NOT.
// =============================================================================

#[test]
fn seat_dual_gate_demotes_only_when_covered_and_fires_meet_threshold() {
    let store = unique_dir("seats");
    let cfg = Config::default();

    // Three seats, each with distinct evidence shapes:
    // - covered (a real `commands` trigger) + fires >= seatPromoteMinFires(5)  -> DEMOTE
    // - covered + fires BELOW the threshold                                    -> no demote
    // - fires >= threshold but UNCOVERED (only a synonym; no derivable probe)   -> no demote
    write_memory(
        &store,
        "seat-covered-highfire",
        "    commands: [bazcmd]\n",
        Some(0),
    );
    write_memory(
        &store,
        "seat-covered-lowfire",
        "    commands: [barcmd]\n",
        Some(0),
    );
    write_memory(
        &store,
        "seat-uncovered-highfire",
        "    synonyms: [zzz-only-synonym]\n",
        Some(0),
    );

    fs::write(
        store.join("MEMORY.md"),
        "# Memory Router\n\n## Always-relevant entries\n\n\
         - [Covered, high fire](seat-covered-highfire.md)\n\
         - [Covered, low fire](seat-covered-lowfire.md)\n\
         - [Uncovered, high fire](seat-uncovered-highfire.md)\n",
    )
    .unwrap();

    // Real end-to-end index: an empty grammar (per-memory triggers route
    // independent of grammar tags) + a real `rebuild`, so `recall` (the seat
    // probe) reads the ACTUAL compiled index, not a hand-rolled fixture.
    let grammar_path = store.join("_grammar.toml");
    fs::write(&grammar_path, EMPTY_GRAMMAR_SEED).unwrap();
    rebuild(&store, &grammar_path, &BuildConfig::default()).expect("rebuild");

    let now = now_unix();
    let mut lines = Vec::new();
    for i in 0..7 {
        lines.push(fire_line(now - i, "seat-covered-highfire")); // 7 >= 5
    }
    for i in 0..2 {
        lines.push(fire_line(now - i, "seat-covered-lowfire")); // 2 < 5
    }
    for i in 0..7 {
        lines.push(fire_line(now - i, "seat-uncovered-highfire")); // 7 >= 5, but uncovered
    }
    append_telemetry(&store, &lines);
    sufficient_evidence_sessions(&store);

    let outcome = curation::seats(&store, &cfg, true).expect("seats");
    let (demote, probes, written) = match outcome {
        SeatsOutcome::Ran {
            demote,
            probes,
            written,
        } => (demote, probes, written),
        other => panic!("expected Ran: {other:?}"),
    };
    assert!(written);
    assert_eq!(probes.len(), 3);

    let probe = |stem: &str| probes.iter().find(|p| p.stem == stem).unwrap();
    assert!(probe("seat-covered-highfire").covered);
    assert_eq!(probe("seat-covered-highfire").fire_count, 7);
    assert!(probe("seat-covered-lowfire").covered);
    assert_eq!(probe("seat-covered-lowfire").fire_count, 2);
    assert!(
        !probe("seat-uncovered-highfire").covered,
        "a synonym-only seat has no derivable probe -> not covered"
    );

    assert_eq!(
        demote,
        vec!["seat-covered-highfire".to_string()],
        "ONLY the covered + >=5-fires seat is proposed for demotion: {demote:?}"
    );

    // The PENDING-SEAT-CHANGES block reflects exactly that one proposal.
    let router = fs::read_to_string(store.join("MEMORY.md")).unwrap();
    assert!(router.contains("<!-- PENDING-SEAT-CHANGES"));
    assert!(router.contains("seat-covered-highfire.md"));
    assert!(!router.contains("seat-covered-lowfire.md —"));
    assert!(!router.contains("DEMOTE: seat-uncovered-highfire"));

    // Replace-not-stack: a second run with the SAME result must not stack a
    // second block, and non-block content stays byte-identical.
    curation::seats(&store, &cfg, true).unwrap();
    let router_second = fs::read_to_string(store.join("MEMORY.md")).unwrap();
    assert_eq!(
        router_second.matches("<!-- PENDING-SEAT-CHANGES").count(),
        1,
        "re-run must replace, not stack"
    );
    let non_block_first = router.rsplit_once("-->\n").unwrap().1;
    let non_block_second = router_second.rsplit_once("-->\n").unwrap().1;
    assert_eq!(
        non_block_first, non_block_second,
        "non-block MEMORY.md content must be byte-identical across re-runs"
    );

    // D7: seat memory bodies were never touched by seat governance either.
    for stem in [
        "seat-covered-highfire",
        "seat-covered-lowfire",
        "seat-uncovered-highfire",
    ] {
        let content = read_memory(&store, stem);
        assert_eq!(
            body_of(&content),
            format!("BODY for {stem} — must never change.\n")
        );
    }
}

// =============================================================================
// FIX 3 — `seatPromoteMinFires = 0` + a covered ZERO-FIRE seat must not panic
// (`fires_by_id[s]` indexing a stem `count_fires_by_id` never inserted) and
// must NEVER propose that zero-fire seat for demotion (the zero-fire floor
// applies to seats too, regardless of how low `seatPromoteMinFires` is set).
// =============================================================================

#[test]
fn seat_promote_min_fires_zero_does_not_panic_and_never_demotes_a_zero_fire_seat() {
    let store = unique_dir("seats-zerofire-zerothreshold");
    let cfg = Config {
        seat_promote_min_fires: 0,
        ..Config::default()
    };

    // A covered seat (a real derivable `commands` probe) that NEVER fires at
    // all — `count_fires_by_id` never inserts an entry for it.
    write_memory(
        &store,
        "seat-covered-zerofire",
        "    commands: [quxcmd]\n",
        Some(0),
    );
    fs::write(
        store.join("MEMORY.md"),
        "# Memory Router\n\n## Always-relevant entries\n\n\
         - [Covered, zero fire](seat-covered-zerofire.md)\n",
    )
    .unwrap();

    let grammar_path = store.join("_grammar.toml");
    fs::write(&grammar_path, EMPTY_GRAMMAR_SEED).unwrap();
    rebuild(&store, &grammar_path, &BuildConfig::default()).expect("rebuild");

    // NO fire telemetry for this seat at all — only enough evidence (session
    // markers) to clear the minimum-evidence floor so the dual-gate itself is
    // actually exercised.
    sufficient_evidence_sessions(&store);

    // Must exit cleanly (no panic) and propose zero demotions.
    let outcome = curation::seats(&store, &cfg, true).expect("seats must not panic or error");
    let (demote, probes, written) = match outcome {
        SeatsOutcome::Ran {
            demote,
            probes,
            written,
        } => (demote, probes, written),
        other => panic!("expected Ran: {other:?}"),
    };
    assert!(written);
    assert_eq!(probes.len(), 1);
    assert!(
        probes[0].covered,
        "the seat's own `commands` trigger must probe as covered"
    );
    assert_eq!(probes[0].fire_count, 0);
    assert!(
        demote.is_empty(),
        "a zero-fire seat must NEVER be demote-eligible, even with seatPromoteMinFires=0: {demote:?}"
    );

    let router = fs::read_to_string(store.join("MEMORY.md")).unwrap();
    assert!(
        !router.contains("DEMOTE: seat-covered-zerofire"),
        "no demotion proposal must be written for the zero-fire seat"
    );
}

// =============================================================================
// FIX 4 — a re-run of `seats --propose` must not eat MEMORY.md's own leading
// blank lines (the non-block region stays byte-identical across re-runs, §8).
// =============================================================================

#[test]
fn seats_propose_twice_preserves_memory_md_leading_blank_lines() {
    let store = unique_dir("seats-leading-nl");
    let cfg = Config::default();

    write_memory(&store, "seat-a", "    commands: [quuxcmd]\n", Some(0));
    // The real (non-block) MEMORY.md content starts with TWO blank lines.
    fs::write(
        store.join("MEMORY.md"),
        "\n\n# Memory Router\n\n## Always-relevant entries\n\n- [A](seat-a.md)\n",
    )
    .unwrap();

    let grammar_path = store.join("_grammar.toml");
    fs::write(&grammar_path, EMPTY_GRAMMAR_SEED).unwrap();
    rebuild(&store, &grammar_path, &BuildConfig::default()).expect("rebuild");
    sufficient_evidence_sessions(&store);

    // Covered + >= seatPromoteMinFires(5) fires -> a real demotion proposal, so
    // a pending block actually gets written (an empty proposal set would be a
    // true no-op over an already-block-free file, defeating this test).
    let now = now_unix();
    let lines: Vec<String> = (0..6).map(|i| fire_line(now - i, "seat-a")).collect();
    append_telemetry(&store, &lines);

    // First `seats --propose`.
    curation::seats(&store, &cfg, true).expect("first seats run");
    let after_first = fs::read_to_string(store.join("MEMORY.md")).unwrap();
    let non_block_first = after_first
        .rsplit_once("-->\n")
        .map(|(_, rest)| rest)
        .unwrap_or(after_first.as_str());

    // Second `seats --propose` re-runs strip -> re-prepend over its OWN output.
    curation::seats(&store, &cfg, true).expect("second seats run");
    let after_second = fs::read_to_string(store.join("MEMORY.md")).unwrap();
    assert_eq!(
        after_second.matches("<!-- PENDING-SEAT-CHANGES").count(),
        1,
        "exactly one pending block after two runs"
    );
    let non_block_second = after_second
        .rsplit_once("-->\n")
        .map(|(_, rest)| rest)
        .unwrap_or(after_second.as_str());

    assert_eq!(
        non_block_first, non_block_second,
        "the non-block region (incl. MEMORY.md's leading blank lines) must be \
         byte-identical across both runs"
    );
    assert!(
        non_block_second.starts_with("\n\n# Memory Router"),
        "the leading blank lines must survive: {non_block_second:?}"
    );
}

// =============================================================================
// A second `maintain` while a fresh lock is held no-ops (WR-02 mutual exclusion,
// exercised through the public API by pre-planting a lock file).
// =============================================================================

#[test]
fn maintain_no_ops_while_another_pass_holds_the_lock() {
    let store = unique_dir("lockheld");
    let cfg = Config::default();
    // Pre-plant a FRESH lock file, as a concurrently-running pass would.
    fs::write(store.join("_maintenance_state.json.lock"), b"").unwrap();

    let outcome = curation::maintain(&store, &cfg, true);
    assert_eq!(outcome, MaintainOutcome::LockHeld);
}

// =============================================================================
// Below the ≥50-record trigger, an unforced `maintain` is a pure no-op.
// =============================================================================

#[test]
fn maintain_below_trigger_is_a_noop_without_force() {
    let store = unique_dir("belowtrigger");
    let cfg = Config::default();
    write_memory(&store, "some-mem", "", Some(0));
    // No telemetry file at all -> 0 lines, below the 50-record trigger.
    let outcome = curation::maintain(&store, &cfg, false);
    assert_eq!(outcome, MaintainOutcome::BelowTrigger);
    // Untouched.
    assert!(read_memory(&store, "some-mem").contains("declineCount: 0"));
}
