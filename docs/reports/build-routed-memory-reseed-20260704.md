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
| WP-1 | P2, P3 | `c07dcd6` | 21 fns / 39 fixtures | Parser + grammar loader; 6 verify defects fixed & locked before commit |
| WP-2 | P4, P5 | `2bf5a74` | 51 (+30) | Flat index + one walk + rebuild + drift; 5 verify defects fixed (byPath `**`, line-safety) |
| WP-2b | P11 | `2e0d8fb` | 70 (+19) | Marks + telemetry; 5 verify defects fixed (atomic append, injective marks, correlation gating) |
| WP-3 | P7, P6 | `see log` | 100 (+30) | Host-event parser + recall; 3 verify defects fixed (generic-verb shadow, web-keyword tier, is_full_write gate) |
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

- **WP-2** (4 Opus lenses vs the synapse ground truth + Python fnmatch, read-only, uncommitted tree): 0 blockers, 3 majors, 2 minors, 2 nits — all empirically confirmed, all fixed & locked before commit:
  1. byPath `**` false-fire (MAJOR, D5 precision): the matcher fired mid/bare `**` (`**`, `**/*.md`, `~/**/settings.json`) on every path. The frozen ground truth (Appendix A → `memory_surface.py:1765-1771`, "`**` sanctioned ONLY as trailing `/**`") skips them as broad (§3.x). Added the missing skip branch; corrected the two tests that wrongly asserted `**/*.md` fires.
  2. control chars break one-record-per-line (MAJOR×2 + minor, A2e/RB2): the exclusion guarded only the routing `pattern`, so a `\t`/`\n` in `lastReviewed` or in a memory's FILENAME (→ memory_id/route_tag/path columns) split the line → whole index `Malformed` → recall silently index-free store-wide. Generalized: filename-hostile memories excluded+reported; `lastReviewed` sanitized; `emit` `debug_assert` no-control-char tripwire.
  3. fnmatch `[^...]` parity (minor): `^` is a literal class member in Python fnmatch (only `!` negates) — fixed.
  4. generation_id NUL-framing not injective (nit): length-prefixed hash fields.
  5. no fsync in write_atomic (nit): fsync temp + parent dir (D14 durable across power loss).
  Confirmed sound & unchanged: normalization symmetry (the A2 fix), RB9 one-walk, torn-pair detection, single-reader fail-open, tier map, D10 partition, N1/N2/N5/N10/N11/N12.
- **WP-2b** (4 Opus lenses vs D25/A7 correlation invariant + §8, read-only, uncommitted tree): 0 blockers, 1 major, 3 minors, 2 nits — all fixed & locked before commit:
  1. non-atomic JSONL append (MAJOR): record + `\n` were two `write_all`s under O_APPEND → concurrent hook processes (Claude Code batches parallel tool calls) interleave to `{RA}{RB}` → both silently dropped by the reader (telemetry loss beyond the accepted A7 bias). Fixed: single `write_all(line+"\n")`; reader now byte-reads + `from_utf8_lossy` per line so corruption is bounded to one line.
  2. non-injective mark filename (minor): `gpu notes`/`gpu_notes` collided → cross-memory read mis-credit (a per-memory correlation-invariant breach). Fixed: injective `%XX` percent-encoding.
  3. `log_fire` gated on a separate `fired_ids` arg, not `record.mems` (minor): a mems⊄fired_ids caller could log a write-time fired-but-unread. Fixed: **new signature `log_fire(record)`** derives the gated set from `record.mems`; empty mems → ZeroFire.
  4. no-follow TOCTOU + write/advisory disagreement (minor): fixed with `O_NOFOLLOW` opens + `runtime_dir_safe` (euid-owned, not group/world-writable) gating BOTH the write path and the inert advisory.
  5. session markers discarded (nit): surfaced `WindowedTelemetry.sessions` for WP-6 floor-2.
  Dep added: `libc 0.2` (arch-correct O_NOFOLLOW/geteuid; N10-fine). Confirmed sound: TTL single-source, WR-04 `.1`-first, WR-05 bad-ts symmetry, R7 window bound, record shapes, N2/N10/N12. **Directive to WP-3:** call `log_fire(&FireRecord)` (no separate fired_ids). **Directive to WP-6:** consume `WindowedTelemetry.sessions`.
- **WP-3** (4 Opus lenses vs D1/D3/D5/D19/A5/N3/N11 + the synapse tiebreaker, read-only): 0 blockers, 3 majors, 1 minor — all fixed & locked before commit:
  1. GENERIC_VERBS applied POST-dedup on the first-seen tuple representative (MAJOR, found by 2 lenses): a generic command shadowed a co-present specific command under the same route_tag → order-dependent silent MISS (`install && rsync` vs `rsync && install` disagreed). Fixed: hit-level generic-verb filter BEFORE the tuple dedup (a command tuple survives iff ≥1 non-generic command matched) — matches synapse's pre-walk filtering.
  2. web keywords over-tiered into byArg/medium (MAJOR): the tiebreaker routes web/tag-kind tokens through bySynonym/weak only (`kind=="tag"` never consults byArg) — the reseed let one `WebSearch{query}` clear the ≥2 gate synapse keeps silent. Fixed: `web_content` → synonyms bucket only; Bash args keep the args+synonyms dual-lookup.
  3. `is_full_write` un-gated by tool (minor): a crafted Edit/MultiEdit carrying a `content` key flipped to a guardable full write → a fail-open→fail-closed (N13/D6) inversion risk for WP-4. Fixed: `is_full_write = tool_name=="Write" && content non-empty`.
  Confirmed sound: index-only/no-rebuild/no-body-load (N11/D1), silence emits nothing (D19), Bash tokenizer + §5.x lexical canonicalization (checked vs real `realpath -sm`), A5b fail-open (no panic on hostile payloads), tier map/penalties/§10 tunables. WebSearch/WebFetch/context7 routing on tool *inputs* is D3-grounded (synapse tiebreaker). **Directive to WP-4:** `is_full_write`/`proposed_content` are Write-only now — still independently confirm tool identity before denying.

## Spec-friction reports from builders (G5)

- **WP-2 friction #1 — the `type` column (RESOLVED, no amendment).** Appendix A's 13-column schema carries both `trigger_type` and `type`; the reseed frontmatter (D21) has no `type` key. Synapse tiebreaker (`memory_surface.py`): `trigger_type` = the axis (command/path/arg/synonym), used for tier + the citation `{tag} ← {trigger_type}:{matched_value}` (:2091) — POPULATED; `type` = `meta.get("type","")` (a memory-classification field the reseed dropped along with synapse's `_type_boost`). So `type` is a faithful empty reserved column and the P6 citation must read `trigger_type`. **Directive to WP-3:** the recall citation uses the populated `trigger_type`/axis, NOT the empty `type` column.
- **WP-2 friction #2 — §11 BLOCK-degenerate assertion (deferred to WP-4, fail-open).** The drift guardrail's assertion 2 (and the broad-path arm of assertion 1) need WP-4's `is_broad_path` + collision projection; they ship as documented fail-open stubs (`would_block_degenerate`, `static_gate_would_deny`) — no false advisory, no block. **Directive to WP-4:** tighten these named predicates once `is_broad_path`/BLOCK-degenerate exist.

_No WP-0/WP-1/WP-2 builder note rose to a G4 spec contradiction; all were interpretation/under-spec points resolved via the ground-truth tiebreaker._
