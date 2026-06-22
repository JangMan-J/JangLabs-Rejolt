# Synapse Memory — Runtime-Agnostic Core Spec

> **Status:** reseed draft.
> **Scope:** the tag-routed memory subsystem: recall, write-guard, self-curation, catalog, and
> collision projection. Host-runtime base harnesses and corpusforge are out of scope (§12).
>
> **Sources:** distilled from the sibling `synapse` lab, pinned by the parent JangLabs workspace
> gitlink at `26f1691b176f4ffb6056d2a7505f63a477a4a468` on 2026-06-20. Source paths are referenced,
> not vendored: ADRs (`../synapse/docs/adr/`), seed inventory
> (`../synapse/openspec/specs/_PENDING-FROM-GSD.md`), promoted specs
> (`../synapse/openspec/specs/write-guard/`, `…/collision-projection/`), and the live engine
> (`../synapse/lib/memory_surface.py`, the tiebreaker on conflict).
>
> **Provenance:** sections are distilled from working behavior unless marked **[DESIGNED]**. Only §13
> is designed. [`CONTEXT.md`](./CONTEXT.md) holds the one-screen glossary; this file is the contract.

---

## §0 — Purpose & core value

**Purpose.** Surface durable memory from what a session is *actually doing*; otherwise stay silent.

**Core value.** *The right memory surfaces at the right moment through self-curation, and the system
stays legible and maximum-punch-per-pound while doing it.*

The load-bearing commitments are:

- **Behavioral recall:** command/path/arg evidence, not prompt text (§5).
- **Precision over recall:** a false fire is more expensive than a miss (§2, §5).
- **Self-curation:** telemetry governs demotion and seat changes; no standing review loop (§8).
- **Legibility:** fires cite their evidence, tunables are separated from invariants, and generated
  metadata is disposable (§2, §4, §10).

### The punch-per-pound directive

> **Maximum punch per pound.** Per-operation recall must be near-free; heavy work belongs at memory
> write time, session start, or offline rebuilds.

This is a directive, not a mechanism. The current adapter substrate, POSIX-ish shell + `jq` with
Python only for recall, is a finding under the directive. The contract is the cost shape: write-time
intelligence, read-time lookup (§2, §9). Numbers and mechanisms below are tunables or findings unless
§2 names them as invariants (§10).

### Host-runtime premise

This spec is model-agnostic. It assumes a **host runtime** can supply the necessary lifecycle and tool
operation primitives directly or through a thin adapter. The core engine does not depend on any
particular agent model or host product.

The required host capabilities are:

- session-start lifecycle handling;
- pre-operation and post-operation handling for structured tool events;
- a way to surface advisory memory text back to the agent context;
- a way to allow or deny a full-file write before it commits;
- a way to observe memory-file reads and committed memory-file writes;
- a persistent filesystem store with atomic rename semantics available to the engine;
- configurable box, project, and repo memory-store roots;
- quiet success and actionable failure semantics for adapter handlers (exit codes or equivalent
  structured decisions).

If a host cannot provide one of these primitives, that host is unsupported for the affected feature
until its adapter supplies an equivalent.
Runtime-specific conformance profiles may define event/path mappings, but they must not alter the
core engine contract below.

### Constraints (standing, beyond the read/write/curation behavior)

- **Stdlib-only engine:** `lib/memory_surface.py` uses only the Python 3 standard library.
- **Data survives, metadata is expendable:** memory bodies survive; routing/ranking metadata is
  rebuildable and has no migration obligation (§4).
- **No permission-policy writes:** the memory engine never writes host permission policy (`allow`,
  `deny`, `defaultMode`, or equivalent). Permission posture belongs to the out-of-scope base harness
  (§12).

---

## §1 — Glossary (ubiquitous language)

Only the terms load-bearing for the memory subsystem. Corpusforge vocabulary is excluded by scope.
This table is mirrored, condensed, in [`CONTEXT.md`](./CONTEXT.md) as the repo's domain glossary.

| Term | Meaning |
|------|---------|
| **host runtime** | The agent environment that supplies lifecycle events, tool-operation events, filesystem access, and allow/deny behavior. It is outside the core engine. |
| **adapter** | The host-specific glue that maps runtime events into this spec's normalized read/write/telemetry operations and maps engine results back to host allow/advisory/deny behavior. |
| **normalized operation** | The adapter-facing event shape used by the engine: operation kind, tool name, structured input, target paths, command text/args, and, for guarded writes, full proposed file content. |
| **store** | The directory of memory files plus its infra files (`_grammar.md`, `MEMORY.md`, …). The *source of truth*. |
| **memory** | One markdown file: YAML frontmatter (`metadata:` incl. a `triggers:` block) + a body. |
| **grammar** | `_grammar.md` — the single trigger grammar. Each tag declares a gloss, a placement hint (`box`\|`project`\|`either`), and evidence fields. *A tag **is** its evidence patterns.* |
| **placement hint / placement model** | The grammar tag hint (`box`\|`project`\|`either`) plus target-store classification used for write guidance and high-confidence misplacement denial (§3, §6). |
| **trigger** | A behavioral evidence pattern that routes a memory: a command, path, arg token, or synonym. |
| **catalog** | `_memory_catalog.json` — the compiled, rebuildable build artifact holding the routing index. Never the source of truth (§4). |
| **triggerIndex** | The inverted-index section inside the catalog: `byCommand` / `byPath` / `byArg` / `bySynonym` / `byMemoryId`. |
| **routabilityReport** | The rebuild report and catalog metadata listing memories that cannot route; clean cutovers require `0 unroutable` (§4, §13). |
| **memory-derived route** | A catalog-only `byMemoryId` fallback route derived from legacy memory body tokens when no grammar or frontmatter route exists; it is never written to frontmatter (§13). |
| **recall / fire** | A per-operation lookup surfacing zero or more memories. A surfaced memory has *fired*. |
| **tier** | The strength class of a matched trigger: command/path = **strong**, arg = **medium**, synonym = **weak**. |
| **surface gate** | The precision threshold a candidate set must clear to fire (§5). Below it: silence. |
| **fail open / fail closed** | A failure direction: fail open lets the host operation proceed; fail closed denies it, and only the write-guard boundary may hard-deny memory writes (§2, §6). |
| **co-fire breadth** (a.k.a. `distinct_count`) | How many *other* memories a proposed trigger set would also match — the collision quantity (§7). One quantity, two names; this spec uses **co-fire breadth** in prose and notes `distinct_count` as the projection field. |
| **broad path / specific path** | A path trigger classified by the shared lexical `is_broad_path()` rule (§3.x). Broad paths are root/home/current-dir catchalls with no concrete narrowing segment; every other path is specific. |
| **narrowing / live lever** | An explicitly declared trigger that actually *routes* the memory: a routable arg (present in `byArg` **or** `bySynonym`), a routable synonym (in `bySynonym`), or a *specific* path. The **same** definition drives the write-guard static gate (§6) and the collision verdict (§7) — defined once here, referenced from both. |
| **full-file write / partial edit** | A full-file write supplies the complete proposed target file content before commit. A partial edit supplies only a diff, patch, range, shell command, or incomplete post-edit content. |
| **write-guard boundary** | The only fail-closed write surface: a full-file write of a frontmatter-bearing memory. Inside this boundary, §6 enumerates the deny reasons; outside it, writes fail open. |
| **verdict** | The write-time collision outcome: PASS / GUIDE-broad (advisory) / BLOCK-degenerate (§7). |
| **bootstrap** | The designed clean-store procedure, implemented by `bootstrap-store`, that seeds grammar/router files, rebuilds the catalog, and verifies fail-open behavior (§13). |
| **seat** | A memory holding a slot in `MEMORY.md`, the always-loaded router floor. Seat membership is machine-governed (§8). |

---

## §2 — Load-bearing invariants (the frozen shapes)

These rule forms are frozen: changing one changes what the system is. Magnitudes are tunables unless
this section says otherwise (§10).

**Architecture**

1. **Write-time intelligence, read-time lookup.** All derivation, linking, and ranking is spent once
   at memory-write time, session start, or offline rebuild. Per-operation recall is a precomputed
   index lookup: no LLM, no embeddings, no rebuild, and no memory-body loads.
2. **Store is source, index is binary.** The store is the source of truth; the catalog/triggerIndex
   is a disposable build artifact, rebuilt from the store at will, never treated as editable source,
   never migrated.
   A format change is a *rebuild*, not a migration.
3. **Route on observable behavior, never on prompt text.** Recall keys on what the session does
   (commands run, paths touched, arg tokens), never on prompt text. `validate_grammar` rejects a tag
   with no behavioral evidence; a synonyms-only tag fails with exit 2.
4. **One grep-provable matcher.** Recall and write-time collision projection walk the *same* index
   path (`_walk_index`). There is no second matcher.

**Calibration**

5. **Precision over recall.** Silence is the default. Every fire must be justified by behavioral
   evidence; misses are backstopped by the session-start floor and explicit reads.
6. **Diagnosable fires.** Every surfaced memory cites its firing evidence inline as
   `{tag} <- {trigger_type}:{matched_value}`.

**Safety / failure**

7. **Fail open.** A missing engine, unreadable store, missing/corrupt catalog, or unexpected error
   never blocks a host operation; the adapter handler returns an allow decision. In an exit-code
   handler model, this is exit 0. Only an actionable taxonomy/config error may return a host-visible
   hard failure such as exit 2. Read-path function return and adapter-handler outcome are different
   layers (§5, §6).
8. **Never delete autonomously.** Automated curation demotes and flags but never deletes a memory.
   Deletion is outside the automated curation boundary. Memory *content* is never rewritten by the
   curation pass (§8).
9. **One fail-closed write-guard boundary.** On an otherwise advisory system, only a full-file write
   of a frontmatter-bearing memory may be denied. The deny reasons inside that boundary are
   enumerated in §6 (shape/evidence, static degeneracy, duplicate backstop, BLOCK-degenerate
   collision, and high-confidence misplacement). Everything outside the boundary — partial edits,
   frontmatter-less content, any projection fault — fails open.

**Adapter discipline** *(one required rule beyond fail-open, which is invariant 7)*

10. **Quiet on success.** An adapter handler emits no user/context-visible output on the pass path.
    In an exit-code handler model, it exits 0. `stderr` or equivalent diagnostics are reserved for
    actionable failure only. No status/progress lines ever feed host context.

Not invariants here: POSIX+`jq` is a finding under §0, not a rule. "No permission-policy writes" is a
§0 security constraint, not adapter discipline.

**Engine substrate**

11. **Stdlib-only, atomic mutation.** The engine is stdlib-only (§0). All store mutations go through
    an atomic write primitive (write-temp-then-rename), so a crash never leaves a half-written memory
    or catalog.

---

## §3 — Data model

**Memory file.** A memory is one markdown file: a YAML frontmatter block (`---…---`) followed by a
free-text body. Routing-relevant metadata lives under a `metadata:` key in frontmatter.

**The `triggers:` block.** Explicit behavioral evidence lives at `metadata.triggers` — never at the
top level. It holds four string arrays:

```yaml
metadata:
  triggers:
    commands: [nvidia-smi, ...]   # strong tier
    paths:    [~/.config/foo, ...] # strong tier
    args:     [--no-cache, ...]    # medium tier
    synonyms: [vram, ...]          # weak tier
```

- **Top-level `triggers:` is rejected** (parity with how top-level `tags:` is rejected). A trigger
  block at the document root is a write-time error, not silently relocated. *(Frozen shape, §2.)*
- The tier-to-array mapping (commands/paths = strong, args = medium, synonyms = weak) is the same
  mapping recall scores on (§5) and is **hardcoded**, not config.

**The grammar (`_grammar.md`).** The single trigger grammar. Each tag entry declares:
a one-line **gloss**, a **placement hint** (`box` | `project` | `either`), and **evidence fields**
(`commands` / `paths` / `args` / `synonyms` / `related`). It is parsed by a stdlib `re` scanner —
**not** PyYAML/JSON (stdlib-only, §2). It is the lab-resident source of routing vocabulary,
relative-symlinked into the box store, and is the **sole** routing source (it superseded the legacy
`_tags.md` / `_tag_links.md` at the Phase-2 cutover; those remain write-path-only data, never read by
recall). `validate_grammar` enforces invariant §2.3: a tag with no behavioral evidence (e.g.
synonyms-only) **fails, exit 2** — a tag *is* its evidence.

**Placement model.** Every memory belongs to a store, classified by subject. The adapter supplies the
host-specific roots; the engine classifies targets against those configured roots.

- Stores recognized: the **box-brain** store, any host-declared **project** store, and **repo**
  `memory/` dirs (`*/memory/*.md` by default).
- **Infra files are exempt first** — underscore-prefixed files, `MEMORY.md`, `_grammar.md` are
  checked and excluded *before* any placement logic.
- `_classify_target` returns `box | project-store | repo-memory | other` via realpath-normalized
  prefix comparison (which also prevents `../` escape — see §5.x). The grammar's `box|project|either`
  hint feeds this. Enforcement of placement is graduated and lives in §6.

### §3.x — Path specificity classifier

Static degeneracy (§6) and path liveness (§7) both call the same pure lexical classifier:
`is_broad_path(raw)`. It does **not** stat the filesystem, resolve symlinks, or consult the catalog.
It is deliberately shared so a path cannot rescue the static gate while being dead for projection, or
vice versa.

**Normalization for classification only:**

1. Trim surrounding whitespace.
2. Treat `~`, `$HOME`, and `${HOME}` as the same home anchor.
3. Collapse repeated `/` and strip trailing `/`, except for `/` and the home anchor.
4. For `./foo`, drop the leading `./`; for bare relative paths, use the relative path as written.

**Broad iff there is no concrete narrowing segment before the first glob metacharacter** (`*`, `?`,
or `[`), after ignoring empty, `.`, and `..` segments. Anchor-only paths are broad. A concrete segment
is any non-empty segment that contains no glob metacharacter and is not `.` or `..`.

Examples that **must be broad**:

- `*`, `**`, `**/*.md`
- `.`, `./**`, `../**`
- `/`, `/*`, `/**`
- `~`, `~/`, `~/*`, `~/**`, `$HOME/**`, `${HOME}/*`
- `~/.*`, `~/**/settings.json`

Examples that **must be specific**:

- `CORE-SPEC.md`, `src/**`, `./src/**`
- `~/JangLabs/**`, `~/.config/nvim/**`, `~/agent-projects/*/memory/*.md`
- `/etc/modprobe.d/*.conf`, `/var/log/pacman.log`

A recursive catchall after a concrete prefix is allowed (`~/.config/**` is specific); a catchall
before any concrete prefix is broad (`~/**/settings.json` is broad). This exact rule is the operative
definition of "specific path" everywhere the spec uses the phrase.

---

## §4 — The catalog (rebuildable build artifact)

`_memory_catalog.json` is the single compiled routing index. It is not `_routing_index.json`,
SQLite, or FTS5; it stays rebuildable and `jq`-inspectable (§2.2, §12).

`triggerIndex` contains five tables: `byCommand`, `byPath`, `byArg`, `bySynonym`, and `byMemoryId`.
Grammar evidence and per-memory `metadata.triggers` fold into the shared axis tables; `byMemoryId`
carries direct or derived routes for a specific memory. `compile_trigger_index()` builds it.

**`rebuild()` rules.** `rebuild()` reconstructs the entire catalog from store contents in one
command. It is **never treated as editable source**. Consistency is *structural*:

- Every engine mutator (`add_tag`/`link`/`unlink`, via `_mutate_then_validate`) calls `rebuild()`
  **before returning**.
- The post-operation catalog-refresh adapter rebuilds on **any** committed store `.md` write
  (grammar/taxonomy writes arriving through the lab-side symlink are resolved via `readlink -f` —
  see §5.x).
- **Ranking-metadata writes do NOT rebuild.** `lastReviewed` / `declineCount` affect ranking, not
  routing. If a future field affects both, the §11 drift guardrail must surface the partition break.

**Reports & staleness.**

- `rebuild()` emits a **`routabilityReport`** — count and ids of unroutable memories — on stderr and
  in catalog metadata. Cutover requires a real `routabilityReport: 0 unroutable`.
- `fingerprint()` hashes `_grammar.md` into the catalog (`sourceFingerprint`) so staleness is
  mechanically detectable.

**Single reader.** `_load_catalog` is the only catalog reader. It returns `None` for malformed but
parseable catalogs (for example, `memories` is not a list of dicts), so callers fail cleanly instead
of throwing uncaught type errors.

---

## §5 — Read path / recall

On each normalized operation, recall extracts command basenames, canonicalized paths, and arg tokens
from structured operation input, then looks them up in the precomputed `triggerIndex`. `search()`
walks grammar-tag evidence and per-memory triggers over `triggerIndex` / `recallVocab` only. It never
reads legacy `parse_tags_md` / `parse_tag_links`; those are write-path-only.

**Scoring & gate** *(magnitudes are tunables, §10; the rule forms are frozen, §2):*

- `TIER_WEIGHTS = {strong: 10, medium: 6, weak: 3}` (config-overridable via `tierWeights`). The
  type→tier map (command/path/unit/tag = strong, arg = medium, synonym = weak) is **hardcoded**.
- **Surface gate:** a candidate set fires iff it has **≥ 1 strong-tier tuple OR ≥ 2 tuples total**;
  otherwise **silence** (`_meets_min_candidate`). No-evidence operations emit nothing.
- **Score penalties (hardcoded):** `-5 × stale` and `-2 × min(declineCount, 3)`. These affect
  confidence labels against `confidenceHighThreshold = 10`.
- Default false-fire suppressors: a `GENERIC_VERBS` stop-list (generic verbs like restart/install/
  check don't count as strong tokens) and a per-memory-id dedup window (`dedupeTtlSeconds = 900`,
  i.e. 15 min).

**Diagnosable fires (§2.6).** Each surfaced memory cites firing evidence inline as
`{tag} <- {trigger_type}:{matched_value}` (e.g. `nvidia <- command:nvidia-smi`).

**Hard read-path invariant.** `search()` must never rebuild and must never load memory bodies. A
missing/corrupt catalog makes `search()` return `None`; it must not "just rebuild if stale."

**Fail-open layering (one of four fail-directions — see §6 for the full set).** Two different layers:

- `search()` returns `None` on a missing/corrupt catalog.
- The recall adapter handler still returns allow. With `_memory_catalog.json` missing, the handler
  returns allow silently, never a host-visible hard failure such as rc 2.
- **`.surface-disabled`** in the store is a break-glass kill-switch: every memory adapter handler
  returns allow with no stdout/stderr, or equivalent allow-and-silent outcome, for any host operation
  (the whole recall pipeline is suppressed).

### §5.x — Path canonicalization differs on purpose

Adapters and engine canonicalize paths differently; do not unify them:

- **Adapters** canonicalize **lexically** with `realpath -sm` or an equivalent no-symlink-resolution
  normalizer (symlinks **not** resolved), because the
  store's infra files (`_grammar.md`, `_tags.md`, `_tag_links.md`) are *relative symlinks* into the
  lab (`synapse/memory/`); resolving them would break store-path gating.
- **Engine** resolves symlinks via `os.path.realpath` / `os.path.expanduser` (e.g. in
  `_classify_target`'s prefix-containment check that prevents `../` escape).

A target can be classified one way by an adapter and differently by the engine. This is intentional.

---

## §6 — Write path

Saving a memory derives triggers in context at write time. The write-context injector emits one
payload under `WRITE_CONTEXT_BUDGET = 9500`: trigger schema + worked example, grammar vocabulary (or
one-line-per-tag digest fallback), top-N dedup candidates, and placement guidance.

**The write-guard tiers, in order.** The guard runs `check-write` and applies these in sequence; all
but the noted boundaries fail open:

1. **Shape / evidence validation.** Frontmatter present and a valid `metadata.triggers` block.
2. **Static degenerate gate** (corpus-free by design). Denies a trigger set whose only evidence is
   degenerate, across **two arms**: (1) only generic/low-signal commands, (2) only broad paths
   (`is_broad_path()`, §3.x). Any narrowing/live lever rescues the set: a routable arg (`byArg` OR
   `bySynonym`), a routable synonym (`bySynonym`), or a specific path (§1, §7). The gate fails open
   when catalog vocabularies (`byArg` / `bySynonym`) are absent.
3. **Dedup backstop.** Similarity to existing memories is
   `0.6 × tag_overlap + 0.4 × bag-of-words-cosine` using stdlib `collections.Counter` only (§12).
   The advisory layer injects top-N candidates above `DEDUP_CANDIDATE_FLOOR = 0.2` (display only).
   The fail-closed layer denies a **new-file** write whose best score is
   `≥ DEDUP_BACKSTOP_THRESHOLD = 0.85`, naming the existing file. Existing-file consolidation always
   passes. `DEDUP_STOPWORDS` must include function words plus store-domain noise: `box`, `memory`,
   `memories`, `note`, `notes`, `lesson`, `lessons`.
4. **Collision tier** (corpus-aware). Reads the projection verdict (§7). BLOCK-degenerate hard-denies;
   GUIDE-broad is advisory; below the floor it is silent. It fires only on new files. Any projection
   fault fails open to the static gate's result.

**Placement enforcement** (graduated; from the §3 placement model). Guidance is always injected, but
the guard **denies only high-confidence misplacement** — a memory all of whose grammar-known tags
carry `placement: box` written to a non-box target — naming the correct box path. Ambiguous/mixed
placement (unknown tags, mixed or `project`/`either` hints) **fails open**.

**The single fail-closed write-guard boundary (§2.9).** A full-file write of a frontmatter-bearing
memory may be denied only for: invalid shape/evidence, static degeneracy when catalog vocab is
present, duplicate new-file backstop, BLOCK-degenerate collision, or high-confidence misplacement.
Everything outside that boundary fails open. Keep these fail-directions distinct:

| # | Surface | Direction | Why |
|---|---------|-----------|-----|
| a | read: `search()` function | **closed** (returns None) | calling rebuild on read would load bodies |
| a | read: recall adapter handler | **open** (allow / exit 0) | a host operation is never blocked |
| b | write: shape/evidence + static gate | **closed** | embed triggers at write time… |
| b | …static gate w/ catalog vocab absent | **open** | …but can't judge without the index |
| c | Collision tier (any projection fault) | **open** to static gate | projection is best-effort |
| d | Collision BLOCK-degenerate | **closed** | the one collision verdict that hard-denies |

**Partial edits fail open.** The guard cannot judge incomplete content. Host operations that provide
only a patch, diff, range edit, shell command, or other partial mutation always pass unless the
adapter can supply the complete proposed post-write file and explicitly classify the operation as a
full-file write. If a partial edit makes a memory unroutable, the post-operation rebuild's
`routabilityReport` (§4) flags it advisory-only.

---

## §7 — Collision projection

`project_triggers()` reports which existing memories a proposed trigger set would co-fire with, so
the write path (§6) can judge over-breadth.

Projection walks the same `_walk_index` as recall (§2.4), but ungated and unscored. Ungated means it
sees single-weak-tier co-fires; unscored means it reports breadth, not confidence. Do not
re-synthesize a host command-execution event through `extract_tokens`; event tokenization and generic
command drops misclassify proposed triggers.

**The projection's fields.**

- `collisions` — the co-firing memories.
- `per_trigger` — the per-axis (command/path/arg/synonym) contribution table. **Not a sum** — the
  axis-resolved breadth.
- **`distinct_count`** (= **co-fire breadth**, §1) — how many other memories the set matches.
- **`live_levers`** — the declared levers that would actually **route** the proposed memory at recall.
  This is the load-bearing field (below).

**Liveness = routability** *(supersedes the old co-fire-count model; Appendix A).* A lever is live iff
it would route the memory inside the single matcher's walk, independent of how many other memories it
touches:

- **arg** → live if in `byArg` **OR** `bySynonym` (a grammar-tag-name route is excluded as decorative).
- **path** → live if **specific** under `is_broad_path()` (§3.x); needs no catalog membership.
- **synonym** → live if in `bySynonym`.

Liveness applies `_norm` (strip + lowercase + `TAG_RE` filter) and exact membership against raw
catalog keys, matching `_walk_index`'s lookup. Unroutable forms (`--bare`, `-p`, mixed-case keys) are
not live.

**Verdict semantics** (a pure read of the projection; the live-lever and static-gate definitions are
unified, so the two tiers can never disagree):

- **PASS** — `distinct_count == 0` or `≤ collisionGuideFloor`.
- **BLOCK-degenerate** — co-fire breadth **strictly greater than** `collisionGuideFloor` **AND**
  `live_levers` is **empty**. (Breadth carried entirely by an axis with no declared
  narrowing — the degenerate case. Strictly `>`, not `≥`, is load-bearing.)
- **GUIDE-broad** — breadth above the floor but **a declared lever is live** (e.g. a deliberately
  broad host project-store path). Advisory note, **never** a hard block.

**Consolidation/update is exempt.** The collision tier fires only for new files (`target` None or
non-existent), like the dedup backstop. Rewriting an existing memory always passes.

**Fails open.** Any projection fault returns an empty projection (`_empty_projection()`, which carries
`live_levers`), and the write proceeds under the static gate only.

---

## §8 — Self-curation

The store curates itself from usage telemetry; there is no standing review ritual. Automated curation
demotes and flags only; it never deletes memory content (§2.8).

**Telemetry capture.** Every recall fire appends one JSONL record
`{ts, qid, mems:[{id,tag,type,val}], conf}` to `_recall_telemetry.jsonl` **after** the advisory
emission, fail-open (`|| true`) — a telemetry fault never blocks recall. A read-confirmation record
`{ts, id, signal:"read"}` is appended when a normalized read operation targets a store memory with a
**live (<15 min) dedup mark** — *the mark's presence IS the fire↔read correlation* (no timestamp
join). Fire-append is gated on at least one dedup mark having persisted, so unloggable reads produce
**zero-fire** (never demoted) rather than fires-without-possible-reads.

- **Rotation:** at `_TEL_MAX = 1 MB` (1048576 bytes) the file rotates to `_recall_telemetry.jsonl.1`
  (one generation, atomic `mv`; ~2 MB total).
- **Window read order (WR-04):** the `.1` generation is read **first**, then the live file.
- **Bad-ts symmetry (WR-05):** both fires **and** reads drop on an unparseable timestamp — keeping
  only one side would inflate read-rate.

**The maintenance pass.** `maintenance()` runs at the session-start lifecycle event only when
`_recall_telemetry.jsonl`
has grown **≥ 50 records** since the last pass (`_maintenance_state.json`). It runs under
`timeout 2 || true`, uses an `O_EXCL` lock, and reclaims stale locks by atomic rename-to-corpse
(`_MAINT_LOCK_STALE_SECS = 300`; never stat→unlink→create). It scores each memory over a rectangular
`telemetryWindowDays = 30` window:

- `read_rate ≥ promoteThreshold (0.4)` → clears `declineCount`.
- `read_rate ≤ demoteThreshold (0.05)` → increments `declineCount`.
- It **never** deletes/moves/rewrites content — only frontmatter `declineCount`, via
  `parse_frontmatter → generate_frontmatter → write_atomic`.

Read-rate is a conservative lower bound because the agent may act on inline advisory text without an
observable read operation; the low demote threshold absorbs that undercount.

**Two concurrency rules (the pass is not idempotent).**

- **WR-01 claim-before-mutate:** the pass must claim state (`_update_maintenance_state`) before
  applying any `declineCount` mutation. Per-file writes are atomic; the pass is not.
- **WR-02 recheck-under-lock:** the ≥50-record trigger count is re-verified **under the lock** to
  close the adapter's read-then-act race.

**The three floors** (all three are load-bearing; a reseed must enumerate exactly these):

1. **Zero-fire floor (D-43):** `fire_count == 0` → never demoted. This precedes rate computation.
2. **Minimum-evidence guard:** no mutation until **≥ 10 distinct session-days OR ≥ 30 days span**
   (`minEvidenceSessions = 10`, counting distinct session-days; `minEvidenceDays = 30`).
3. **Seat dual-gate:** a seat is proposed for demotion only when **both** a probe payload demonstrably
   surfaces it through the live recall adapter **AND** telemetry shows it fired
   (`seatPromoteMinFires = 5`).

**Seat governance.** `MEMORY.md` router-seat membership is machine-governed inside `seats()`. Because
the box-brain store is not git-tracked, proposed changes are emitted as a `PENDING-SEAT-CHANGES`
HTML-comment block prepended to `MEMORY.md`; removing the block accepts the change. Non-block content
stays byte-identical, and re-runs replace rather than stack the block. If always-on seats have no
`triggers:`, probes return `covered:false` and the engine proposes zero demotions.

---

## §9 — Performance contract

`bench_recall.sh` is regression-relative, not an absolute cliff. It compares current recall p95 to a
committed baseline:

```
ceiling = baseline + max(25%, 15 ms)
```

A committed integer file holds the accepted steady-state p95 (§10). Four verdicts:

- **PASS** — `p95 ≤ ceiling` and within the design budget.
- **WARN** — over the design budget but `≤ ceiling`; advisory, **exit 0** (does not block); prints how
  to accept drift.
- **REGRESSED** — `p95 > ceiling`; a genuine structural slowdown, **exit 1** (blocks).
- **NOBASELINE** — no baseline file; measure-only, exit 0.

Accepting corpus-growth drift is auditable: `--update-baseline` rewrites the baseline file as a
reviewable diff. The script is its own judge; the old MVR/GSD judge is retired (§12).

**The two "55 ms" — never conflate them.**

- The **live advisory design budget** (`BUDGET_MS = 55`): exceeding it produces **WARN / exit 0**.
- The **retired absolute cliff** (a hard `p95 ≤ 55 ms`): **dead.** It lives only in Appendix A as
  superseded rationale. Same number, opposite status.

---

## §10 — Tunables vs. invariants

The contract is shape (§2), not number. Config-tunable values live in `_memory_surface_config.json`
(`DEFAULT_CONFIG`); hardcoded constants require code changes.

| Tunable | Default | Home | Note |
|---------|---------|------|------|
| `tierWeights` | `{strong:10, medium:6, weak:3}` | config | type→tier map is hardcoded |
| surface gate | ≥1 strong OR ≥2 total | hardcoded rule-form | the *form* is a §2 invariant; only the counts here |
| score penalties | `-5×stale`, `-2×min(declineCount,3)` (cap 3) | hardcoded | sets confidence labeling |
| `confidenceHighThreshold` / `MediumThreshold` | 10 / 6 | config | |
| `DEDUP_BACKSTOP_THRESHOLD` | 0.85 | hardcoded const | new-file deny backstop; never loosen silently |
| `DEDUP_CANDIDATE_FLOOR` | 0.2 | hardcoded const | advisory display floor, **not** a block |
| dedup formula | `0.6×tag_overlap + 0.4×bow-cosine` | hardcoded | stdlib Counter only (= no-embeddings, §12) |
| `DEDUP_STOPWORDS` | ~40 words + store-domain noise | hardcoded | store-domain members are load-bearing |
| `WRITE_CONTEXT_BUDGET` | 9500 chars | hardcoded const | digest-fallback when over |
| `collisionGuideFloor` | 8 | config | the **only** corpus-breadth cutoff; same floor gates block + advisory; asserted, not derived |
| `promoteThreshold` / `demoteThreshold` | 0.4 / 0.05 | config | |
| `telemetryWindowDays` | 30 | config | rectangular/hard cutoff, no decay |
| `minEvidenceSessions` / `minEvidenceDays` | 10 / 30 | config | guard is **OR** of the two |
| `seatPromoteMinFires` | 5 | config | seat dual-gate |
| `dedupeTtlSeconds` | 900 (15 min) | config | mark presence = fire↔read correlation |
| `_MAINT_LOCK_STALE_SECS` | 300 | const | O_EXCL corpse reclaim |
| `_TEL_MAX` | 1 MB | const | telemetry rotation, one generation |
| `maintenance` trigger | ≥50 new records | const | re-checked under lock (WR-02) |
| bench `BUDGET_MS` | 55 (advisory WARN) | const | regression ceiling = `baseline + max(25%,15ms)` |
| recall p95 baseline | committed integer | tracked file | refreshed via `--update-baseline` |
| (secondary) | `obligationTtlSeconds 1800`, `maxResults 3`, `maxRequiredReads 2`, `maxDescriptionChars 220`, `maxBlockChars 4000` | config | |

**Direct-CLI inversion.** Directly invoked CLIs (for example, `scripts/lint.sh`) invert adapter
discipline: they are loud on success and fail closed on missing dependencies. Invariants §2.7 and
§2.10 apply to adapter handlers, not direct CLIs.

---

## §11 — Drift guardrail

Corpus-relative safety claims can drift: static degeneracy, `live_levers` liveness against
`byArg`/`bySynonym`, and `collisionGuideFloor`. Rebuild or session-start runs one fail-open
invariant-check where the catalog is already being touched. It emits one advisory line if violated;
it is a guardrail, not a gate, and does not tune constants per corpus:

- assert no existing trigger-bearing memory is a bare-degenerate-only set (would be denied by the
  static gate today), and
- assert no curated memory would be BLOCK-degenerate under the current verdict.

It never blocks; it only surfaces that a point-in-time assumption has drifted. This is also where the
§4 routing-vs-ranking partition would surface if a future field became both.

---

## §12 — Non-goals / out of scope

The reseed must not re-import:

- **No GSD / planning spine.** Do not reintroduce a per-edit workflow-enforcement layer. GSD was
  distilled to ADRs + capability specs + box-brain memory and archived at `gsd-archive-pre-removal`.
- **No SQLite / FTS5** on the routing path (§4).
- **No embeddings, no LLM on the read path** (§1/§5). Dedup similarity is stdlib `Counter` only.
- **No prompt-keyword routing** (§2.3) — it was implemented once and rolled back as noise.
- **No standing review ritual** (§8) — no Memory Roulette.
- **No bulk-LLM trigger derivation** over the corpus; legacy memories were made routable mechanically.
- **No second matcher** (§2.4) — recall and projection share `_walk_index`.
- **No per-corpus block cutoff** beyond the single `collisionGuideFloor` (§7) — nothing to drift.
- **No permission-policy writes** (§0 security constraint) — permission posture is outside the memory
  subsystem.

**Fenced-out apparatus, named but not specified:**

- The **host-runtime base harness** (non-memory lifecycle/policy handlers: config-drift-guard,
  handoff-index, lab-scope, system-fingerprint, syntax-check-touched, etc.) — a separate concern. The
  permission-policy guard and install manifest live here, not in the memory engine.
- **corpusforge** — the seeker/helper-duel corpus-generation apparatus described in synapse's
  CONTEXT.md — is an entirely separate lab apparatus and contributes no term or capability here.

---

## §13 — Reseed / bootstrap procedure  **[DESIGNED]**

> **Provenance:** designed, not distilled. The sources leave fresh-host bootstrap open; the contract
> below is normative for the reseed implementation.

**Goal.** On a clean host or clean-slate reset, the first session has a valid store and an
empty-but-correct catalog.

**Bootstrap CLI.** The reseed ships a directly invoked `bootstrap-store` command. The wrapper path may
vary, but the contract is:

```
bootstrap-store --store <store-dir> --grammar <lab/_grammar.md> [--import-legacy]
```

It is loud like every direct CLI (§10), fails closed on missing dependencies, and is idempotent:
repeated runs may atomically rewrite `_memory_catalog.json`, but must not duplicate content, stack
pending blocks, rewrite memory bodies, remove unmanaged store files, or touch host permission policy.
With no input changes, a second run leaves the same observable store state.

**Empty grammar seed.** A fresh host with no `_grammar.md` starts with this literal schema-valid empty
grammar in the lab, relative-symlinked into the store:

```markdown
# Memory Trigger Grammar

<!-- memory-grammar-version: 1 -->
```

`validate_grammar` accepts an empty grammar with zero tag entries. That is the only zero-evidence
case that passes: a present tag entry with no behavioral evidence still fails with exit 2 (§2.3).

**Cold-start sequence and resulting state:**

1. **Seed the grammar.** Ensure the lab grammar file exists, then create or repair the store's
   `_grammar.md` as a **relative symlink** to it. This is the only file the install manifest manages
   inside the store. Store taxonomy/data files like `_tags.md` and `_tag_links.md` are unmanaged and
   left in place.
2. **Seed/confirm the store.** Ensure the store directory exists. If `MEMORY.md` is absent, create a
   minimal router file with no seats:

   ```markdown
   # Memory Router
   ```

   Bootstrap never overwrites existing `MEMORY.md`, seat memories, telemetry files, taxonomy files,
   or ordinary memories.
3. **First rebuild.** Run `rebuild()` once and atomically write `_memory_catalog.json`. On an empty
   store this yields a valid catalog with empty `triggerIndex.byCommand`, `byPath`, `byArg`,
   `bySynonym`, and `byMemoryId` tables plus `routabilityReport: 0 unroutable`.
4. **Mechanical legacy routability (only with `--import-legacy`).** Grammar-covered memories route
   via tag-level evidence. Any memory with no grammar or frontmatter route gets engine-side fallback
   derivation (`derive_fallback_triggers` extracts backtick-quoted command/path tokens from the body)
   written as catalog-side `byMemoryId` entries with `source = memory-derived` — **never** into
   frontmatter. The legacy-import cutover gate is a literal `routabilityReport: 0 unroutable`; if
   fallback derivation cannot route a memory, bootstrap reports the unroutable ids and exits non-zero.
5. **Verify fail-open.** Run the safety checks before reporting success.

**Fail-open checks:**

- With `.surface-disabled` present, every memory adapter handler returns allow with empty
  stdout/stderr, or equivalent allow-and-silent outcome.
- With `_memory_catalog.json` temporarily absent, the recall adapter returns allow, surfaces nothing,
  and does **not** rebuild the catalog on the read path.
- Restoring or rebuilding the catalog returns the store to the expected post-bootstrap state.

After a successful clean bootstrap, `<store-dir>/`, `<store-dir>/MEMORY.md`,
`<store-dir>/_grammar.md` (relative symlink), and valid `<store-dir>/_memory_catalog.json` with the
five `triggerIndex` tables exist. No host permission-policy file or block has been written, and no
unmanaged taxonomy/data file has been removed.

**`byMemoryId` fallback lifecycle.** Memory-derived routes are build artifacts, not store content:

- `rebuild()` recomputes them only for memories that have no grammar or frontmatter route.
- Once a memory gains a natural route, the next rebuild drops its `source = memory-derived`
  `byMemoryId` entries with no tombstone or migration.
- Projection and recall may use memory-derived entries while they exist, but write guards must never
  require them to be present; any fallback derivation fault fails open outside `--import-legacy`
  cutover.

**Install-manifest boundary.** The host install set manages only `_grammar.md`; store taxonomy/data
files are unmanaged and left in place. Stores are data, not install-managed code.

---

## §14 — Conformance matrix

Conformance means the implementation supplies an automated check for every row below. A check may be
a unit test, integration test, fixture script, or benchmark, as long as it is included in the default
verification path documented by the implementation. Adapter-handler rows assert allow/deny outcome
and user-visible stdout/stderr or equivalent diagnostics; engine rows assert function return values
and store/catalog state.

| Contract area | Required check | Source |
|---------------|----------------|--------|
| Read path is index-only | `search()` never calls `rebuild()` and never opens memory bodies; a missing/corrupt catalog returns `None`. | §2.1, §5 |
| Adapter fail-open layering | Missing engine/store/catalog and unexpected recall faults return allow; taxonomy/config errors hard-fail only when actionable. | §2.7, §5, §6 |
| Quiet adapter success | Passing adapter invocations emit no stdout/stderr or equivalent context-visible output. Direct CLIs remain loud and fail closed on missing dependencies. | §2.10, §10 |
| Store source / catalog artifact | `rebuild()` fully recreates `_memory_catalog.json`; direct catalog edits are overwritten; malformed-but-parseable catalogs load as `None`. | §2.2, §4 |
| Observable routing only | Prompt text alone never routes; command/path/arg/synonym evidence does; a grammar tag with no behavioral evidence fails validation. | §2.3, §3, §5 |
| Single matcher | Recall and projection both exercise `_walk_index`; a fixture trigger set yields the same ungated hit set through both call paths. | §2.4, §7 |
| Surface gate and scoring | No evidence is silent; one weak tuple is silent; one strong tuple fires; two tuples fire; generic verbs do not count as strong; stale/decline penalties affect confidence labels. | §5, §10 |
| Diagnosable fires | Every surfaced memory includes `{tag} <- {trigger_type}:{matched_value}` evidence. | §2.6, §5 |
| Path specificity | Every broad/specific example in §3.x is classified exactly as listed; §6 and §7 call the same classifier. | §3.x, §6, §7 |
| Path canonicalization divergence | Adapter lexical canonicalization preserves symlinked store infra paths; engine realpath containment blocks `../` escape. | §5.x |
| Write-guard boundary | Only full-file writes of frontmatter-bearing memories can fail closed; partial edits, frontmatter-less writes, and projection faults pass. | §2.9, §6 |
| Write deny reasons | Full-file write fixtures deny exactly for invalid shape/evidence, static degeneracy with catalog vocab, duplicate new-file backstop, BLOCK-degenerate collision, and high-confidence misplacement. | §6 |
| Dedup semantics | Similarity uses the specified weighted formula and stopword set; the backstop denies new files only; existing-file consolidation always passes. | §6, §10 |
| Collision projection | `distinct_count`, `per_trigger`, and `live_levers` are reported; `BLOCK-degenerate` uses strict `>` floor plus empty `live_levers`; live levers produce GUIDE-broad above the floor. | §7 |
| Placement enforcement | Infra files are exempt before placement checks; all-known-`box` tags deny non-box targets; unknown/mixed/`project`/`either` placement passes. | §3, §6 |
| Telemetry capture | Fire records append after advisory emission and fail open; read records require a live dedup mark; rotation reads `.1` before live; bad timestamps drop symmetrically. | §8 |
| Maintenance concurrency | The pass rechecks the ≥50-record trigger under lock, claims state before mutation, and handles stale locks by atomic rename-to-corpse. | §8 |
| Curation floors | Zero-fire memories never demote; minimum evidence is `≥10` distinct session-days OR `≥30` days span; seat demotion requires probe coverage and telemetry fires. | §8, §10 |
| Seat governance | `PENDING-SEAT-CHANGES` blocks replace rather than stack; non-block router content stays byte-identical; memory content is never rewritten or deleted by curation. | §8 |
| Performance gate | `bench_recall.sh` produces PASS/WARN/REGRESSED/NOBASELINE exactly by the baseline-relative formula; only REGRESSED exits non-zero. | §9 |
| Drift guardrail | Rebuild or session-start emits one advisory line when existing trigger-bearing memories violate the static gate or current collision verdict; the guardrail never blocks. | §11 |
| Security boundary | Engine and bootstrap paths never write host permission policy. | §0, §12, §13 |
| Bootstrap | `bootstrap-store` is idempotent, seeds the literal empty grammar, creates expected files, verifies `.surface-disabled`, verifies missing-catalog recall fail-open without rebuild, and applies the `byMemoryId` lifecycle. | §13 |

---

## Appendix A — Superseded rationale (kept for the *why*, not operative)

- **Collision model: co-fire count → live-lever routability.** The earlier model operationalized
  "dead lever" as `sum(per_trigger) == 0` (co-fire count). This was a **signal inversion**:
  `per_trigger == 0` is ambiguous — it means **both** a decorative lever that routes nothing (must
  block) **and** a perfectly-discriminating unique lever that co-fires with nobody (must *pass* — the
  best possible narrowing). Measured safe at ~9 memories, it **false-denied curated memories at 165**
  (the milestone's #1-rule violation: a legitimate memory denied at the sole fail-closed boundary).
  §7's `live_levers` (routability) replaces it. The thesis — *read a per-component verdict from the
  projection, not a scalar sum; block the degenerate, guide the weak, floor the block* — survives;
  only the operationalization was wrong.
- **Performance: absolute 55 ms cliff → regression-relative gate.** A hard `p95 ≤ 55 ms` (itself
  already a recalibration up from a stale 50 ms) drifted permanently red on corpus growth and so
  stopped meaning "regressed." §9's regression-relative gate replaces it; the 55 ms survives only as
  the advisory WARN budget.

## Appendix B — Deferred / historical items

- **One-time live-engine cutover recipe** (synapse's ADR-0016: dual flag-gate → atomic single-commit
  flip → zero config residue) is *historical migration*, not steady-state behavior; recorded here, not
  in the operative spec. Its `.surface-disabled` kill-switch lives in §5 independently.
