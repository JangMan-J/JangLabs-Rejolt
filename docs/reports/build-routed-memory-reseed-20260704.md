# routed-memory-reseed — build report (S3 /build, T2)

**Stage:** S3 `/build` · **Tier:** T2 · **Started:** 2026-07-04
**Plan:** `docs/frozen/routed-memory-reseed-plan-20260704.md` (FROZEN v1)
**Spec:** `docs/frozen/routed-memory-reseed-decisions-20260703.md` + `…-amendments.md` (A1–A7)
**Freeze-commit (DIFF anchor for /vet):** `69184e7185d9a9c10e371fd3f80ec9bb1e1ddf46`
**Build session model:** Opus 4.8 (`claude-opus-4-8[1m]`) — S3 session floor (§8) met.
**Packet builders:** Opus (§8 / build SKILL.md: opus default; no packet is labelled *mechanical*).
See "Model reconciliation" below.

## Execution model

Sequential packet builds in dependency order on `wf/routed-memory-reseed`. Each
packet is implemented by one Opus builder subagent working in the main tree; the
integrator (this Opus session) independently re-runs the packet gate
(`cargo fmt --check` · `cargo clippy --all-targets -- -D warnings` · `cargo test`
· repo `verify.sh` where present) and is the **sole committer** (G5). Gate
green → commit immediately (G7 loss-net); push per stage boundary / packet batch.
The G7 worktree-commit rule is satisfied here by immediate post-gate commits in
the main tree plus the PreCompact/SessionEnd snapshot hooks; parallel worktree
isolation was not used because the packet DAG is near-linear over a single crate
and `worktree.baseRef` defaults to `fresh` (would not carry prior packets).

Dependency-respecting build order: **WP-0 → WP-1 → WP-2 → WP-2b → WP-3 → WP-4 →
WP-7 → WP-5 → WP-6 → WP-8.**

## Model reconciliation (recorded, not friction)

The frozen plan's Budget line reads "packet builders sonnet; integrator opus"
(pre-ADR-0007 economics). WORKFLOW.md §8 (v1.6, ADR 0007) and the build SKILL.md
supersede it: WP packet builders are **opus by default, sonnet only where the
plan labels a packet mechanical**. No packet carries a *mechanical* label, so all
builders run on Opus. Governed by §8's "resolve model from this table — never
hardcode a different tier"; this is a tier resolution, not a G4 spec amendment.

## RB1(b) — LIVE-host deny probe (HUMAN-ONLY, discharged at build START)

Per plan Budget + Risk register RB1(b): the sole deferral with owner-regret
potential — every fail-closed guarantee rests on the live host actually blocking
on the proven mechanism. **This needs no `rejolt` code** and must be done by the
owner at build start, not build end.

Procedure:
1. In a **fresh** Claude Code session, register a minimal PreToolUse hook that
   emits a line to **stderr** and exits **2** on a matcher (e.g. `Bash`).
2. Make **one deliberate matching tool call** (e.g. a trivial `Bash` command).
3. Observe that the host **blocks** the call (exit 2 + stderr is the proven
   deny mechanism — Appendix C / A5(a); exit 1 does *not* block).

Record the outcome here; `/vet` verifies this record exists (doc evidence alone
never closes it — the observation is the evidence):

> **RB1(b) result:** _[PENDING owner observation — date, matcher used, blocked? y/n]_

## WP → commit map

| WP | P-items | Commit | Tests (cum.) | Notes |
|----|---------|--------|--------------|-------|
| WP-0 | P1 | `f9616df` | 5 | Skeleton + G2 harness proven (rubber-stamp self-test) |
| WP-1 | P2, P3 | `see log` | 21 fns / 39 fixtures | Parser + grammar loader; 6 verify defects fixed & locked before commit |
| WP-2 | P4, P5 | _pending_ | | |
| WP-2b | P11 | _pending_ | | |
| WP-3 | P7, P6 | _pending_ | | |
| WP-4 | P9, P10 | _pending_ | | |
| WP-7 | P13, P14, P15 | _pending_ | | |
| WP-5 | P8 | _pending_ | | |
| WP-6 | P12 | _pending_ | | |
| WP-8 | P16, P17, P18 | _pending_ | | |

## Amendments raised during build (G4)

_none yet_

## Rule-of-two / Fable consults (§8)

_none yet_

## Adversarial verify passes (ultracode; before each packet commit)

- **WP-1** (4 Opus lenses, read-only, over the uncommitted tree; all findings empirically confirmed vs PyYAML 6.0.3): 0 blockers, 3 majors (2 were the same false-deny, independently found by 2 lenses), 7 minors, 1 nit. Theme: the hand-rolled parser diverged from the named PyYAML `safe_load` oracle (A3/B2) on cases the goods-only differential never exercised. Fixed by the builder before commit:
  1. mid-scalar `{`/`}` in a plain scalar → wrongly `FlowMapping` (false-deny, RB3): dropped the blanket contains-guard; a true flow map starts with `{` and is still caught.
  2. inline ` #` comments absorbed into trigger values → reject space-then-`#` as out-of-subset (`c#` with no leading space still a literal).
  3. `: ` in a plain scalar false-accepted (PyYAML errors) → reject unquoted `": "`.
  4. unknown `\x`/`\u` double-quote escapes silently mangled → reject unknown escapes (keep `\" \\ \n \t \r`).
  5. grammar `commands=[""]` false-accepts the D3 evidence guard → count only non-blank evidence; reject blank/control-char evidence + newlines in gloss (digest-injection).
  6. nit: `DuplicateFacet.facets` sorted to match its doc.
  Each fix locked into the differential/vector oracle by new good (braced) + bad (inline-comment / colon / bad-escape) fixtures. Confirmed sound and unchanged: metadata-key strictness, A6a fourth-table/dup-facet, version/placement enums, N10/N12, one hand-rolled parser, the differential comparison method.

## Spec-friction reports from builders (G5)

_none yet — WP-0, WP-1 raised no G4 spec contradictions (WP-1's builder notes were interpretation/under-spec points, not contradictions)_
