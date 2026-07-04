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
