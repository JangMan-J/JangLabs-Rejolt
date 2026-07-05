# routed-memory-reseed — amendments (A-ledger)

**Append-only.** Opened 2026-07-04 against `docs/frozen/routed-memory-reseed-decisions-20260703.md`.
An amendment is the ONLY legal response to build-time evidence contradicting a
D-entry — faithful compliance with a wrong spec and silent drift are both
illegal (WORKFLOW.md G4). Same provenance discipline as D-entries.

<!-- Rules:
  - Heading form `## A<n>. <imperative title>` is the trace anchor.
  - Each A names: the D it amends, the EVIDENCE (measured, not vibes), the
    decision, affected P-items (which must be re-gated), and who decided.
  - An A that reverses an owner-attributed D requires explicit owner
    confirmation, recorded here.
  - After appending: update affected P-items' refs, re-run trace-lint.
-->

## A1. Record the defective grill delivery behind Part B; D15–D26 stand

- Amends: D15, D16, D17, D18, D19, D20, D21, D22, D23, D24, D25, D26 —
  provenance only; no content change.
- Evidence:
  - Owner report (2026-07-04, this amendment's trigger): the /distill skill
    that ran the 2026-07-03 grill "was essentially broken and needed to be
    rebuilt."
  - Defect, as diagnosed in the fix commit (jskills `9dc82a7`): interview
    substance — framing, evidence, trade-offs, interviewer recommendations —
    was routed into AskUserQuestion choice dialogs; the client does not
    reliably render text written before a mid-turn tool call, so the grill
    degraded to a quiz. Part B's owner answers were given through that
    degraded channel.
  - Rebuild, in-session, before freeze: grilling engine
    `~/.agents/skills/grilling/SKILL.md` rewritten 2026-07-03 05:41:44 -0700
    (every question is end-of-turn conversational output answered in free
    text; a dialog may only close out a matter already fully delivered —
    inherited by grill-us*); distill wrapper fixed in jskills `9dc82a7`
    (05:45:00 -0700) applying the same delivery contract to the tier step.
  - Freeze ordering: the ledger froze at rejolt `39ad170`
    (2026-07-03 05:48:50 -0700), minutes after both fixes — the defect was
    known at freeze time and the owner froze regardless.
  - Workflow version (noted per owner instruction): the ledger froze under
    jskills `workflow/WORKFLOW.md` v1.4 (`df633aa`); WORKFLOW.md is now
    FROZEN v1.5 (`ce673d3`, 2026-07-03 06:09 -0700). v1.5's T2 gate matrix
    added a pre-freeze ledger red-team at /distill; it therefore never ran on
    this ledger.
- Decision: provenance annotation — **D15–D26 remain in force unchanged.**
  The defect degraded the interview channel, not the recorded decisions:
  D16/D24 rest on in-session measured benchmarks independent of the channel;
  D15, D17, D20, D22, D26 carry recorded owner rationale; the
  interviewer-recommendation overrides in D15/D22/D23 are owner acts that
  survive the channel defect. Any content-level challenge to an individual D
  takes its own A-entry with content-level evidence. Follow-up recorded for
  the missed v1.5 gate: run the S1 bias-lens ledger red-team (registry §5)
  over Part B at `/blueprint` entry, before the plan consumes it.
- Affected plan items: none — no plan artifact exists (S2 /blueprint not yet
  run); nothing to re-gate.
- Decided by: owner (amendment initiated by owner 2026-07-04); recorded by
  Claude Fable 5. No owner-attributed D is reversed.

<!-- A2–A7 below fold the A1-mandated S1 bias-lens red-team (6 lenses, 22
     findings, run 2026-07-04 at /blueprint entry) plus ground-truth extraction
     over the synapse engine. Full panel output preserved in the /blueprint
     session record; adjudication: accepted findings land here or as plan
     constraints, rejections carry one-line reasons in the plan. -->

## A2. Make the flat index the sole routing structure; bind the artifact pair

- Amends: D24 (and the D4/N1 reading; extends D14's span).
- Evidence: red-team blocker (scope lens): as written, recall walks the flat
  index while projection/liveness walk the JSON catalog's tables — two
  independently rebuilt serializations of the routing facts, i.e. the second
  matcher N1 forbids; a torn rebuild diverges them invisibly (failure lens —
  D14 is per-file only, and §4's fingerprint rides the report, not the index).
  Ground truth: the old engine's `byMemoryId` table is never read by
  `_walk_index` (`memory_surface.py:2119,2152–2155`), and D18 removed its only
  producer; per-hit field consumption inventoried with line cites (extraction
  report, preserved in the plan appendix).
- Decision: (a) the flat index is the ONLY routing structure any matcher
  walks — recall, collision projection, and byArg/bySynonym liveness all read
  it through one walk module; (b) the JSON catalog report carries NO routing
  tables — write-side metadata only (memories with tags+description for dedup,
  routabilityReport, sourceFingerprint, rendered vocab digest); (c) the index
  has FOUR tables (byCommand/byPath/byArg/bySynonym) — byMemoryId is dropped;
  tag→member expansion is pre-flattened at build (one row per
  (table, pattern, memory_id)); (d) both artifacts carry an identical rebuild
  generation id + sourceFingerprint, written index-first/report-last; a
  generation mismatch = stale pair, detected fail-open with one advisory —
  D14's crash guarantee now spans the pair; (e) one record = one physical
  line, NO escaping layer: routing-critical fields containing tab/newline are
  excluded at build and listed in routabilityReport (never a hook failure);
  display fields (snippet) are sanitized/truncated at build.
- Affected plan items: P4, P5, P10, P16 — gated at draft (this session).
- Decided by: agent (S1 red-team fold, blueprint session 2026-07-04); owner
  ratification at plan freeze. Preserves D24's two-artifact shape and D4's
  owner tenet; no owner call reversed.

## A3. Bind the frontmatter parser to an oracle; record the hand-roll rationale

- Amends: D21 (and D24's "only hand-rolled parser" wording).
- Evidence: three lenses converged (sourcing major, rigor major, scope minor):
  D21 mandates the system's one bespoke parser at the sole fail-closed
  boundary with no recorded sourcing rationale, while D23 records
  minimize-bespoke-parsing as a value; "property-tested" names no oracle; and
  D24's flat-index reader is a second de-facto bespoke READ surface D21's
  wording denies, so its test obligations would be lost.
- Decision: the hand-roll STANDS (rationale now recorded: full-YAML semantics
  at a fail-closed boundary are a liability, and the Rust serde-YAML crate
  landscape is poor — serde_yaml archived 2024, successor forks unproven;
  re-verify the crate landscape at /build before implementation as a cheap
  exit). Bound by: (a) the dialect is frozen as a formal grammar artifact
  in-repo (WP-1 deliverable) covering §3's flow and block forms; (b) oracle:
  differential testing against a reference YAML parser restricted to the
  subset, plus generate→parse→regenerate round-trip, plus the §3 examples as
  a fixture corpus; (c) deny diagnostics cite the violated dialect rule;
  (d) D21's wording is amended: the flat-index reader is the second bespoke
  read surface — kept trivial by A2(e)'s no-escaping rule and carrying its own
  property tests.
- Affected plan items: P2, P5 — gated at draft.
- Decided by: agent (S1 red-team fold, 2026-07-04); owner ratification at
  freeze. D21's owner-adjacent posture (bespoke subset) preserved.

## A4. Pin the calibration derivation and environment predicate; never degrade silently

- Amends: D26 (and the D9 interplay).
- Evidence: rigor lens: safety factor, jitter→slack function, and
  reference-corpus roles are free variables — R1/N14 are unverifiable and an
  arbitrary number can be laundered through one reviewable commit; under D17
  the real store is empty at first calibration, so a real-store-derived budget
  recreates the permanent-red drift Appendix A retired. Environment "matching"
  is undefined; this box runs a rolling kernel (`uname -r` =
  7.2.0-rc1-4-cachyos-rc), so exact-match disables WARN/REGRESSED after every
  routine update — silently (scope lens major).
- Decision: the derivation STRUCTURE is spec, frozen here; numbers stay
  outputs (owner rationale intact). (a) Design budget = synthetic-1000 p95 ×
  safety factor 3.0 (frozen constant); (b) regression baseline tracks the
  real store; (c) ceiling slack floor = max(3 × cross-run σ of p95, observed
  min→max p95 band) over ≥5 runs of ≥100 samples; the calibration commit
  shows the arithmetic; (d) environment gate key = CPU model + governor +
  power source; kernel is recorded metadata, never a gate key; (e) on
  fingerprint mismatch or missing baseline the gate is measure-only AND LOUD
  (one line naming the degradation and the recalibration step) — silent
  degradation is a conformance failure.
- Affected plan items: P13 — gated at draft.
- Decided by: agent (S1 red-team fold, 2026-07-04); owner ratification at
  freeze — D26 is owner-attributed; this pins mechanics in service of its
  stated rationale, reverses nothing.

## A5. Hook modes use the host's real block mechanism and fail open on parse; the write-capable tool set is closed

- Amends: D19, D20 (hook-mode mechanics; the D20 name, shape, and direct-CLI
  taxonomy are untouched).
- Evidence: agency major, confirmed by ground truth: Claude Code PreToolUse
  blocks on exit 2 + stderr — exit 1 does NOT block; the old guard used "the
  on-box-proven exit-2 + stderr form" (`memory-write-guard.sh:21-22`). D20's
  frozen taxonomy maps a gate deny to exit 1, which would leave D6's boundary
  inert on the sole v1 profile. Failure-posture major: D19 states no posture
  for malformed/schema-evolved payloads; a strict parse coded as
  "exit 2 = config error" would BLOCK ordinary tool calls (N13 violation).
  Rigor major: no unknown-tool posture — the fail-closed boundary is silently
  best-effort over a deferred tool table.
- Decision: (a) the 0/1/2 taxonomy governs DIRECT CLI modes only; hook modes
  obey host semantics: allow = exit 0 quiet; deny = the host's proven block
  mechanism (v1: exit 2 + stderr), issued ONLY from the write-guard branch;
  recall/post-op/session-start branches never exit nonzero. (b) Host-payload
  deserialization failure fails OPEN on every hook path — unknown fields
  ignored, missing optionals tolerated, unparseable event = unclassifiable =
  allow, silently. (c) The write-capable tool set is CLOSED and enumerated
  (v1: Write = full content; Edit/MultiEdit = partial edit, fail open, no
  reconstruction — parity with the proven old behavior and D6's partial-edit
  contract); tools outside the enumeration are non-guardable and fail open as
  an accepted, recorded limitation — D6 now reads "the single fail-closed
  boundary over the enumerated write-capable tool set." (d) Conformance: a
  hook-mode deny fixture must actually block under the live v1 host; a
  malformed-payload fixture must pass silently (both G2-style).
- Affected plan items: P7, P8, P9, P15, P16 — gated at draft.
- Decided by: agent (S1 red-team fold, 2026-07-04); owner ratification at
  freeze — D20 is owner-named; name/shape/exit-taxonomy-for-direct-CLIs stand.

## A6. Enforce the closed facet set for real; grammar writes join the guard, diff-aware

- Amends: D23 (enforcement mechanics), D6/D22 (boundary enumeration —
  **owner confirmation required**: this widens the fail-closed boundary).
- Evidence: rigor BLOCKER: serde's default Deserialize silently ignores
  unknown keys — a fourth facet table would be dropped, not exit-2'd, and a
  tag under two facet tables is legal TOML; D23's "structurally enforced" is
  false as written, so D22's governor and N8 never fire. Agency major: D22
  says "denied at write time" but grammar.toml is not a frontmatter-bearing
  memory, so under D6 an interactive grammar edit sails through and the
  anti-junkyard wall never gates where junk actually enters. Ground truth:
  the old system DID fail-close grammar/taxonomy full writes at PreToolUse,
  diff-aware, bootstrap-allowed (`memory-write-guard.sh:112-160`) — the live
  engine is the ledger's own tiebreaker on conflict.
- Decision: (a) `#[serde(deny_unknown_fields)]` is mandated on the grammar
  root and every entry struct, plus an engine-side cross-table check that
  each tag appears under exactly one facet; conformance: a fourth table AND a
  duplicate-facet tag both exit 2. (b) The write-guard boundary gains its
  second (and last) enumerated surface: a FULL-FILE write of the grammar file
  is denied when it introduces genuinely NEW validation errors relative to
  the current file (diff-aware; file absent = bootstrap = allow). Partial
  grammar edits fail open; the post-op refresh stays fail-open (stale catalog
  retained + one loud stderr correction line — the old system's proven
  shape). This implements D22's owner-stated governor at the surface where
  junk enters, matching live-engine behavior.
- Affected plan items: P3, P9, P16 — gated at draft.
- Decided by: agent (S1 red-team fold, 2026-07-04); **(b) requires explicit
  owner confirmation at freeze** (boundary enumeration change); (a) makes
  D22/D23's stated intent true and reverses nothing.

## A7. State the correlation invariant's true guarantee; record the reboot-straddle bias

- Amends: D25 (contract wording only).
- Evidence: coverage major: marks wiped AFTER fire-logging (reboot/suspend
  inside the TTL window) orphan already-logged fires — a fired-but-unread
  record exists in exactly that case, so "can never be recorded
  fired-but-unread" overclaims; on this laptop suspend/reboot is routine
  (D26's own rationale cites governor/thermal churn).
- Decision: wording corrected to the true guarantee — fires are logged only
  when marks persisted AT WRITE TIME; a later mark wipe can orphan a logged
  fire. The resulting read-rate deflation is a stated, accepted bias, bounded
  by the zero-fire floor, the min-evidence guard, and the low demote
  threshold (D7), with declineCount clearable and non-destructive. No
  mechanical fix (persisting mark metadata reintroduces the join D25
  rejected). Bootstrap and maintain verify the runtime mark dir is writable
  and emit one advisory when telemetry is structurally inert (curation
  otherwise dies silently — failure lens minor, folded here). Calibration may
  quantify reboot-straddle frequency if curation behaves oddly (risk
  register).
- Affected plan items: P11, P13 — gated at draft.
- Decided by: agent (S1 red-team fold, 2026-07-04); owner ratification at
  freeze; no owner call reversed.

## A8. Admit the host memory tool's bookkeeping keys into the dialect

- Amends: D21 / plan Appendix B2 (the frontmatter dialect's accept surface —
  metadata key set only; tags-required, top-level-`triggers:` rejection, and
  every other boundary rule untouched).
- Evidence (measured at deployment, 2026-07-04, live box store
  `~/.claude/projects/-home-jangmanj/memory`):
  - Claude Code's own memory tool — not the Write tool — is how memories
    "occur spontaneously" on this host (owner-stated usage model). It
    rewrites frontmatter on save: injects `metadata.node_type: memory`,
    `metadata.type: <class>`, `metadata.originSessionId: <uuid>`, reflows to
    block style with trailing spaces after mapping keys. Verified identical
    across three independent files: a fresh memory-tool save
    (`rejolt-memory-engine-live.md`), a weeks-old box memory
    (`multipass-fork-build.md`), and a project-store memory
    (`grill-questions-conversational-not-dialogs.md`).
  - Under the frozen dialect these keys reject: `rejolt check-write` denies
    with "line 5: unknown metadata key `node_type` (allowed: tags, triggers,
    lastReviewed, declineCount)"; rebuild classifies every such memory
    malformed → zero routes → recall silent. Demonstrated end-to-end at
    deployment: a dialect-valid memory with tags+triggers was saved through
    the memory tool, landed with injected keys, and never routed.
  - Consequence as frozen: NO spontaneously-written memory on this host can
    ever route — the engine's core purpose fails at deployment.
- Decision: the dialect ACCEPTS an enumerated host-metadata allowlist —
  `node_type`, `type`, `originSessionId` — as tolerated bookkeeping keys:
  parsed, carried, and re-emitted verbatim by `generate` (curation's
  frontmatter round-trip must not strip them), never routing inputs, never
  evidence. The closed-world posture is preserved: any key OUTSIDE
  {tags, triggers, ranking fields, this allowlist} still rejects, and
  growing the allowlist takes another A-entry. `metadata.type` additionally
  populates the flat index's reserved `type` column (Appendix A already
  carries that column for exactly this memory-classification field;
  recall ranking still ignores it — no scoring change). The memory tool
  itself remains outside the A5(c) write-capable tool set (its writes are
  unguarded — recorded host reality, not a boundary change; the guard still
  gates Write/Edit/MultiEdit and the routability report + curation are the
  net for tool-written memories).
- Rejected: (a) a box store outside the harness memory dirs — spontaneous
  memories would never reach it (defeats the owner's stated usage model);
  (b) tolerating ALL unknown metadata keys — reopens the anti-junkyard wall
  D21/D22 exist to hold.
- Affected plan items: P2 (dialect accept surface), P5 (Appendix A `type`
  column now populated) — refs updated, trace-lint re-run.
- Decided by: agent (deployment session, 2026-07-04), narrowing the deny
  surface (false-deny prevention, the #1-rule direction — same class as the
  /vet fixes); no owner-attributed call reversed. Surfaced to the owner in
  the deployment report.

---

**Ratification record — plan freeze, 2026-07-04.** Owner ratified A2–A7
wholesale ("ratify all"); A6(b)'s boundary widening explicitly confirmed
("yes"); R5 resolved empty-seed ("empty seed"), recorded as the P14 OWNER
ref. Second-model review: Codex via multipass, 2 turns — 3/3 owner calls
AGREE, material gaps none; adversarial probe folded (RB1(b) live deny check
hoisted to build start).
