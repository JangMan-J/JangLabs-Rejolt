# routed-memory-reseed — decisions (D-ledger)

**Status: FROZEN — 2026-07-03.**
**Tier: T2 (confirmed by owner 2026-07-03)** — axis scores: blast 2, revert 1, decisions 2, stakes 1, duration 2 (total 8).
**Produced by:** /distill → `grilling` engine (fallback; Codex transport absent — announced), interviewer Claude Fable 5 (`claude-fable-5`); wire-format extraction by a Sonnet subagent over the synapse engine; recall-path benchmark run in-session.
**Inputs:** `CORE-SPEC.md` (draft-in-hand), `docs/reports/salvage-20260703.md`, synapse engine `../synapse/lib/memory_surface.py` + `hooks/*.sh` at commit `26f1691` (extraction findings with line citations), recall benchmark (methodology + results in D16/D24 below).
**Precedence:** this document wins over its inputs — including `CORE-SPEC.md` where they conflict (substrate, grammar format, legacy import, fallback derivation, CLI, perf magnitudes). Where this ledger is silent, `CORE-SPEC.md` stands as the detailed behavioral contract.

**Provenance split.** D1–D14 are **RECONSTRUCTED 2026-07-03 from `docs/reports/salvage-20260703.md` §6** — distilled from CORE-SPEC.md and synapse ADRs, evidence cited per entry; they are not contemporaneous owner statements. D15–D26 are contemporaneous owner decisions from the 2026-07-03 /distill grill, owner-attribution flagged inline.

---

## Part A — reconstructed core contract (evidence-cited)

## D1. Spend intelligence at write time; recall is a precomputed lookup

- The owner's founding tenet, restated 2026-07-03: *speed at recall time, thorough at write time.* All derivation, linking, ranking at memory-write/rebuild; per-operation recall is index lookup only — no LLM, no embeddings, no rebuild, no memory-body loads.
- Evidence: CORE-SPEC §2.1, §5; synapse ADR-0004.
- Rejected: read-time cleverness of any kind — every past attempt grew latency and complexity (synapse history).

## D2. Store is source of truth; compiled artifacts are disposable

- The store (markdown memories + infra files) is authoritative; every compiled routing artifact is rebuildable at will, never edited, never migrated. A format change is a rebuild, not a migration.
- Evidence: CORE-SPEC §2.2, §4; ADR-0008.
- Rejected: treating the index as data (migration obligations follow).

## D3. Route on observed behavior, never prompt text

- Recall keys on commands run, paths touched, arg tokens — never prompt text. A grammar tag with no behavioral evidence fails validation (a tag **is** its evidence patterns).
- Evidence: CORE-SPEC §2.3, §3; ADR-0003 (prompt-keyword routing implemented once, rolled back as noise).

## D4. One matcher

- Recall and write-time collision projection walk the same index path. There is no second matcher, in any language or layer (see N1, N11).
- Evidence: CORE-SPEC §2.4, §7; ADR-0015.

## D5. Precision over recall; the surface gate

- Silence is the default. Gate form frozen: fire iff ≥1 strong-tier tuple OR ≥2 tuples total; tier map command/path=strong, arg=medium, synonym=weak is hardcoded; generic-verb stop-list; per-memory dedup window. Magnitudes are §10 tunables.
- Evidence: CORE-SPEC §2.5, §5, §10; ADR-0005.

## D6. Fail open everywhere; exactly one fail-closed boundary

- Missing engine/store/index or unexpected error never blocks a host operation. The single fail-closed surface is a full-file write of a frontmatter-bearing memory, deniable only for: invalid shape/evidence, static degeneracy (with index vocab present), duplicate new-file backstop, BLOCK-degenerate collision, high-confidence misplacement. Partial edits always pass.
- Evidence: CORE-SPEC §2.7, §2.9, §6; ADR-0011.
- Wording correction (extraction finding): synapse's `search()` returns a schema-shaped empty response, not `None`; the reseed's contract is **empty output at the adapter surface on silence** — no envelope (see D12/D19).

## D7. Self-curation demotes and flags; never deletes, never rewrites bodies

- Telemetry-driven curation; no standing review ritual. Three floors, all load-bearing: zero-fire memories never demote; minimum-evidence guard (≥10 distinct session-days OR ≥30 days span); seat demotion requires probe coverage AND telemetry fires. Claim-before-mutate and recheck-under-lock concurrency rules carry over.
- Evidence: CORE-SPEC §2.8, §8; ADR-0006, ADR-0007.

## D8. Collision verdict reads live levers, not co-fire counts

- Verdicts: PASS / GUIDE-broad (advisory) / BLOCK-degenerate (deny). BLOCK requires breadth strictly > floor AND empty `live_levers`. Liveness = routability, shared definition with the static gate. The superseded co-fire-count model false-denied curated memories at 165-memory scale (measured, synapse milestone).
- Evidence: CORE-SPEC §7, Appendix A; ADR-0019.

## D9. Performance gate is regression-relative in form

- Four verdicts (PASS/WARN/REGRESSED/NOBASELINE); only REGRESSED blocks; baseline committed and auditable. Form frozen here; **all magnitudes are deferred to the D26 calibration protocol** (supersedes CORE-SPEC §9's 55 ms budget and `max(25%, 15 ms)` slack, which were python-stack calibrations — see D16).
- Evidence: CORE-SPEC §9; ADR-0018.

## D10. Tunables vs invariants partition

- The contract is shape; magnitudes are config unless frozen by a D-entry. Routing-affecting writes rebuild; ranking-only metadata writes do not; a field that becomes both must trip the drift guardrail.
- Evidence: CORE-SPEC §10, §4, §11.

## D11. Non-goals fence

- No GSD/planning spine; no SQLite/FTS5 on the routing path; no embeddings/LLM on the read path; no prompt-keyword routing; no standing review ritual; no bulk-LLM trigger derivation; no second matcher; no per-corpus cutoff beyond the single collision floor. Host-runtime base harness and corpusforge remain out of scope.
- Evidence: CORE-SPEC §12; ADR-0002.

## D12. Adapter discipline: quiet handlers, loud direct CLIs

- Adapter handlers: no output on the pass path, allow on failure (fail-open), diagnostics only for actionable errors. Directly invoked CLIs invert: loud on success, fail closed on missing dependencies.
- Evidence: CORE-SPEC §2.10, §10; ADR-0014.

## D13. The engine never writes host permission policy

- No `allow`/`deny`/`defaultMode` or equivalent, ever — including bootstrap.
- Evidence: CORE-SPEC §0, §12, §13.

## D14. All store mutations are atomic

- Write-temp-then-rename for every store/index mutation; a crash never leaves a half-written memory or index. (CORE-SPEC §2.11's *stdlib-only Python* clause is superseded by D16; atomicity survives the substrate change.)
- Evidence: CORE-SPEC §2.11, §0.

---

## Part B — grill decisions, 2026-07-03 (contemporaneous)

## D15. Wire formats are clean-slate; synapse is reference, not constraint

- All seven residual formats (R1–R7 of the salvage report) redesigned freely; the synapse engine remains citable evidence of de-facto behavior, nothing more. **Owner rationale, recorded** (overrode the interviewer's split-compat recommendation): third rewrite; "focus the idea, not the parts" — synapse is a junkyard of parts around a genuinely neat idea.
- Rejected: split posture (surviving-data formats adopted verbatim) — moot under D17; full compat — carries the warts forward.

## D16. Substrate: one all-Rust static binary

- The engine is a single Rust binary (see D20 for its CLI). Measured on this box (serial, 40 runs after warmup, same event, three corpus scales — full table preserved here as the deciding evidence):

  | Candidate | 26 mems p50/p95 (ms) | 200 | 1000 |
  |---|---|---|---|
  | synapse engine (python) | 35.0 / 39.3 | 35.5 / 39.6 | 42.6 / 48.1 |
  | minimal python + split index | 15.5 / 19.9 | 15.9 / 19.1 | 21.5 / 26.7 |
  | jq only (JSON index) | 2.5 / 2.9 | 4.2 / 5.0 | 11.1 / 14.0 |
  | jq + grep (flat TSV) | 3.1 / 3.7 | 3.1 / 3.7 | 3.4 / 4.3 |
  | **rust (flat TSV index)** | **0.7 / 0.9** | **1.0 / 1.2** | **2.4 / 3.2** |
  | rust (serde JSON index) | 1.0 / 1.2 | 2.3 / 3.1 | 8.3 / 10.1 |
  | python spawn floor | 9.6 / 11.2 | — | — |

- The python interpreter spawn alone (~10 ms) exceeds the entire measured Rust recall path at every scale; format choice inside python is noise against that tax. Flat-text index + compiled lookup is scale-flat 26→1000.
- Aligns with the account-level Rust-primary rule (`~/AGENTS.md`). The *ideas* stdlib-python served — zero runtime deps, no daemon, crash-safe, inspectable — all carry over: static binary, plain-text store and index.
- Rejected: Rust-recall + python-write split (two toolchains; D4's one-matcher rule would have to span languages); POSIX-only jq+grep (viable at ~3 ms but complex write-path logic in shell is how the junkyard formed); status-quo python (measured: 64–77% of the old budget consumed at spawn).

## D17. No legacy import

- `--import-legacy` is dropped entirely. The reseed bootstraps an **empty** store; existing synapse-era memories migrate by hand or not at all. **Owner rationale, recorded.** CORE-SPEC §13 step 4 and the `routabilityReport: 0 unroutable` cutover gate for imports are void; bootstrap on an empty store trivially satisfies routability.
- Rejected: one-time frontmatter conversion (still couples the new engine to a migration pass); wrap-in-place legacy reader (permanent compat shim = full-compat posture through the back door).

## D18. Fallback trigger derivation is removed

- No `derive_fallback_triggers`, no `source = memory-derived` routes, no `byMemoryId` fallback lifecycle. Every route is **declared** (grammar or frontmatter), never inferred from body text. Hand-created routeless files surface in `routabilityReport`, advisory-only.
- With D17 the mechanism's only consumer is gone; the readiness review had flagged its rules as under-deterministic (handoff 2026-06-20) — deleting beats specifying.

## D19. v1 host profile: Claude Code hooks, engine parses host events natively

- The sole v1 conformance profile is Claude Code's hook interface (SessionStart / PreToolUse / PostToolUse; JSON on stdin; exit-code + stdout semantics). Hooks invoke the engine binary **directly** — no jq preprocessing, no shell tokenization; the engine deserializes the host payload itself (kills synapse's WR-08 newline-flattening quirk and one process spawn).
- The **normalized operation** is an internal typed struct the host-event parser produces: operation kind (session-start / pre-op / post-op), tool name, structured tool input, resolved target paths, command text, cwd, and — for guarded writes — the full proposed file content plus full-write/partial-edit classification. Core engine consumes only normalized operations; nothing Claude-specific below the adapter layer (CORE-SPEC §0 host premise unchanged). Field-level freeze is R3 (plan-stage).
- On silence the hook emits **nothing** (D12) — no empty envelope (synapse emitted one and relied on the shell to suppress it; extraction finding).

## D20. CLI: one multiplexed binary, `rejolt`

- **Owner-named.** Subcommands (shape frozen; flags at plan stage): `bootstrap`, `rebuild`, `validate`, `check-write`, `project`, `search`, `maintain`, `seats`, `bench`, plus the hook entry mode(s) per D19. Exit taxonomy: `0` success, `1` failed check / operational failure (REGRESSED, gate deny), `2` usage/config/taxonomy error. Hook modes obey D12 (quiet, fail-open); direct modes are loud and fail closed on missing dependencies.
- Name collision checked 2026-07-03: `rejolt` free on PATH and in pacman.
- Rejected: script family (synapse-style; multiplies names, no capability); deferring the name (owner chose to fix it now).

## D21. Memory frontmatter: the one bespoke-parsed surface

- Memory files stay markdown with `---` frontmatter (host-ecosystem convention — the Rust-rules ecosystem exemption, noted). The frontmatter dialect is a **formally specified constrained-YAML subset**, the *only* hand-rolled parser in the system, property-tested.
- Schema: top-level `name`, `description`; `metadata:` block with `metadata.tags` — **required, ≥1 entry**, kebab-case (`TAG_RE` shape) — and optional `metadata.triggers` with exactly four string arrays (`commands`/`paths`/`args`/`synonyms`); ranking metadata (`lastReviewed`, `declineCount`, …) per CORE-SPEC.
- Tags required because an empty tag set caps dedup similarity at 0.4 (below the 0.85 backstop — the fail-closed dedup boundary would silently never fire) and makes placement unenforceable. Unknown-to-grammar tags: advisory nudge with closest-match, never a deny (§6 fail-open posture preserved).
- **Top-level `triggers:` is rejected at the write guard** — CORE-SPEC claimed this but synapse never implemented it (extraction: parsed as inert top-level string); the reseed makes the written contract true.

## D22. Facets: a closed, amendment-governed 3-axis set

- **Owner rationale, recorded:** the facet axes are the hard-learned anti-junkyard mechanism — the closed-world governor keeping the routing vocabulary "precisely scoped indefinitely." The names (`domain` / `tool` / `pattern`) are incidental; the **count and definitions** are the contract.
- Every grammar tag declares exactly one facet. A proposed tag fitting no axis is **denied at write time** (that is the scoping gate working). Changing the axis set (add/remove/redefine) requires a spec amendment (`/flow amend`, A-entry with evidence) — never a casual grammar edit.
- Interviewer's initial drop-as-decoration recommendation withdrawn on owner evidence: facets have zero recall-time role by design; their function is write-time scoping (the thorough half of D1's tenet).

## D23. Grammar file: `grammar.toml`, serde-typed

- The vocabulary file — the scoping gate — carries **zero bespoke parsing**: `toml` crate + serde into typed structs. Facets are the top-level tables (`[tool.<tag>]` …); a fourth table name is a deserialization error (exit 2), so D22's closed set is structurally enforced. Fields: `gloss` (non-empty), `placement` (`box`|`project`|`either`), evidence arrays (`commands`/`paths`/`args`/`synonyms`), `related`. A tag whose command/path/arg evidence is all empty fails validation (D3; synonyms alone insufficient). `grammar-version` key required and **actually validated** (synapse's version comment was never read — extraction finding). Empty seed = the version line alone.
- The write-context vocabulary digest is **rendered from parsed data** — doc-quality is the renderer's property, decoupling storage format from prompt text.
- Rejected: markdown + shared frontmatter dialect (interviewer's initial recommendation, withdrawn under owner challenge — its real arguments were synapse continuity and a parser-sharing claim that inverts: keeping the grammar *out* of the bespoke parser shrinks the one hand-rolled surface); synapse's ad-hoc line format (unspecified scanner was the accuracy hole R3 flagged).

## D24. Compiled artifacts: flat grep-provable recall index + JSON report

- `rebuild` emits two views, both disposable (D2): a **flat line-oriented recall index** (TSV-shaped; one trigger entry per line, **denormalized** with everything recall needs as columns — table, pattern, memory id, tier, ranking metadata, advisory snippet) and a **JSON catalog report** (write-side: memories, `routabilityReport`, source fingerprint, vocab) for jq inspection and the write path.
- The recall hot path reads only the flat index: measured scale-flat (D16 table: 0.7→2.4 ms across 38× corpus growth vs 1.0→8.3 ms for JSON-DOM). This literally fulfills D4's "grep-provable": the index is interrogable with grep.
- Column-level schema freeze is R2 (plan-stage).

## D25. Dedup marks: mtime-only runtime files; the correlation invariant is contract

- Marks are empty files, mtime-only, under `${XDG_RUNTIME_DIR:-~/.cache}/<engine>/m_<sanitized-memory-id>` (tmpfs → per-boot self-cleaning; ownership/symlink hardening as in synapse). TTL read from config in exactly one place (synapse declared `dedupeTtlSeconds = 900` but hardcoded `-mmin -15` — latent divergence; extraction finding). One code path inside `rejolt` for mark-write/mark-check/telemetry-append; hooks pass events only.
- **Correlation invariant (promoted to contract):** a fresh mark's presence IS the fire↔read correlation — no timestamp joins; and mark persistence gates fire logging — a fire whose marks did not persist is never logged, so a memory can never be recorded fired-but-unread when reads were unobservable. Bad-timestamp records drop symmetrically (fires AND reads).
- Rejected: in-store marks (runtime state polluting the source of truth, survives reboots); SQLite/JSONL correlation log (a join where a stat suffices; N2).

## D26. Performance magnitudes come from a frozen calibration protocol, not this spec

- **Owner rationale, recorded:** no confidence that compute is as steady in the <10 ms range as at <100 ms; the old regime's model must not be transplanted. Supporting data: benchmark min→p95 relative spread grew from ~30% (35 ms scale) to ~64% (sub-ms scale); laptop governor/thermal states move single-digit-ms numbers wholesale.
- Protocol (frozen): when the real recall path first runs end-to-end, calibrate with ≥100 samples at reference corpora (real store + synthetic-1000), reporting p50/p95 **and cross-run variance**; the ceiling's absolute slack floor is derived from measured jitter, the advisory design budget from measured p95 × a safety factor — the derivation rule is spec, the numbers are outputs. The committed baseline records an **environment fingerprint** (CPU governor, AC/battery, kernel); WARN/REGRESSED verdicts are issued only under a matching environment — otherwise measure-only. Numbers land as one reviewable commit.
- Until calibration is committed, the gate runs NOBASELINE (measure-only, exit 0) — already the contract's behavior, so the interim needs no special case. Magnitudes residual: R1.

---

## Negative contract

- N1. No second matcher — recall and projection share one index walk (D4), in one language (D16).
- N2. No SQLite/FTS5 on the routing path; no embeddings or LLM on the read path.
- N3. No prompt-keyword routing (D3).
- N4. No standing review ritual; curation never deletes or rewrites memory content (D7).
- N5. No bulk-LLM trigger derivation — and per D18, no mechanical body-token derivation either: no inferred routes at all.
- N6. No per-corpus block cutoff beyond the single collision floor (D8).
- N7. No host permission-policy writes, including bootstrap (D13).
- N8. No new facet axis, removal, or redefinition without a spec amendment; no facet-less tags; no tag-less memories (D21, D22).
- N9. No legacy-format parsing code in the engine; no import flag (D17).
- N10. No python (or any interpreter) on any engine path; no runtime dependencies beyond the static binary (D16).
- N11. Recall never rebuilds, never loads memory bodies, never emits output on silence (D1, D19).
- N12. No vendoring of synapse files into bolt — reference by path only (workspace rule).
- N13. Adapter handlers never block a host operation on engine/store/index failure (D6).
- N14. No performance magnitude asserted without the D26 calibration protocol behind it.

## Residual open items

- R1. Perf budget + regression-slack magnitudes — resolve at `/build` via the D26 calibration protocol; `/vet` verifies the calibration commit exists and followed the derivation rule. *Refined by A4 (blueprint session, 2026-07-04): derivation structure, corpus roles, environment predicate, and loud-degrade rule are now pinned; only the measured numbers remain open.*
- R2. **RESOLVED (blueprint session, 2026-07-04)** under D24+A2 — flat-index schema frozen in plan P5: four tables (`byMemoryId` dropped — dead on the old read path, producer removed by D18), rows pre-flattened one per (table, pattern, memory_id), 13 columns (table, pattern, route_tag, source, memory_id, trigger_type, tier, type, lastReviewed, declineCount, tags, path, snippet; `name` dropped — dead on the render path; `route_tag`+`source` added at S2 fold — recall citations and telemetry attribution need the grammar-tag label, unrecoverable on an index-only read path), build-time key normalization for byCommand/byArg/bySynonym (fixes the old raw-key/normalized-lookup asymmetry; byPath EXEMPT — raw glob patterns, scan-matched), one record per physical line with NO escaping layer: routing fields with control chars are excluded+reported at build, display fields sanitized at build. Ground truth: field-consumption inventory over `memory_surface.py` with line cites (plan appendix).
- R3. **RESOLVED (blueprint session, 2026-07-04)** under D19+A5 — NormalizedOp frozen in plan P7: serde enum {session-start{cwd}, pre-op(ToolOp), post-op(ToolOp)}; ToolOp {tool_name, raw_tool_input, cwd, command_text, target_path, bash_embedded_paths, proposed_content, is_full_write}; closed v1 tool set (Bash, Read, Edit, Write, MultiEdit, WebSearch, WebFetch, the proven MCP-context7 matcher); `is_full_write` = `tool_input.content` presence, decided once in the normalizer; NO partial-edit reconstruction (D6 parity); Bash tokenizer parity (segmentation on `;`/`&&`/`||`/`|`/newline, privilege+env-prefix stripping, naive quote strip — documented parity choices); adapter-lexical vs engine-realpath canonicalization split preserved (§5.x). Per-tool extraction table with line cites: plan appendix.
- R4. **RESOLVED (blueprint session, 2026-07-04)** — spec-sync is ship-stage compaction (G6), not a work packet: during /build the ledger+amendments hold precedence and builders consume them directly; editing the 740-line spec mid-build invites drift. At /ship, CORE-SPEC.md is either folded to v2 or SUPERSEDED-bannered toward the authoritative pair (truth in ≤2 documents either way).
- R5. **RESOLVED (blueprint session, 2026-07-04, owner)** — empty seed confirmed (the version line alone). The Codex cold-start pushback and the port-proven-synapse-tags delta option were presented and declined: tags arrive with the behavioral evidence that proves them (D3/D22/D23); hand-porting proven synapse tags stays available at any later time without amendment. Landed in P14 as an OWNER ref.
- R6. **RESOLVED (blueprint session, 2026-07-04)** under D19/D13 — one multiplexed hook entry per event: `rejolt hook session-start|pre-op|post-op`, dispatching internally (recall / write-guard / write-context / rebuild-refresh / read-signal / floor+maintenance); matcher strings carried v1 from the proven synapse fragment; the engine NEVER writes host settings — `rejolt bootstrap --print-hooks` emits the hooks JSON for the host install set to place (user-global placement documented; D13 extended in spirit: no host-policy writes of any kind); marks namespace shared across recall and read-signal (one code path per D25); SessionStart ordering carried (telemetry marker + maintenance check BEFORE the at-home floor skip); host-profile facts (exit-2 deny is the proven block form, additionalContext envelope + suppressOutput, PostToolUse cannot block, SessionStart re-fires on resume/clear/compact) recorded in the plan's host-profile appendix.
- R7. **RESOLVED (blueprint session, 2026-07-04)** — §8/§10 tunables carry unchanged EXCEPT: perf magnitudes (D26/A4 protocol); the telemetry window is explicitly min(30 days, rotation bound ≈2×`_TEL_MAX`) — calibration measures the real record rate and resizes `_TEL_MAX` if 30 days does not fit (red-team coverage finding); config file becomes TOML via serde (D23 pattern, zero bespoke parsing; unknown config keys warn advisory — never fatal on hook paths).

## Pre-build gate

- [x] All D-entries have rationale + rejected alternatives (D1–D14 cite reconstruction evidence; D15–D26 record the grill).
- [x] Negative contract derived (N1–N14).
- [x] Residuals assigned to a resolving stage (R1–R7).
- [x] Tier recorded and owner-confirmed (T2, 2026-07-03).
