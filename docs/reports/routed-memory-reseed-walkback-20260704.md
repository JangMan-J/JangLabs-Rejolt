# routed-memory-reseed — walk-back report

**Date:** 2026-07-04. **Tier:** T2.
**Model:** Claude Fable 5 (`claude-fable-5`) — meets the §8/G8 T2 floor (fable). Mechanical fan-out (diff inventory + three risky-bit checkers) pinned to `model: opus`; the Fable session adjudicated.
**Inputs:** SPEC `docs/frozen/routed-memory-reseed-decisions-20260703.md` (+ A-ledger `docs/frozen/routed-memory-reseed-amendments.md`, A1–A7), PLAN `docs/frozen/routed-memory-reseed-plan-20260704.md`, DIFF `69184e7..HEAD-at-entry` (`2654da5`; 22 commits, 132 files, ~20.8k insertions), RISKY BITS = plan register RB1–RB11 (floor) **+ gate-nominated from the diff**: (RC-a) the store-resolution/at-home interpretation in `hook.rs`; (RC-b) the newest integration fix `2654da5` in `bench.rs` (A4(c) arithmetic, post-verify code); (RC-c) the citation `trigger_type`-vs-`type` column resolution; (RC-d) cross-packet directive compliance (`log_fire(record)`, `WindowedTelemetry.sessions`, `is_full_write` Write-only + guard double-check); (RC-e) `.gitignore`/`Cargo.lock` contents; (RC-f) the frozen-plan file edit inside the diff.
**Mode:** T2 full three-pass + Codex counter-lens + N1–N14 sweep + runtime verification.
**Counter-lens (registry §5 / ADR 0008):** RAN — headless Codex via multipass (`~/JangLabs/multipass/target/release/multipass`, fresh session `vet-rmr-cl`, primer `workflow/counter-lens-primer.md`) over the risky-bit union + diff. 2 findings, both verified real: → F3 (byPath anchor-only broad globs) and F6 (qid omits synonyms). Dispositions in the table below. No §7 downgrade.

<!-- Produced by /vet wrapping /walk-back. Findings are dispositioned in
     the fix loop below — a finding may be FIXED (commit ref) or DEFERRED
     (written reason). Silent drops are illegal. -->

## Orphan changes (diff → plan)

**None.** Every changed file maps to a P-item: 22 src modules → P1–P15 (incl. `tier.rs` → P4/P5 Appendix A vocabulary), 19 test files → their packet gates, ~70 fixtures → the G2 good/bad corpus per parser/classifier, 4 reports → stage docs, `.gitignore` +`/target` → P1. Cargo dependencies are exactly `{clap, libc, serde, serde_json, toml}` — all statically linked build-time crates (libc recorded at WP-2b for O_NOFOLLOW/geteuid), no `[dev-dependencies]`, N10's test pins the set; the PyYAML oracle is out-of-process test-harness-only per B2. `Cargo.lock`'s 39 crates are the expected resolver tree (no sqlite/pyo3/tokio/embedding). The frozen-plan edit inside the diff (RC-f) is only the `Freeze-commit: pending → 69184e7` hash landing — benign. All 22 commits map to a WP / integration / fix / docs purpose (the build report's WP→commit table is a headline-per-WP simplification of traceable parallel-worktree mechanics).

## Under-built items (plan item with no/weak change)

**None.** All 18 P-items carry real, non-vacuous implementations; zero `todo!()`/`unimplemented!()`/panic-stubs in src. Spot-verified with teeth checks (opened, read, invert-would-fail): §14 rows 6/G, K, 12/13/15, 20 and N-rows N1, N5, N6, N13, N14; the WP-2 drift-guardrail stubs were genuinely tightened by WP-4 (`would_block_degenerate` drives the real projection). Two honestly-declared residuals are dispositioned below (RB1(b); §14 row 23), not build gaps. Two weak-teeth notes, both covered elsewhere: `tests/flat_index.rs::rb9_recall_equals_projection_single_walk` is near-tautological (`walk(q)==walk(q)`), but `tests/projection.rs::rb9_projection_uses_the_same_walk_as_recall` and N1's behavioral test carry the contract; `kill_switch_suppresses_post_op_rebuild_refresh` lacks an inline GOOD contrast, but `hook_dispatch.rs` proves the un-suppressed path.

## Plan drift (plan → spec)

All of P1–P18 still trace to their D/A anchors; nothing quietly grew. Four recorded reconciliations (none a spec violation):

1. **Builder-tier reconciliation** — plan Budget says "packet builders sonnet"; the build ran Opus builders per WORKFLOW.md v1.6 §8 (ADR 0007) supersession. A workflow-registry resolution, recorded in the build report; not a G4 amendment.
2. **§14 row 23 `byMemoryId` lifecycle** — CORE-SPEC names a feature the frozen plan's orphan ledger cut (D18/A2 removed the producer). Ledger precedence resolves it; the CORE-SPEC wording drift goes to /ship's R4 compaction. DEFERRED below.
3. **Appendix D `search` row** omits the universal exit-2 usage/config arm that D20's global taxonomy mandates and the code implements (`}{ not json` on stdin → 2, config error → 2). The code follows D20; the plan-appendix row is compressed. Recorded, no change.
4. **P6's citation literal `{type}`** was correctly resolved to the populated `trigger_type` axis column via the synapse tiebreaker (WP-2 friction #1); Appendix A's `type` column is the empty reserved memory-classification field. Verified in code and in the live advisory.

Also recorded: G7's worktree-commit rule was satisfied by immediate post-gate commits in the main tree + snapshot hooks (justification written in the build report); the WP-5/6 store-resolution + at-home floor interpretations were gate-nominated as RC-a and held under attack (fail-open on HOME unset / config missing / unparseable / relative paths — driven).

## Correctness findings on risky bits

Numbered F-findings; every one carries the concrete failing input that was demonstrated or traced. Severities: 1 BLOCKER, 3 MAJOR, 9 MINOR, 4 NIT + 2 doc classes (F24 landed as an addendum after the rb-guard resend).

- **F1 [process] RB1(b) live-host deny probe record MISSING.** The plan required the human-only probe at build START; the build report's record still reads `PENDING owner observation`. `/vet` verifies the record exists — it does not. The engine-contract half RB1(a) is fully fixtured and was re-driven here (deny = stderr + exit 2, short-circuit, never exit 1); what remains unproven is that the LIVE host build blocks on exit 2. **This is the sole owner-regret deferral — everything fail-closed rests on it.** Owner procedure (5 minutes): fresh Claude Code session → register a minimal PreToolUse hook that prints to stderr and exits 2 on a `Bash` matcher → run one Bash call → observe it blocked → record date/matcher/blocked in the build report's RB1(b) line.
- **F2 [BLOCKER] write guard unscoped by location** (`guard.rs::check_write` via `hook.rs::dispatch_pre_op`). Any full `Write` of `---`-fenced content ANYWHERE was shape-validated against the memory dialect. Demonstrated live pre-fix: a Jekyll page at `<tmp>/blog/2026-07-04-post.md` (`title:`/`layout:` keys) → **exit 2, "unknown top-level key `title`"**; rb-guard independently confirmed for `.markdown`, `.mdx`, and a `.yaml` manifest. Under the installed hooks every static-site page, Claude skill file, and multi-doc manifest write on the box would be blocked — mass false-deny, the RB3 #1-rule violation writ large. The synapse tiebreaker (`memory-write-guard.sh` D-14/CR-02) gates only watched locations and says so explicitly. All 270 pre-vet tests were green because no fixture exercised an out-of-store frontmatter write. Fixed at the adapter layer (`is_guard_scope`: grammar file by engine-realpath identity + box store + `*/.claude/projects/*/memory/` + repo `memory/` dirs, `.md` only); write-context scoped identically; `check-write` CLI deliberately stays an unscoped probe (seam documented, F17). Live re-drive: Jekyll page now exit 0 silent; in-store/project-store/repo-memory invalid writes still deny.
- **F3 [MAJOR] byPath anchor-only broad globs routed on every path** (`index.rs::path_scan_match`; counter-lens + rb-index convergence). `/**` (empty prefix) and `~/**` ($HOME prefix) passed the trailing-`/**` containment; `/*`/`~/*` reached fnmatch where `*` crosses `/`. Failing input: index row `byPath /**` + any op touching any absolute path → strong-tier fire, clearing the surface gate alone → advisory on essentially every host op (D5 flood) + projection breadth pollution; reachable via grammar path evidence (validate did not check broadness). `is_broad_path` classifies all four broad — routing and liveness disagreed. The synapse matcher shares the hole; D15 makes synapse reference-not-constraint and D5/§3.x are spec. Fixed: the walk consults the same §3.x classifier liveness reads; concrete-prefix `/**` still fires.
- **F4 [MAJOR] `log_fire` any-mark gating logged write-time fired-but-unread** (`telemetry.rs`; rb-telem). With ≥1 of `record.mems` persisting a mark, the FULL record was appended — crediting memories whose own `write_mark` failed (e.g. a stem whose percent-encoding exceeds NAME_MAX, co-surfaced with a normal memory). Such a memory is fired-but-unread from birth (`log_read` gates on the live mark), the exact state A7's corrected wording forbids at write time. Fixed: the logged record is rebuilt from the persisted subset; none persisted → ZeroFire.
- **F5 [MAJOR] `--update-baseline` laundered calibration across an env change** (`bench.rs`; rb-telem). It copied prior budget/slack unconditionally while stamping the CURRENT fingerprint: calibrate on AC/performance → switch to battery/powersave → update-baseline → the gate goes live under the new fingerprint using a slack floor never measured there — silently defeating the A4(d) mismatch detector (an A4(e) conformance failure). Fixed: carry only when `prior.env.matches(current)`; else inert + a loud line naming recalibration.
- **F6 [MINOR] qid omitted the synonyms bucket** (`recall.rs::compute_query_id`; counter-lens). All WebSearch/WebFetch/context7 queries (synonym-only routing) collapsed to one qid per tool, falsifying the function's own distinct-queries-differ contract. Telemetry-discriminator impact only (curation never reads qid). Fixed: all four buckets hash.
- **F7 [MINOR] parseable-nonsense baseline silently degraded the gate** (`bench.rs::Baseline::load`; rb-telem). TOML `nan`/`inf`/negative loaded fine; `NaN > 0.0` is false so REGRESSED silently disarmed. Fixed: non-finite/negative magnitudes → `None` → the LOUD NOBASELINE advisory.
- **F8 [MINOR] R7 rate measurement dropped a whole generation on one bad byte** (`bench.rs::measure_tel_rate`; rb-telem). `read_to_string` errored on one invalid-UTF-8 byte — the WP-2b anti-pattern reintroduced. Fixed: byte-read + per-line lossy decode, matching `read_window`.
- **F9 [MINOR] const-consumed config keys were silent no-ops** (`config.rs`; rb-telem). `collisionGuideFloor`/`DEDUP_BACKSTOP_THRESHOLD`/`WRITE_CONTEXT_BUDGET`/`BUDGET_MS` parse as known keys but their consumers are compile-time consts — an override changed nothing, warned nothing (the D25 declared-vs-honored divergence pattern). Fixed: the loud direct-CLI load emits one advisory per inert override; hook path stays silent.
- **F10 [MINOR] dead evidence died silently** (observed live at runtime verification). Grammar `args = ["--no-cache"]` produced no byArg row (`routing_key` → None, synapse `_norm` parity) with `rebuild: 0 excluded` and `validate: OK`; post-F3, broad path evidence is equally dead. Fixed: `validate` emits one finding per dead value (non-routable key / §3.x-broad glob) over grammar entries AND memory triggers — exit 1, advisory class.
- **F11 [NIT] second copy of the tier map** (`recall.rs::trigger_type_rank`; rb-index). Citation-representative ranking re-implemented axis→tier inline. Fixed: derived from `Axis::tier` (Appendix A "one module").
- **F12 [NIT] generation id ignored the build config** (`catalog.rs::generation_id`; rb-index). A crash between the pair's two writes across a `maxDescriptionChars` change left a mixed-config pair under EQUAL generations — false-Consistent past the A2(d) detector. Fixed: a config tag participates in the hash.
- **F13 [docs] three stale build-phase comments** contradicted shipped code (cli.rs "hook NOT-YET-WIRED"; rebuild.rs "WP-4 classifier stub" on the real predicate; hook.rs `maintenance_due` "not wired" + dangling INTEGRATOR pointer). Fixed.
- **F14 [MINOR] `\/` escape false-denied vs the named oracle** (`frontmatter.rs::parse_double_quoted`; rb-guard, live PyYAML 6.0.3 differential). PyYAML accepts `"path\/to"` → `path/to`; rejolt denied InvalidEscape — a false-deny on an in-subset construct (RB3 #1 class). Fixed: the no-op solidus joins the whitelist; decoding escapes (`\xNN`, `\uNNNN`) stay rejected because the parser does not decode them.
- **F15 [MINOR] no panic net on hook paths** (`hook.rs::dispatch`; rb-guard). A panic escaped as exit 101 + backtrace — non-blocking (N13 held) but non-SILENT, deviating from A5(b); the class is proven reachable (WP-6 verify caught a live seat-gate panic). Fixed: hook mode installs a no-op panic hook + `catch_unwind` → quiet exit-0 allow; direct CLIs keep loud panics.
- **F16 [MINOR] grammar tag NAMES skipped TAG_RE** (`grammar.rs::validate_grammar`; rb-guard). `[tool.GPU_Tools]` validated + rebuilt as dead vocabulary (kebab-enforced memory tags can never be members). Confirmed NOT an index-corruption vector (a memberless grammar tag emits zero rows). Plan Appendix A mandates "TAG_RE conformance at validate". Fixed: `InvalidTagName` (exit 2) + known-bad fixture; `error_signatures` mirrors it so the A6 diff guard participates.
- **F17 [docs] `check_write` scope seam documented** (rb-guard's seam concern on the F2 fix): scope lives in the adapter; `check_write` judges content unscoped by design (the `check-write` CLI is a loud explicit probe; `tests/write_guard.rs` exercises engine tiers directly, incl. the misplacement tier that at the hook level now only fires for in-scope non-box targets — synapse D-15 parity). Both forbidden drift directions named on the doc.
- **F18 [observed, deferred] calibration under-measures cross-invocation jitter.** Empirical, from runtime verification on the 1-memory toy store: five back-to-back in-process calibration runs measured `ceiling_slack_ms = 0.0017` while separate invocations swing 8.6→20 µs (powersave governor, cold caches) → the very next `bench` run falsely REGRESSED. The A4(c) formula is implemented faithfully (independently re-derived); the run MECHANICS under-measure the gate's own measurement conditions. At D16's reference scale (0.7–2.4 ms) the effect is unproven either way. DEFERRED to the real deployment-time calibration (see R1 below); if false-REGRESSED reproduces at real scale, that is a G4 amendment to A4's run mechanics, not a code patch.
- **F19 [MINOR, deferred] one Bash token can clear the ≥2 surface gate** (rb-index): a memory declaring the SAME value in both `args` and `synonyms` yields two distinct tuples from one token (`echo release` vs `{args:[release], synonyms:[release]}`). Faithful to the frozen "≥2 tuples total" wording, synapse dual-lookup parity, and only reachable by an author duplicating their own evidence across axes. DEFERRED: changing tuple counting risks false-silence elsewhere; recorded as a known corner.
- **F20 [NIT, deferred] pre-op guard (engine-realpath) vs post-op refresh (adapter-lexical) symlink seam** (rb-guard): a symlink-addressed store write is guarded pre-op but may skip the rebuild-refresh → stale catalog until the next rebuild. Fail-open, self-heals, and §5.x says the two canonicalizations differ on purpose. Recorded.
- **F21 [NIT, deferred] parser laxer than PyYAML on two malformed scalars** (`description: weird:` trailing colon; bracket mid flow-item) — fail-open direction (stores the literal), outside the B2 rejection surface and the differential corpus. Recorded as future corpus vectors.
- **F22 [NIT, deferred] UTF-8 BOM → not-a-memory** (PyYAML strips BOM; rejolt's fence check doesn't). Fails OPEN at the guard (no false-deny); a BOM'd memory is unguarded/unroutable but IS surfaced by `validate`/`scan_store` as malformed. Changing it touches the frozen B2 boundary — recorded for a future B2 amendment, not patched at /vet.
- **F23 [NIT, deferred] non-tmpfs mark dir accumulates dead `m_*` files** (`~/.cache` fallback when XDG_RUNTIME_DIR unset) — TTL correctness holds via mtime; disk hygiene only, by design (D25 tmpfs-first).
- **F24 [MINOR, addendum] misplacement tier gated `other` targets** (`guard.rs::misplacement_box_path`; rb-guard resend, adjudicated against the tiebreaker). The engine denied an all-box memory written to ANY target outside box_root; the ground-truth engine (`_classify_target`, `memory_surface.py:1480–1663`) runs the placement gate only for the `project-store`/`repo-memory` classes and passes `other` targets through — "no grammar authority, no gate". Post-F2 the hook path was already identical (out-of-scope never reaches the guard); the divergence lived in the `check-write` CLI probe (denied where the proven boundary allows) and in two tests pinning the over-broad semantics. Fixed: tier-5 fails open on `other`; the misplacement and §14 row-10 containment tests re-pinned with DENY-in-recognized-store + ALLOW-on-`other` arms (realpath containment keeps its teeth — a lexical check would classify the symlink escape as in-box).

**Cleared under attack** (each actively probed, not vibes; checker attack lists preserved in their reports): RB2 (hostile content in every column, `\r` everywhere `\n` is, comma-in-tag unreachable, whole-memory exclusion keeps the index well-formed), RB4 (torn-pair symmetric detection, atomic write with fsync, deterministic retry can't mix), RB9/N1 (one walk, no options-parameter divergence, liveness is pure key membership), RB11 (tilde at match time, dir-self match, `[!]` vs `[^]` fnmatch parity, backslash literal), RB5/A6(a) (fourth table incl. nested form, dup-facet incl. cross-case, wrong-typed version), A6(b) diff-aware set (fix-one-introduce-another denies; identical-broken allows; unparseable baseline allows), A5 taxonomy (0/2 only, driven; short-circuit; no empty envelope; suppressOutput top-level), RB1(a) engine contract, D6 boundary (exactly 6 deny sites; partial edits incl. smuggled `content` allow; `is_full_write` Write-only at its sole site), kill-switch ordering, store-resolution fail-open matrix (RC-a), A7 session-start ordering, curation floors (unwindowed span leg live, seat dual-gate + zero-fire floor, WR-01/02 locks, corpse reclaim, byte-identical bodies), R7 window bound + rotation-straddle safety, RB10 (kernel excluded from the gate key, loud degrade), N14 arithmetic at `2654da5` (independently re-derived), RC-c (citation reads `trigger_type`), RC-d (all four cross-packet directives complied).

## Risky bits with no test coverage

- **RB1(b)** — live-host deny observation: not testable in-repo by design (human-only). OPEN, owner action above.
- **A4-at-real-scale** (F18): the calibration protocol has run only against toy/synthetic stores; its behavior against the real deployed store is unexercised until deployment (R1).
- Everything else on the risky list now carries a lock test — including the eight added by this gate (out-of-scope allow, anchor-only globs, partial-mark crediting, cross-env baseline, qid discrimination, NaN baseline, const-override warnings, dead evidence, solidus escape, bad tag name).

## Negative-contract sweep

Executable sweep `cargo test --test negative_contract`: **27/27 green at HEAD** (re-run after every fix). Per-N verdict over the FULL diff, sweep-sheet checks plus this gate's independent probes:

| N | Verdict | Note |
|---|---|---|
| N1 | **clean** | one `fn walk(`; recall+projection same hit through it; no options-parameter second matcher (probed) |
| N2 | **clean** | zero sqlite/fts5/embedding hits; Cargo.lock tree inspected independently |
| N3 | **clean** | `prompt` key byte-ignored by the normalizer (behavioral) |
| N4 | **clean** | zero-fire byte-identical; demote via frontmatter only; files never removed |
| N5 | **clean** | no derivation code; `Source` closed at compile time |
| N6 | **clean** | floor scale-invariant; single floor constant (its config shadow now WARNS when overridden — F9) |
| N7 | **clean** | no policy keys in hooks JSON; hooks module zero-I/O; bootstrap writes only caller paths (also re-driven live) |
| N8 | **clean, tightened** | fourth table / dup-facet / tagless all deny; F16 adds tag-NAME TAG_RE conformance |
| N9 | **clean** | no import flag (clap-rejected, driven); one legacy mention, a comment |
| N10 | **clean** | deps exactly the known set; no process spawn; PyYAML oracle out-of-process, vector-corpus fallback confirmed real |
| N11 | **clean** | no rebuild/body-read/output-on-silence (behavioral + no-fs-call grep; re-driven live) |
| N12 | **clean** | no `synapse` path/`.py` in tree (synapse consulted read-only by path as the ledger's tiebreaker — the sanctioned mode) |
| N13 | **clean, tightened** | 0/2 only, every fault branch allows; F15 closes the exit-101 panic escape (non-blocking but non-silent) |
| N14 | **clean (was fixed in-build)** | ceiling = baseline + calibrated slack only; REGRESSED gated on slack>0; F5/F7 close the two remaining silent-degrade laundering paths |

## Fix loop disposition

Every fix commit is one finding, gated green (fmt + clippy `-D warnings` + full suite) before commit. Suite: 270 → **278 tests, 0 failures**.

| Finding | Disposition | Ref / reason |
|---|---|---|
| F1 RB1(b) record missing | **DEFERRED — OWNER ACTION REQUIRED** | Human-only by definition; procedure restated above and in the build report. /ship should refuse until the record is filled — this is the one deferral with owner-regret potential. |
| F2 guard scoping (BLOCKER) | FIXED | `20cc8be` |
| F3 anchor-only broad globs | FIXED | `a4fdd21` |
| F4 partial-mark fire logging | FIXED | `ab35b3f` |
| F5 cross-env baseline carry | FIXED | `c21bb2a` |
| F6 qid synonyms | FIXED | `fabdff3` |
| F7 NaN/inf baseline | FIXED | `bf75a02` |
| F8 lossy rate read | FIXED | `45844bd` |
| F9 const-consumed overrides | FIXED | `dbb11bc` |
| F10 dead-evidence findings | FIXED | `605a9ac` |
| F11 second tier map | FIXED | `894609d` |
| F12 config in generation id | FIXED | `0e2e46d` |
| F13 stale docs | FIXED | `8879f18` |
| F14 solidus escape | FIXED | `7955861` |
| F15 hook panic net | FIXED | `8eebdb5` |
| F16 grammar tag names | FIXED | `302a53e` |
| F17 scope-seam doc | FIXED | `7e32730` |
| F24 misplacement `other` targets | FIXED | `a731133` |
| F18 calibration jitter mechanics | DEFERRED | Formula faithful to A4(c); observed only at 1-memory/10 µs scale (500× under D16 reference); real-scale behavior unknown until deployment calibration. If false-REGRESSED reproduces there → G4 amendment to A4's run mechanics. |
| F19 dual-bucket ≥2 corner | DEFERRED | Faithful to frozen "≥2 tuples total" wording + synapse dual-lookup parity; only author-self-inflicted (same value declared on two axes); changing tuple counting risks false-silence. |
| F20 symlink pre/post-op seam | DEFERRED | §5.x: the two canonicalizations differ on purpose; fail-open, self-heals at next rebuild. |
| F21 parser laxer on 2 scalars | DEFERRED | Fail-open direction, outside the frozen B2 rejection surface; recorded as future differential-corpus vectors. |
| F22 BOM handling | DEFERRED | Fails open; surfaced by validate as malformed; changing it edits the frozen B2 boundary → future amendment material, not a /vet patch. |
| F23 non-tmpfs mark hygiene | DEFERRED | By design (D25); mtime TTL keeps correctness; disk-hygiene only. |
| R1 calibration commit | **DEFERRED to deployment** | D-ledger R1 says /vet verifies the calibration commit; none exists and none CAN faithfully exist yet: A4(b) pins the regression baseline to the REAL store, which is empty/nonexistent until the engine is deployed (D17 empty bootstrap). A toy-store calibration was run as protocol verification and produced a degenerate slack + false REGRESSED (F18) — committing such numbers would be exactly the laundering A4 forbids. NOBASELINE-until-calibrated is the contract's own designed interim (D26). Owner step at deployment: bootstrap the real store → author initial memories → `rejolt bench --store <real> --calibrate` → commit baseline + report as the R1 reviewable commit. |
| RB7 legacy corpus | CLOSED (accepted risk) | Per `docs/reports/legacy-fence-rb7-routed-memory-reseed.md`; not re-opened, fence suite green at HEAD. |
| §14 row 23 `byMemoryId` wording | DEFERRED to /ship | R4 compaction folds or banners CORE-SPEC; ledger precedence already resolves the conflict. |

## Runtime verification

Full end-to-end drive of the release binary against a temp store + sandboxed XDG config (pre-fix and re-driven post-fix):

- **bootstrap**: loud creation report, idempotent rerun, never-overwrite, `--print-hooks` JSON carries zero permission-policy keys, fail-open verification rows all pass.
- **rebuild/index**: 13-column TSV, grammar-tag rows pre-flattened per member (`t`) beside per-memory rows (`m`), deterministic generation.
- **recall**: matching Bash event fires with the frozen citation form (`gpu-tools <- command:nvidia-smi`), high confidence; non-matching and `hook_event_name`-less payloads are silent exit 0 (A5(b) observed working).
- **hook pre-op**: advisory envelope with TOP-LEVEL `suppressOutput` (Appendix C); in-store invalid write → stderr + exit 2, short-circuit; **out-of-store Jekyll write → exit 2 pre-fix (the F2 demonstration) → exit 0 silent post-fix**; kill-switch suppresses the same deny to exit 0; malformed payload exit 0 silent.
- **direct CLIs**: `check-write` deny loud exit 1; `project` JSON verdict PASS; `maintain` below-trigger exit 0; `seats` insufficient-evidence exit 0; `search --expect` absent → exit 1 (seat-probe form); `validate` names the dead `--no-cache` evidence post-F10 (exit 1).
- **bench**: NOBASELINE loud + exit 0; `--calibrate` runs the full A4 protocol (synthetic-1000 budget ×3.0, env fingerprint with kernel-as-metadata, R7 advisory) — and exposed F18/informed the R1 deferral; REGRESSED exits 1.

Final state at HEAD: `cargo fmt --check` clean, `clippy --all-targets -- -D warnings` clean, **278/278 tests green**, all four post-fix smoke probes per contract.

**Next command: `/ship`** — blocked on the F1/RB1(b) owner record per this report's disposition.
