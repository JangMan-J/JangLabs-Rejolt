//! Conformance for the marks + telemetry primitive (WP-2b / P11; D25, A7, D7,
//! R7, §8). Covers the WP-2b gate rows: correlation/zero-fire, rotation +
//! `.1`-first read order (WR-04), mark-dir writability + inert advisory, bad-ts
//! symmetric drop (WR-05), TTL single-source (D25), and the R7 window bound —
//! plus the adversarial-verify lock-ins: per-line append atomicity + reader
//! corruption bounding (FIX 1), injective marks / no cross-memory credit (FIX 2),
//! `log_fire` gating on `record.mems` (FIX 3), `O_NOFOLLOW` + write/advisory
//! agreement on an unsafe dir (FIX 4), and session-marker surfacing (FIX 5).
//!
//! Every location is INJECTED at a temp dir (never the real `$XDG_RUNTIME_DIR`).

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use rejolt::config::Config;
use rejolt::conformance::{Check, assert_counts, fixtures_root};
use rejolt::telemetry::{
    FireMem, FireOutcome, FireRecord, ReadOutcome, ReadRecord, Telemetry, WindowBound,
    mark_filename,
};

// =============================================================================
// Helpers (temp dirs; no temp-file crate — WP-0 keeps the dep tree minimal)
// =============================================================================

/// A fresh, unique temp DIRECTORY for a test.
fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rejolt-wp2b-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A path to a fresh regular FILE (used as an unwritable "root": a mark dir under a
/// file cannot be created — ENOTDIR — so this is bulletproof regardless of uid,
/// unlike a chmod that root would bypass).
fn unique_file(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let f = std::env::temp_dir().join(format!(
        "rejolt-wp2b-blocker-{tag}-{}-{n}",
        std::process::id()
    ));
    fs::write(&f, b"i am a file, not a dir\n").expect("write blocker file");
    f
}

/// A Telemetry with a writable runtime dir + a telemetry file, both in temp.
fn writable_telemetry(tag: &str, config: Config) -> (Telemetry, PathBuf, PathBuf) {
    let base = unique_dir(tag);
    let runtime = base.join("run");
    let tel = base.join("_recall_telemetry.jsonl");
    (
        Telemetry::new(runtime.clone(), tel.clone(), config),
        runtime,
        tel,
    )
}

fn a_fire(ts: i64, qid: &str, mem_ids: &[&str]) -> FireRecord {
    FireRecord {
        ts,
        qid: qid.to_string(),
        mems: mem_ids
            .iter()
            .map(|id| FireMem {
                id: (*id).to_string(),
                tag: "gpu-tools".to_string(),
                trigger_type: "command".to_string(),
                val: "nvidia-smi".to_string(),
            })
            .collect(),
        conf: "high".to_string(),
    }
}

/// Write raw JSONL lines to `path` (for controlled-ts window-reader tests).
fn write_lines(path: &Path, lines: &[String]) {
    let mut s = String::new();
    for l in lines {
        s.push_str(l);
        s.push('\n');
    }
    fs::write(path, s).expect("write telemetry lines");
}

// =============================================================================
// G2: the window reader accepts a well-formed telemetry file, rejects an all-bad-
// ts one (WR-05). Runs through the G2 harness with real good/bad fixtures.
// =============================================================================

/// Predicate: reading this JSONL through the real `read_window` yields ≥1 in-window
/// fire/read record. Good fixture (integer ts, far-future so always in-window) →
/// true; bad fixture (unparseable ts on every fire/read) → all dropped → false.
fn telemetry_window_has_records(path: &Path) -> bool {
    let tel = Telemetry::new(unique_dir("g2-run"), path.to_path_buf(), Config::default());
    let w = tel.read_window();
    !w.fires.is_empty() || !w.reads.is_empty()
}

#[test]
fn g2_telemetry_window_reader_counts() {
    let check = Check::new(
        "telemetry-window-has-records",
        "telemetry",
        telemetry_window_has_records,
    );
    assert_counts(&check, &fixtures_root());
}

// =============================================================================
// Correlation invariant + zero-fire gate (A7/D25) + read gated on a live mark
// =============================================================================

#[test]
fn correlation_fire_logged_when_mark_persists_zerofire_when_not() {
    // GOOD: writable mark dir ⇒ mark persists ⇒ fire logged; the mark is live; a
    // read of that memory is then recorded (the mark IS the correlation).
    let (tel, _run, _telp) = writable_telemetry("corr-good", Config::default());
    let fire = a_fire(now(), "q1", &["gpu"]);
    assert_eq!(
        tel.log_fire(&fire),
        FireOutcome::Logged,
        "a persisted mark must log the fire"
    );
    assert!(tel.is_live("gpu"), "the fire must have left a live mark");

    let w = tel.read_window();
    assert_eq!(w.fires.len(), 1, "one fire logged");
    assert!(w.reads.is_empty(), "no read yet");

    assert_eq!(
        tel.log_read("gpu"),
        ReadOutcome::Logged,
        "a read of a live-marked memory is recorded"
    );
    let w2 = tel.read_window();
    assert_eq!(w2.reads.len(), 1, "the read is now recorded");

    // A read of a memory with NO live mark is NOT recorded (the correlation gate).
    assert_eq!(
        tel.log_read("never-fired"),
        ReadOutcome::NoLiveMark,
        "no live mark ⇒ no read record"
    );
    assert_eq!(
        tel.read_window().reads.len(),
        1,
        "the unmarked read did not append"
    );

    // BAD: an unwritable mark-dir root ⇒ no mark persists ⇒ ZERO-FIRE, no panic,
    // nothing logged (the fire never becomes fired-but-unread).
    let blocker = unique_file("corr-bad");
    let tel_bad = Telemetry::new(
        blocker.join("run"), // under a FILE ⇒ mkdir fails ⇒ mark cannot persist
        unique_dir("corr-bad-store").join("_recall_telemetry.jsonl"),
        Config::default(),
    );
    assert_eq!(
        tel_bad.log_fire(&a_fire(now(), "q2", &["gpu"])),
        FireOutcome::ZeroFire,
        "an unwritable mark dir must yield ZeroFire, never a logged fire"
    );
    assert!(!tel_bad.is_live("gpu"), "no mark could have persisted");
    assert!(
        tel_bad.read_window().fires.is_empty(),
        "ZeroFire logs nothing"
    );
}

// =============================================================================
// Rotation (§8) + WR-04 `.1`-first read order
// =============================================================================

#[test]
fn rotation_at_max_and_reads_dotone_before_live() {
    // Small tel_max_bytes so a handful of appends crosses the rotation threshold.
    let cfg = Config {
        tel_max_bytes: 200,
        ..Config::default()
    };
    let (tel, run, telp) = writable_telemetry("rot", cfg);
    let dotone = {
        let mut s = telp.clone().into_os_string();
        s.push(".1");
        PathBuf::from(s)
    };

    // Append fires until the live file rotates (its `.1` generation appears). Each
    // fire persists a mark first (writable dir), so every append is a real Logged.
    let mut appended = 0;
    for i in 0..50 {
        let out = tel.log_fire(&a_fire(now(), &format!("q{i}"), &["gpu"]));
        assert_eq!(out, FireOutcome::Logged);
        appended += 1;
        if dotone.exists() {
            break;
        }
    }
    assert!(
        dotone.exists(),
        "the live file must rotate to `.1` once it crosses tel_max_bytes (after {appended} appends)"
    );
    // The live file exists too (a fresh live file was appended to post-rotation).
    assert!(telp.exists(), "a fresh live file continues after rotation");
    // Marks kept working throughout (the dir stayed usable).
    assert!(run.exists());

    // WR-04 ordering: write `.1` and live directly with records that would SORT
    // differently by ts (the `.1` record's ts is LATER than live's). The reader
    // must still return the `.1` record FIRST — proving file order, not ts order.
    let (tel2, _run2, telp2) = writable_telemetry("wr04", Config::default());
    let dotone2 = {
        let mut s = telp2.clone().into_os_string();
        s.push(".1");
        PathBuf::from(s)
    };
    // Far-future ts (always in-window); `.1` ts is the LARGER value.
    write_lines(
        &dotone2,
        &[serde_json::to_string(&a_fire(9_000_000_002, "from-dotone", &["gpu"])).unwrap()],
    );
    write_lines(
        &telp2,
        &[serde_json::to_string(&a_fire(9_000_000_001, "from-live", &["gpu"])).unwrap()],
    );
    let w = tel2.read_window();
    assert_eq!(w.fires.len(), 2, "both generations parsed");
    assert_eq!(
        w.fires[0].qid, "from-dotone",
        "WR-04: the `.1` generation is read BEFORE live, regardless of ts order"
    );
    assert_eq!(w.fires[1].qid, "from-live");
}

// =============================================================================
// Writability probe + inert-telemetry advisory (A7)
// =============================================================================

#[test]
fn mark_dir_writable_and_inert_advisory() {
    // GOOD: a fresh temp runtime dir is writable; no inert advisory.
    let (tel, _run, _telp) = writable_telemetry("writ-good", Config::default());
    assert!(tel.mark_dir_writable(), "a fresh temp dir must be writable");
    assert!(
        tel.inert_telemetry_advisory().is_none(),
        "a writable mark dir must not raise the inert advisory"
    );

    // BAD: a runtime dir UNDER a regular file cannot be created ⇒ not writable ⇒
    // exactly one inert-telemetry advisory surfaces (A7).
    let blocker = unique_file("writ-bad");
    let tel_bad = Telemetry::new(
        blocker.join("run"),
        unique_dir("writ-bad-store").join("_recall_telemetry.jsonl"),
        Config::default(),
    );
    assert!(
        !tel_bad.mark_dir_writable(),
        "an un-creatable mark dir must not be writable"
    );
    let advisory = tel_bad
        .inert_telemetry_advisory()
        .expect("inert telemetry must raise one advisory");
    assert!(
        advisory.contains("inert"),
        "the advisory must name the inert-telemetry condition: {advisory}"
    );
}

// =============================================================================
// Bad-ts symmetric drop (WR-05): a fire AND a read with unparseable ts both drop;
// good-ts records of both kinds survive.
// =============================================================================

#[test]
fn bad_ts_drops_symmetrically() {
    let (tel, _run, telp) = writable_telemetry("badts", Config::default());
    // Far-future good ts (in-window forever); string ts is the bad-ts case.
    let good_fire = serde_json::to_string(&a_fire(9_000_000_000, "q-good", &["gpu"])).unwrap();
    let good_read = serde_json::to_string(&ReadRecord {
        ts: 9_000_000_000,
        id: "gpu".to_string(),
        signal: "read".to_string(),
    })
    .unwrap();
    let bad_fire = r#"{"ts":"oops","qid":"q-bad","mems":[{"id":"gpu","tag":"t","type":"command","val":"v"}],"conf":"low"}"#.to_string();
    let bad_read = r#"{"ts":"nope","id":"gpu","signal":"read"}"#.to_string();
    write_lines(&telp, &[good_fire, bad_fire, good_read, bad_read]);

    let w = tel.read_window();
    // Good-ts survive on BOTH sides.
    assert_eq!(w.fires.len(), 1, "the good-ts fire survives");
    assert_eq!(w.reads.len(), 1, "the good-ts read survives");
    assert_eq!(w.fires[0].qid, "q-good");
    // Bad-ts dropped on BOTH sides — symmetric (a fire AND a read).
    assert_eq!(
        w.dropped_bad_ts, 2,
        "WR-05: one bad-ts fire AND one bad-ts read must both drop"
    );
}

// =============================================================================
// TTL single-source (D25): Config::dedupe_ttl_seconds alone drives liveness;
// no second hardcoded TTL anywhere in the telemetry module.
// =============================================================================

#[test]
fn ttl_single_source_drives_liveness() {
    let base = unique_dir("ttl");
    let runtime = base.join("run");
    fs::create_dir_all(&runtime).unwrap();
    let telp = base.join("_recall_telemetry.jsonl");

    // Plant a mark backdated 1000s (so its age is a fixed, deterministic 1000s).
    let mark_path = runtime.join(mark_filename("gpu"));
    fs::write(&mark_path, b"").unwrap();
    let past = SystemTime::now()
        .checked_sub(Duration::from_secs(1000))
        .unwrap();
    let f = fs::OpenOptions::new()
        .write(true)
        .open(&mark_path)
        .expect("reopen the mark to set mtime");
    f.set_modified(past).expect("backdate the mark mtime");

    // TTL above the age ⇒ live; TTL below the age ⇒ not live. Same mark, same code,
    // only the config knob changes — proving the TTL is the sole liveness input.
    let tel_live = Telemetry::new(
        runtime.clone(),
        telp.clone(),
        Config {
            dedupe_ttl_seconds: 2000,
            ..Config::default()
        },
    );
    let tel_dead = Telemetry::new(
        runtime.clone(),
        telp.clone(),
        Config {
            dedupe_ttl_seconds: 500,
            ..Config::default()
        },
    );
    assert!(tel_live.is_live("gpu"), "age 1000s < TTL 2000s ⇒ live");
    assert!(!tel_dead.is_live("gpu"), "age 1000s > TTL 500s ⇒ not live");

    // No second hardcoded TTL: the module source must not carry the default TTL
    // literal (900) nor synapse's `-mmin -15` divergence — the whole D25 lesson.
    let src = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/telemetry.rs"))
        .expect("read telemetry source");
    assert!(
        !src.contains("900"),
        "telemetry.rs must not hardcode the TTL (900) — it reads Config::dedupe_ttl_seconds only"
    );
    assert!(
        !src.contains("mmin"),
        "telemetry.rs must not carry synapse's `-mmin -15` hardcode"
    );
}

// =============================================================================
// R7 window bound: a small tel_max_bytes forces RotationCapped; a large one gives
// TimeCapped. The exposed bound is correct on the SAME data.
// =============================================================================

#[test]
fn window_bound_rotation_vs_time_capped() {
    let base = unique_dir("bound");
    let runtime = base.join("run");
    let telp = base.join("_recall_telemetry.jsonl");

    // Several recent-ts records (all in-window) totalling a few hundred bytes.
    let now = now();
    let lines: Vec<String> = (0..6)
        .map(|i| serde_json::to_string(&a_fire(now - i, &format!("q{i}"), &["gpu"])).unwrap())
        .collect();
    write_lines(&telp, &lines);
    let total: usize = lines.iter().map(|l| l.len() + 1).sum();
    assert!(
        total > 100,
        "sanity: fixture is larger than the small budget"
    );

    // Small tel_max_bytes ⇒ 2×budget < total ⇒ rotation binds; its cutoff (recent)
    // beats the 30-day time cutoff ⇒ RotationCapped.
    let small = Telemetry::new(
        runtime.clone(),
        telp.clone(),
        Config {
            tel_max_bytes: 50,
            ..Config::default()
        },
    );
    assert_eq!(
        small.read_window().bound,
        WindowBound::RotationCapped,
        "a tiny tel_max_bytes must rotation-cap the window"
    );

    // Large tel_max_bytes ⇒ total << 2×budget ⇒ rotation never binds ⇒ TimeCapped.
    let large = Telemetry::new(
        runtime.clone(),
        telp.clone(),
        Config {
            tel_max_bytes: 1_000_000,
            ..Config::default()
        },
    );
    assert_eq!(
        large.read_window().bound,
        WindowBound::TimeCapped,
        "a large tel_max_bytes must leave the 30-day time window binding"
    );
}

// =============================================================================
// FIX 1 — reader bounds corruption to the offending line (torn concatenation +
// invalid UTF-8), so a torn append never sinks the whole generation.
// =============================================================================

#[test]
fn reader_bounds_corruption_to_the_offending_line() {
    // (a) A concatenated `{A}{B}` physical line (what two non-atomic appends could
    //     interleave into) is invalid JSON and drops ONLY itself — the surrounding
    //     good fire + read still parse.
    let (tel, _run, telp) = writable_telemetry("corrupt-json", Config::default());
    let good_fire = serde_json::to_string(&a_fire(9_000_000_000, "q-good", &["gpu"])).unwrap();
    let a = serde_json::to_string(&a_fire(9_000_000_000, "qa", &["gpu"])).unwrap();
    let b = serde_json::to_string(&a_fire(9_000_000_000, "qb", &["gpu"])).unwrap();
    let torn = format!("{a}{b}"); // no newline between → one torn physical line
    let good_read = serde_json::to_string(&ReadRecord {
        ts: 9_000_000_000,
        id: "gpu".to_string(),
        signal: "read".to_string(),
    })
    .unwrap();
    write_lines(&telp, &[good_fire, torn, good_read]);

    let w = tel.read_window();
    assert_eq!(
        w.fires.len(),
        1,
        "only the clean fire survives; the torn {{A}}{{B}} line drops itself"
    );
    assert_eq!(w.fires[0].qid, "q-good");
    assert_eq!(
        w.reads.len(),
        1,
        "the read after the torn line still parses"
    );

    // (b) An invalid-UTF-8 byte in one line must NOT drop the whole generation the
    //     way `read_to_string` would have.
    let (tel2, _run2, telp2) = writable_telemetry("corrupt-utf8", Config::default());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(
        serde_json::to_string(&a_fire(9_000_000_000, "q1", &["gpu"]))
            .unwrap()
            .as_bytes(),
    );
    bytes.push(b'\n');
    bytes.extend_from_slice(&[0xFF, 0xFE, b'{', b'}']); // invalid UTF-8 line
    bytes.push(b'\n');
    bytes.extend_from_slice(
        serde_json::to_string(&ReadRecord {
            ts: 9_000_000_000,
            id: "gpu".to_string(),
            signal: "read".to_string(),
        })
        .unwrap()
        .as_bytes(),
    );
    bytes.push(b'\n');
    fs::write(&telp2, &bytes).unwrap();

    let w2 = tel2.read_window();
    assert_eq!(
        w2.fires.len(),
        1,
        "an invalid-UTF-8 line must not sink the generation"
    );
    assert_eq!(w2.reads.len(), 1);
}

// =============================================================================
// FIX 2 — injective marks prevent cross-memory correlation credit (D25 per-memory)
// =============================================================================

#[test]
fn injective_marks_prevent_cross_memory_credit() {
    // Distinct legal store stems the OLD collapse-to-`_` mapping collided.
    assert_ne!(mark_filename("gpu notes"), mark_filename("gpu_notes"));
    assert_ne!(mark_filename("a/b"), mark_filename("a-b"));

    let (tel, _run, _telp) = writable_telemetry("injective", Config::default());
    // Fire on "gpu notes" ONLY.
    assert_eq!(
        tel.log_fire(&a_fire(now(), "q", &["gpu notes"])),
        FireOutcome::Logged
    );
    assert!(tel.is_live("gpu notes"), "the fired memory has a live mark");
    assert!(
        !tel.is_live("gpu_notes"),
        "a DIFFERENT memory must NOT be mis-credited live via a collided mark"
    );
    // And a read of the un-fired sibling is not recorded (no cross-credit).
    assert_eq!(tel.log_read("gpu_notes"), ReadOutcome::NoLiveMark);
}

// =============================================================================
// FIX 3 — log_fire gates on record.mems (marks EVERY credited memory; empty → ZeroFire)
// =============================================================================

#[test]
fn log_fire_marks_every_credited_memory_and_zerofires_empty() {
    let (tel, _run, _telp) = writable_telemetry("mems-gate", Config::default());
    // A record crediting [a, b] must write a mark for BOTH.
    assert_eq!(
        tel.log_fire(&a_fire(now(), "q", &["a", "b"])),
        FireOutcome::Logged
    );
    assert!(tel.is_live("a"), "memory a credited ⇒ marked");
    assert!(tel.is_live("b"), "memory b credited ⇒ marked");

    // Empty mems credits nobody ⇒ ZeroFire (never log a fire that credits no one).
    let empty = FireRecord {
        ts: now(),
        qid: "q-empty".to_string(),
        mems: vec![],
        conf: "low".to_string(),
    };
    assert_eq!(tel.log_fire(&empty), FireOutcome::ZeroFire);
    assert!(
        tel.read_window().fires.iter().all(|f| f.qid != "q-empty"),
        "the empty-mems fire must not have been logged"
    );
}

// =============================================================================
// FIX 4 — O_NOFOLLOW refuses a symlinked mark (no target truncation); an unsafe
// (world-writable) runtime dir is inert on BOTH the write path and the advisory.
// =============================================================================

#[test]
fn nofollow_refuses_symlink_and_unsafe_dir_is_inert_everywhere() {
    // (a) A symlinked mark path is refused at open() (O_NOFOLLOW): the target file
    //     is NOT truncated, and the fire ZeroFires (its only mem got no mark).
    let base = unique_dir("nofollow");
    let runtime = base.join("run");
    fs::create_dir_all(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let victim = base.join("victim.txt");
    fs::write(&victim, b"precious\n").unwrap();
    // Plant a symlink at EXACTLY the mark path for id "gpu".
    let mark_link = runtime.join(mark_filename("gpu"));
    symlink(&victim, &mark_link).unwrap();

    let tel = Telemetry::new(
        runtime.clone(),
        base.join("_recall_telemetry.jsonl"),
        Config::default(),
    );
    assert_eq!(
        tel.log_fire(&a_fire(now(), "q", &["gpu"])),
        FireOutcome::ZeroFire,
        "a symlinked mark path must be refused ⇒ no mark ⇒ ZeroFire"
    );
    assert_eq!(
        fs::read_to_string(&victim).unwrap(),
        "precious\n",
        "O_NOFOLLOW must prevent truncating the symlink target"
    );

    // (b) A group/world-writable runtime dir is UNSAFE: the write path ZeroFires AND
    //     the advisory reports inert — they AGREE (no silent write into an unsafe dir).
    let base2 = unique_dir("unsafe-dir");
    let runtime2 = base2.join("run");
    fs::create_dir(&runtime2).unwrap();
    fs::set_permissions(&runtime2, fs::Permissions::from_mode(0o777)).unwrap();
    let tel2 = Telemetry::new(
        runtime2.clone(),
        base2.join("_recall_telemetry.jsonl"),
        Config::default(),
    );
    assert_eq!(
        tel2.log_fire(&a_fire(now(), "q", &["gpu"])),
        FireOutcome::ZeroFire,
        "an unsafe (world-writable) mark dir must not be written into"
    );
    assert!(
        tel2.inert_telemetry_advisory().is_some(),
        "the advisory must agree the dir is inert"
    );
    assert!(
        !tel2.mark_dir_writable(),
        "and mark_dir_writable must agree the dir is unsafe"
    );
}

// =============================================================================
// FIX 5 — session markers surface for WP-6 floor-2, not counted as fires/reads.
// =============================================================================

#[test]
fn session_markers_surface_and_are_not_fires_or_reads() {
    let (tel, _run, _telp) = writable_telemetry("sessions", Config::default());
    assert!(tel.log_session());
    assert!(tel.log_session());

    let w = tel.read_window();
    assert_eq!(
        w.sessions.len(),
        2,
        "in-window session markers surface for WP-6 floor-2"
    );
    assert!(w.fires.is_empty(), "a session is not a fire");
    assert!(w.reads.is_empty(), "a session is not a read");
    assert!(w.sessions.iter().all(|s| s.signal == "session"));
}

/// Now, as a unix timestamp (seconds) — matches the module's internal clock.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
