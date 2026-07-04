# routed-memory-reseed — plan

**Status: FROZEN v1 — 2026-07-04.** Owner ratification 2026-07-04: A2–A7 ratified wholesale; A6(b) boundary widening explicitly confirmed; R5 = empty seed (P14 OWNER ref).
**Tier: T2 (inherited from spec: `docs/frozen/routed-memory-reseed-decisions-20260703.md`)**
**Spec:** `docs/frozen/routed-memory-reseed-decisions-20260703.md` + `docs/frozen/routed-memory-reseed-amendments.md` (A1–A7).
**Gate record:** gate-plan run 2026-07-04. A1 follow-up discharged 2026-07-04: S1 bias-lens red-team (6 lenses per brain-soup ADR-0005 basis), 22 findings → A2–A7 + plan constraints. S2 red-team 2026-07-04: 3 lenses (traceability/autonomy/technical), 15 findings, all folded → v0.1 (route_tag+source columns; byPath normalization exemption; WP-2b hoist; RB1 split; dialect rejection surface + named oracle; CLI flag table; §11 asserts into P4; R7 window semantics into P11/P13). Codex consultant review (multipass, 2 turns, 2026-07-04): 3/3 owner calls AGREE, material gaps none; adversarial probe folded — RB1(b) hoisted to build start, Call-1 cold-start pushback recorded with a delta option for the owner.
**trace-lint:** PASS 2026-07-04 (`~/JangLabs/jskills/workflow/bin/trace-lint.sh <spec> <plan> <amendments>`), re-run pending at freeze.
**Freeze-commit:** `69184e7185d9a9c10e371fd3f80ec9bb1e1ddf46` (/vet's DIFF anchor).

<!-- Trace grammar per WORKFLOW.md §4: every P ends `← refs`. -->

## Plan

- P1: Cargo skeleton: one static `rejolt` binary, zero runtime deps beyond the binary; local gates = `cargo fmt --check`, `clippy --all-targets -- -D warnings`, `cargo test`; fixtures tree; G2 discipline scaffold — every conformance check lands with a known-good AND a known-bad fixture before its verdict counts ← D16, D20
- P2: Frontmatter dialect: the constrained-YAML subset frozen per Appendix B2 (accept AND reject surfaces enumerated — the builder implements the boundary, never invents it) + the hand-rolled parser; oracle per A3+B2: differential agreement with the named reference parser on the in-subset corpus, generate→parse→regenerate round-trip, §3 examples + B2 rejection cases as the fixture corpus; deny diagnostics cite the violated dialect rule ← D21, A3
- P3: `grammar.toml` loader: serde structs with `deny_unknown_fields` on root and entries, engine-side one-facet-per-tag cross-check, `grammar-version` validated, evidence non-empty (synonyms-only fails, exit 2), placement field, vocab digest rendered from parsed data ← D22, D23, D3, A6
- P4: `rebuild`: store scan → two artifacts, each write-temp-then-rename, index-first/report-last with shared generation id + sourceFingerprint; routabilityReport (count+ids); ranking-only writes do not rebuild; the §11 drift guardrail built here as a fail-open advisory run at rebuild and session-start — (1) no existing trigger-bearing memory is a bare-degenerate-only set, (2) no curated memory would be BLOCK-degenerate under the current verdict, (3) the D10 routing-vs-ranking partition holds; build-time key normalization per Appendix A (byPath exempt); control-char policy — routing fields excluded+reported, display fields sanitized ← D2, D14, D24, A2, D10, D5, D8
- P5: Flat recall index as the sole routing structure: four tables, 13 frozen columns per Appendix A (incl. `route_tag` + `source` — the grammar-tag label recall citations and telemetry attribution require; S2 technical blocker), one record per physical line, no escaping layer, pre-flattened rows; byPath rows keep raw glob patterns and are matched by prefix/glob scan — the other three tables are exact-key after normalization; ONE walk module consumed by recall, projection, and liveness ← D24, D4, A2, A3, D15
- P6: Recall path: token extraction from normalized ops; surface gate (≥1 strong OR ≥2 total); tier weights + stale/decline penalties; generic-verb stop-lists; per-memory dedup window and fire-telemetry append through the P11 primitive (mark persistence gates fire logging); evidence citations `{route_tag} <- {type}:{value}` (format from CORE-SPEC §2.6/§5, spec-standing where the ledger is silent); advisory rendering under budget caps; index-only invariant — never rebuilds, never loads bodies, silence = empty output ← D1, D3, D5, D19, D25, A7
- P7: Host-event parser → NormalizedOp per R3 freeze (Appendix B): closed v1 tool set, `is_full_write` = content presence decided once in the normalizer, unknown fields ignored / missing optionals tolerated, parse failure = fail open silent, adapter-lexical (`realpath -sm` semantics) vs engine-realpath canonicalization split preserved ← D19, A5, D15
- P8: Hook entry `rejolt hook <event>`: internal dispatch (recall / write-guard / write-context / rebuild-refresh / read-signal / floor+maintenance); quiet pass path; `.surface-disabled` kill-switch; only the write-guard branch may exit 2, and a write-guard deny SHORT-CIRCUITS the invocation — stderr + exit 2 only, recall/write-context output suppressed; SessionStart ordering (telemetry marker + maintenance check before the at-home floor skip); session-start advisory lines: routability delta (the D18 report's reader) + §11 drift guardrail ← D12, D19, D6, A5, D18
- P9: Write guard: deny enumeration — shape/evidence, static degeneracy (vocab present), dedup backstop (0.85, new-file only), BLOCK-degenerate, high-confidence misplacement, plus the A6 grammar-write diff-aware surface; partial edits fail open; write-context payload (schema + vocab digest + dedup candidates + placement) under budget ← D6, D21, D5, A5, A6
- P10: Collision projection: the same walk module, ungated/unscored; per_trigger, distinct_count, live_levers; liveness is INDEX-KEY MEMBERSHIP (byArg∪bySynonym key set + lexical path specificity), computed independently of the co-fire walk — never inferred from hit counts (the retired signal-inversion, Appendix A of CORE-SPEC); verdicts PASS / GUIDE-broad / BLOCK-degenerate with strict > floor; new-file-only; empty projection on any fault ← D8, D4, A2
- P11: Marks + telemetry primitive (foundational — WP-2b): mtime-only empty files under XDG runtime dir; TTL single-sourced from config; ONE code path for mark-write/mark-check/telemetry-append shared by recall (P6), read-signal and session marker (P8), and curation reads (P12); correlation gating per A7's corrected wording; 1 MB rotation, one generation, `.1`-first read order, bad-ts symmetric drop; the effective curation window is explicitly min(30 days, rotation bound ≈2×`_TEL_MAX`) and the reader exposes which bound was hit (R7); mark-dir writability check + inert-telemetry advisory ← D25, A7, D7
- P12: Curation: maintenance pass with ≥50-record trigger recheck-under-lock, claim-before-mutate, O_EXCL lock + rename-to-corpse reclaim; the three floors exactly (zero-fire, min-evidence OR-guard, seat dual-gate); window semantics from P11 (min-bound, R7); seats governance — PENDING-SEAT-CHANGES block replaces, never stacks; non-block content byte-identical; never deletes or rewrites bodies ← D7, D25
- P13: Bench + calibration: four verdicts, NOBASELINE interim; calibration per A4 (factor 3.0, slack = max(3σ, min→max band), budget from synthetic-1000, baseline from real store, env key = CPU+governor+power, kernel = metadata, loud degrade on mismatch); environment fingerprint in the committed baseline; calibration also measures the real telemetry record rate and resizes `_TEL_MAX` if 30 days does not fit the rotation bound (R7); one reviewable calibration commit at first end-to-end run (discharges R1) ← D9, D26, A4, A7
- P14: Bootstrap: empty store (grammar seed = the version line alone; minimal `MEMORY.md` router); idempotent; never overwrites user files; fail-open verification suite (`.surface-disabled`, missing-index allow without rebuild-on-read, runtime mark-dir writability per A7) — built here, bootstrap-local rows gated in WP-7, recall/kill-switch rows asserted in WP-8; `--print-hooks` emits the settings block — the engine never writes host settings or permission policy ← D13, D17, D23, D19, A7, OWNER(2026-07-04: R5 empty grammar seed confirmed)
- P15: CLI surface: nine subcommands + hook mode with one-line per-command rationale (scope-lens fold: `search`/`project`/`maintain`/`seats` justified as the direct probe/inspection surfaces the seat dual-gate and legibility value require); flag/output/exit contract frozen per Appendix D — WP-7's surface is authoritative, WP-8 consumes it verbatim; direct CLIs loud and fail-closed on missing deps; exit taxonomy split per A5; config.toml via serde, unknown keys warn-only ← D20, D12, A5, D10
- P16: Conformance harness (cross-cutting sweep; per-row build parents are the P-items in the coverage map): every CORE-SPEC §14 row as a test plus new rows — write-guard deny contract fixture (exit 2 + stderr from the guard branch, exit 0 quiet elsewhere; the LIVE-host half is RB1(b), owner-discharged at /vet), malformed payload passes silently, fourth facet table and duplicate-facet tag exit 2, one-line-per-entry under hostile field content, cross-artifact generation match, unknown-tool fail-open, recall≡projection same-hit-set, end-to-end byPath glob fire, citation renders the grammar route_tag — each with known-good and known-bad fixtures ← D5, D6, D7, D8, D9, D13, D16, D17, A2, A5, A6
- P17: Negative-contract sweep sheet (cross-cutting): N1–N14 mapped to mechanical checks (grep/clippy/test assertions) handed to `/vet` ← D11, D3, D4, D13, D16, D17, D18, D21, D22, D26, A4
- P18: Legacy fence: no import flag, no legacy-format parsing anywhere; legacy corpus disposition recorded as an accepted risk (RB7) with the store measured at freeze ← D17, D18, D15

## Orphan ledger

Considered and cut (no spec parent), or escalated:

- One-shot legacy body-copy migration helper — cut: D17 rejected every import path; a body-copy tool is import-lite through the back door. Escalated to owner as optional, outside this plan.
- Edit/MultiEdit full-file reconstruction (better guard coverage) — cut: contradicts D6's partial-edits-fail-open contract; recorded as the accepted limitation in A5(c).
- Generalized/configurable MCP tool matchers — cut: v1 carries the proven matcher strings verbatim (R6); generalization is post-v1.
- Engine self-registration of hooks (writing settings.json) — cut: D13's spirit; `--print-hooks` instead (R6).
- `byMemoryId` table in the flat index — cut: dead on the old read path, producer removed by D18 (A2).
- `name` column in the flat index — cut: never rendered on the advisory path; memory metadata lives in the JSON report (R2).
- Deferring `project`/`seats`/`maintain`/`search` to post-v1 (scope-lens recommendation) — cut the deferral: owner froze the nine-command shape (D20); per-command rationale recorded in P15 instead.

## Coverage check

All of D1–D26 and A2–A7 are covered by at least one P-item (D1:P6 · D2:P4 · D3:P3,P6,P17 · D4:P5,P10,P17 · D5:P4,P6,P9,P16 · D6:P8,P9,P16 · D7:P11,P12,P16 · D8:P4,P10,P16 · D9:P13,P16 · D10:P4,P15 · D11:P17 · D12:P8,P15 · D13:P14,P16,P17 · D14:P4 · D15:P5,P7,P18 · D16:P1,P16,P17 · D17:P14,P16,P17,P18 · D18:P8,P17,P18 · D19:P6,P7,P8,P14 · D20:P1,P15 · D21:P2,P9,P17 · D22:P3,P17 · D23:P3,P14 · D24:P4,P5 · D25:P6,P11,P12 · D26:P13,P17 · A2:P4,P5,P10,P16 · A3:P2,P5 · A4:P13,P17 · A5:P7,P8,P9,P15,P16 · A6:P3,P9,P16 · A7:P6,P11,P13,P14). A1 is provenance-only (no build impact). R5 lands in P14 as an OWNER ref at freeze.

## WP decomposition (T2)

| WP | Scope | Depends on | Gate |
|---|---|---|---|
| WP-0 | P1 skeleton + G2 scaffold | — | fmt + clippy + test green; selftest harness proven (known-bad fails) |
| WP-1 | P2 + P3 parsers | WP-0 | gates + differential/round-trip tests + §3/B2 fixtures |
| WP-2 | P4 + P5 rebuild, flat index, walk module | WP-1 | gates + torn-pair, hostile-content, byPath-glob fixtures |
| WP-2b | P11 marks + telemetry primitive | WP-2 | gates + correlation/rotation/writability fixtures |
| WP-3 | P7 + P6 host events + recall | WP-2, WP-2b | gates + surface-gate matrix + dedup-window/fire-append fixtures (a stubbed telemetry path cannot pass) |
| WP-4 | P9 + P10 write guard + projection | WP-2 | gates + deny-enumeration + same-hit-set fixtures |
| WP-5 | P8 hook modes + wiring | WP-3, WP-4 | gates + engine-contract deny/allow fixtures (RB1(a)); RB1(b) is a human-only step, see Budget |
| WP-6 | P12 curation | WP-2b, WP-3 | gates + floors/concurrency fixtures |
| WP-7 | P13 bench/calibration scaffold + P14 bootstrap + P15 CLI | WP-2 | gates + idempotence + bootstrap-LOCAL fail-open rows only |
| WP-8 | P16 + P17 + P18 conformance completion + N-sweep + fence | all | full §14 matrix green (recall/kill-switch fail-open rows land here); N-sheet handed to /vet |

Sole committer after WP-0: the integrator agent (opus) on `wf/routed-memory-reseed`; packet builders work in worktrees and commit there first (G7).

## Risk register (feeds /vet)

- RB1: Hook-mode deny must actually block under the live Claude Code build — split per the autonomy lens: (a) engine-contract fixture, unattended, gates WP-5 (exit 2 + stderr from the guard branch, exit 0 quiet elsewhere); (b) LIVE-host verification — a human-only step, **discharged at build START, not build end** (Codex consultant review 2026-07-04: the sole deferral with owner-regret potential — everything fail-closed rests on it). Mechanism needs no rejolt code: a minimal probe hook (exit 2 + stderr on a PreToolUse matcher) registered in a fresh session + one deliberate matching tool call, observed blocked. /vet verifies the record exists. Doc evidence alone never closes (b).
- RB2: Flat-index one-record-per-line invariant under hostile field content (tabs/newlines in patterns, snippets, paths) — fixture-forced (A2).
- RB3: Bespoke frontmatter parser accept/reject boundary — B2 enumerates the boundary; differential oracle + round-trip (A3/B2); a false deny here is the #1-rule violation.
- RB4: Cross-artifact generation consistency across a crash mid-rebuild — torn-pair fixture (A2).
- RB5: `deny_unknown_fields` + cross-facet check actually reject the fourth table and duplicate-facet tag (A6) — known-bad fixtures.
- RB6: Telemetry window = min(30 d, rotation bound) — now carried in P11/P12/P13 (S2 traceability fold), measured at calibration; `_TEL_MAX` resized if 30 days does not fit.
- RB7: Legacy corpus disposition — measured 2026-07-04: box-brain 30 memories, all project stores combined 70 (well under the 165-memory synapse milestone); empty bootstrap means the routable corpus is re-authored by hand or abandoned; owner accepted (D17), reconfirmed at freeze with the number known.
- RB8: Reboot-straddling fires deflate read_rate (A7) — bounded by the three floors; calibration quantifies if curation misbehaves.
- RB9: Recall≡projection parity through the one walk module — same-hit-set fixture (§14 row); liveness is key membership, never hit counts (P10).
- RB10: Env-fingerprint predicate on a rolling-kernel box — kernel excluded from the gate key (A4); conformance asserts loud degrade on mismatch.
- RB11: byPath axis survives build-time normalization — raw-glob exemption (Appendix A) + end-to-end glob-fire conformance row (S2 technical fold).

## Budget (T2)

- Live-quota ceiling: /build fan-out declared at ≤ 2.5 M subagent tokens across all packets; rule of two per packet — two failed gate attempts on the same packet stop the packet and escalate to the integrator tier; a BLOCKED (un-runnable) gate escalates immediately, without burning attempts (S2 autonomy fold).
- Human-only steps, batched up front (High-Stakes protocol): (1) RB1(b) live-host deny probe — at build START (see RB1); (2) R5 grammar-seed confirmation; (3) ratification of A2–A7 at freeze. Nothing else in the build requires the owner.
- Model economics (per `~/.claude/rules/fable-usage.md`): packet builders sonnet; integrator/sole committer opus; red-team and walk-back reviewers opus (one tier above builders); Fable only at session-level adjudication points (G4 spec-friction, arbitration) — no Fable subagents without qualified permission.

## Appendix A — R2 freeze detail (flat index)

Columns (tab-separated, one record per physical line): `table, pattern, route_tag, source, memory_id, trigger_type, tier, type, lastReviewed, declineCount, tags, path, snippet` (13 — S2 technical fold added `route_tag`/`source`). Tables: byCommand/byPath/byArg/bySynonym. `source` ∈ {t, m}: `t` = grammar-tag route (`route_tag` = the tag name — what recall citations, matchedTags/canonicalTags filtering against the active vocab, and telemetry attribution consume), `m` = per-memory trigger (`route_tag` = memory_id). Keys: byCommand/byArg/bySynonym normalized at build (strip/lowercase; TAG_RE conformance at validate — fixes the old raw-key/normalized-lookup asymmetry); **byPath rows are EXEMPT** — raw glob pattern preserved (case- and slash-bearing), matched by `/**`-prefix + fnmatch scan, never exact-key (S2 technical fold; `project_triggers` pitfall 5). tags joined with `,` (TAG_RE forbids commas). tier precomputed at build from the hardcoded type→tier map (one module, column generated from it). snippet = description, entity-escaped and truncated to maxDescriptionChars at build. Routing fields containing `\t`/`\n`/`\r`: entry excluded + listed in routabilityReport. Ground truth: field-consumption inventory over `memory_surface.py:2112-2362`, `:515-650`, session record 2026-07-04.

## Appendix B — R3 freeze detail (NormalizedOp)

Serde enum: `session-start {cwd}` · `pre-op(ToolOp)` · `post-op(ToolOp)`. ToolOp: `tool_name: String`, `raw_tool_input: Value`, `cwd: Option<PathBuf>`, `command_text: Option<String>` (Bash only), `target_path: Option<PathBuf>`, `bash_embedded_paths: Vec<PathBuf>`, `proposed_content: Option<String>`, `is_full_write: bool` (= content presence; decided once). Closed v1 tool set: Bash, Read, Edit, Write, MultiEdit, WebSearch, WebFetch, MCP-context7 matcher. Bash parity: segment on `;`/`&&`/`||`/`|`/newline; strip privilege (`sudo`/`doas`/`pkexec`/`env`) + runner value-flags + `VAR=val` prefixes; naive surrounding-quote strip (documented parity choice); basename via last `/`. Unknown tool/field posture per A5. Ground truth: extraction table over `memory_surface.py:1776-1880`, `memory-write-guard.sh` content-presence classification, session record 2026-07-04.

### B2 — Frontmatter dialect boundary (S2 autonomy fold: the builder implements this, never invents it)

**In-subset (must parse):** `---` fences; top-level `name`/`description` as single-line plain or single/double-quoted scalars; a `metadata:` nested map at 2-space indentation carrying `tags`, optional `triggers` (exactly `commands`/`paths`/`args`/`synonyms`), and ranking fields; sequences in BOTH flow (`[a, b]`) and block (`- x`) form; full-line `#` comments; UTF-8.
**Out-of-subset (must reject, with the rule named in the deny):** anchors/aliases (`&`/`*`), type tags (`!!`), multi-document (`---` mid-file), block scalars (`|`/`>`), flow mappings (`{}`), multiline strings, tab indentation, duplicate keys, top-level `triggers:` (D21 — its own named error), unknown top-level or metadata keys.
**Oracle (named):** differential agreement with a pinned reference YAML parser run OUT-OF-PROCESS in the test harness only — pinned PyYAML `safe_load` (dev/test dependency; N10 untouched: nothing but the static binary on any ENGINE path) — over the in-subset corpus; a committed vector corpus (input → expected parse or named rejection) is the fallback oracle if the build host lacks Python; round-trip generate→parse→regenerate on all in-subset vectors.

## Appendix C — v1 host-profile facts (R6)

Claude Code, from the proven synapse wiring: PreToolUse deny = exit 2 + stderr (exit 1 does not block); advisory injection = `hookSpecificOutput.additionalContext` (+ `suppressOutput: true`); PostToolUse cannot block, may surface one loud correction; SessionStart re-fires on startup/resume/clear/compact (floor self-heals across compaction); matcher strings carried verbatim from `settings.global.fragment.json`; settings placement user-global; the engine never writes them (`--print-hooks`).

## Appendix D — CLI flag/output contract (P15; discharges D20's "flags at plan stage"; WP-7 authoritative, WP-8 consumes verbatim)

| Subcommand | Flags | Output / exit |
|---|---|---|
| `bootstrap` | `--store DIR --grammar FILE [--print-hooks]` | loud creation report; hooks JSON to stdout with `--print-hooks`; 0 ok / 1 failed check / 2 usage |
| `rebuild` | `--store DIR [--json]` | routabilityReport summary (JSON with `--json`); 0 / 1 / 2 |
| `validate` | `--store DIR [--grammar FILE]` | findings list; 0 clean / 1 findings / 2 config-taxonomy |
| `check-write` | `--store DIR --target PATH` (content on stdin) | guard verdict + reason, loud; 0 pass / 1 deny / 2 usage |
| `project` | `--store DIR` (triggers JSON on stdin) | projection JSON (collisions, per_trigger, distinct_count, live_levers, verdict); 0 / 2 |
| `search` | `--store DIR [--json] [--expect ID]` (event JSON on stdin) | results (advisory text or JSON); 0; with `--expect`: 1 when ID absent (the seat-probe form) |
| `maintain` | `--store DIR [--force]` | pass summary; 0 / 1 / 2 |
| `seats` | `--store DIR [--propose]` | seat report / pending-block write; 0 / 1 / 2 |
| `bench` | `--store DIR [--samples N] [--update-baseline] [--calibrate]` | verdict line + numbers; 0 PASS/WARN/NOBASELINE / 1 REGRESSED / 2 usage |
| `hook` | `session-start\|pre-op\|post-op` (payload on stdin) | per A5: quiet allow exit 0; write-guard deny exit 2 + stderr (short-circuit); never exit 1 |
