# Synapse Memory — Core Spec

> **Status:** reseed draft · **Scope:** the tag-routed memory subsystem only — recall,
> write-guard, self-curation, catalog, collision-projection. The generic Claude-Code
> base-harness and the corpusforge corpus-generation apparatus are out of scope and are
> named (not specified) in §12.
>
> **What this document is.** One self-contained spec from which the memory subsystem can be
> cleanly *reseeded*. It is the primary artifact of the **bolt** lab (the reseed). It is distilled
> from sources that live in the sibling **synapse** lab and is referenced *by path*, never vendored:
> the 19 ADRs (`../synapse/docs/adr/`, the *why*), the seed inventory
> (`../synapse/openspec/specs/_PENDING-FROM-GSD.md`, the current-state capability list), the two
> promoted capability specs (`../synapse/openspec/specs/write-guard/`, `…/collision-projection/`),
> and the live engine (`../synapse/lib/memory_surface.py`, the tiebreaker on any conflict). It
> links *out* to those for deep rationale rather than restating them.
>
> See [`CONTEXT.md`](./CONTEXT.md) for the one-screen domain glossary; this document is the full spec.
>
> **Provenance convention.** Every section is *distilled* from working behavior unless it is
> explicitly marked **[DESIGNED]** — meaning it is new design the sources left open (only §13).

---

## §0 — Purpose & core value

**Purpose.** Surface the right durable memory at the right moment, automatically, from what a
session is *actually doing* — and otherwise stay silent and out of the way.

**Core value** (verbatim intent): *the right memory surfaces at the right moment with zero human
curation — and the whole system stays legible and maximum-punch-per-pound while doing it.*

Three things are load-bearing in that sentence, and the rest of this spec is downstream of them:

- **Right memory, right moment** — recall is driven by observable session behavior, calibrated so
  a wrong fire (which taxes attention irreversibly) is far costlier than a miss (which is cheaply
  backstopped). See §5.
- **Zero human curation** — the store curates itself from usage telemetry; there is no standing
  human review loop. See §8.
- **Legible & maximum-punch-per-pound** — see the directive below.

### The punch-per-pound directive

> **Maximum punch per pound.** The system delivers its effect efficiently regardless of where its
> weight sits: the per-tool-call read path must be near-free, and heavy computation moves to write
> time / session start / offline rebuilds.

This is a **directive**, not a mechanism. It is the *why* behind the cost shape of the whole system
(write-time intelligence, read-time lookup — §2). One discovered *means* of honoring it on the hook
layer is recorded as a **finding**, not a law:

> **Finding (not a requirement):** Hooks are cheapest as pure POSIX-ish shell + jq, with no Python
> interpreter spawned per tool call. Python is spawned by exactly one hook (recall), and its startup
> cost is amortized under the read-path performance budget (§9). A future maintainer may revisit the
> *means* (e.g. a different cheap substrate) without violating the directive — the directive is the
> contract; POSIX+jq is the current best realization of it.

This distinction — directive as contract, implementation as evidence — is the discipline this reseed
exists to restore. Where a number or a mechanism appears below, it is a tunable or a finding under a
rule, never the rule itself (§10).

### Constraints (standing, beyond the read/write/curation behavior)

- **Stdlib-only engine.** `lib/memory_surface.py` uses Python 3 standard library only — no PyPI
  dependencies, ever. Adding a dep means every hook invocation risks an `ImportError` on box
  reconfiguration. This constraint is absolute.
- **Data survives, metadata is expendable.** The memory files' *content* must survive; all derived
  routing/ranking metadata is regenerable (§4) and carries no migration obligation.
- **Security posture — no permissions writes.** The memory engine never writes to a settings
  `permissions` block — not `allow`/`deny`, not `defaultMode`. Permission posture is the operator's
  alone. *(This is a security constraint, deliberately **not** one of the §2 hook-discipline rules;
  runtime/install enforcement of it lives in the base-harness, which is out of scope — §12.)*

---

## §1 — Glossary (ubiquitous language)

Only the terms load-bearing for the memory subsystem. Corpusforge vocabulary is excluded by scope.
This table is mirrored, condensed, in [`CONTEXT.md`](./CONTEXT.md) as the repo's domain glossary.

| Term | Meaning |
|------|---------|
| **store** | The directory of memory files plus its infra files (`_grammar.md`, `MEMORY.md`, …). The *source of truth*. |
| **memory** | One markdown file: YAML frontmatter (`metadata:` incl. a `triggers:` block) + a body. |
| **grammar** | `_grammar.md` — the single trigger grammar. Each tag declares a gloss, a placement hint (`box`\|`project`\|`either`), and evidence fields. *A tag **is** its evidence patterns.* |
| **trigger** | A behavioral evidence pattern that routes a memory: a command, path, arg token, or synonym. |
| **catalog** | `_memory_catalog.json` — the compiled, rebuildable build artifact holding the routing index. Never the source of truth (§4). |
| **triggerIndex** | The inverted-index section inside the catalog: `byCommand` / `byPath` / `byArg` / `bySynonym` / `byMemoryId`. |
| **recall / fire** | A per-tool-call lookup surfacing zero or more memories. A surfaced memory has *fired*. |
| **tier** | The strength class of a matched trigger: command/path = **strong**, arg = **medium**, synonym = **weak**. |
| **surface gate** | The precision threshold a candidate set must clear to fire (§5). Below it: silence. |
| **co-fire breadth** (a.k.a. `distinct_count`) | How many *other* memories a proposed trigger set would also match — the collision quantity (§7). One quantity, two names; this spec uses **co-fire breadth** in prose and notes `distinct_count` as the projection field. |
| **narrowing / live lever** | An author-controlled trigger that actually *routes* the memory: a routable arg (present in `byArg` **or** `bySynonym`), a routable synonym (in `bySynonym`), or a *specific* (non-broad-glob) path. The **same** definition drives the write-guard static gate (§6) and the collision verdict (§7) — defined once here, referenced from both. |
| **verdict** | The write-time collision outcome: PASS / GUIDE-broad (advisory) / BLOCK-degenerate (§7). |
| **seat** | A memory holding a slot in `MEMORY.md`, the always-loaded router floor. Seat membership is machine-governed (§8). |

---

## §2 — Load-bearing invariants (the frozen shapes)

These are the rule *forms* the system rests on. They are frozen: changing one changes what the
system fundamentally *is*. Every magnitude they reference (weights, thresholds, windows) is a
**tunable**, not part of the invariant — see §10. The contract is shape, never number.

**Architecture**

1. **Write-time intelligence, read-time lookup.** All derivation, linking, and ranking is spent once
   — by a full model — at memory-write time, session start, or an offline rebuild. The per-tool-call
   read path is a pure precomputed-index lookup: no LLM, no embeddings, no rebuild, and it never loads
   memory bodies. *(This is the operational form of the punch-per-pound directive, §0.)*
2. **Store is source, index is binary.** The store is the source of truth; the catalog/triggerIndex
   is a disposable build artifact, rebuilt from the store at will, never hand-edited, never migrated.
   A format change is a *rebuild*, not a migration.
3. **Route on observable behavior, never on prompt text.** Recall keys on what the session does
   (commands run, paths touched, arg tokens) — never on the prompt. A tag with no behavioral evidence
   cannot exist (`validate_grammar` enforces this: a synonyms-only tag fails, exit 2).
4. **One grep-provable matcher.** Recall and write-time collision projection walk the *same* index
   path (`_walk_index`). Any change to matching logic goes through that one function, so projection
   can never diverge from recall. No second matcher.

**Calibration**

5. **Precision over recall.** Silence is the default. Every fire must be affirmatively justified
   against behavioral evidence; the confidence threshold is conservative. A miss is cheap (the
   session-start floor and explicit reads backstop it); a false fire is an irreversible cross-session
   attention/trust tax.
6. **Diagnosable fires.** Every surfaced memory cites its firing evidence inline as
   `{tag} <- {trigger_type}:{matched_value}` — a wrong fire is diagnosable from the recall block alone.

**Safety / failure**

7. **Fail open.** A missing engine, unreadable store, missing/corrupt catalog, or unexpected error
   never blocks a tool call — the hook process exits 0. Only a genuinely actionable taxonomy/config
   error exits 2. *(Layered nuance, expanded in §5/§6: the read-path *function* may return "nothing"
   as a closed result while the hook *process* still exits 0 — return-value vs process-exit are
   different layers. This single invariant is also the binding hook-discipline rule; it is not
   restated separately below.)*
8. **Never delete autonomously.** Automated curation demotes and flags but never deletes a memory.
   Deletion is the one retained human-in-the-loop step. Memory *content* is never rewritten by the
   curation pass (§8).
9. **One fail-closed boundary.** On an otherwise advisory system, exactly one memory write is denied:
   a full `Write` of a frontmatter-bearing memory with no valid triggers (§6). Everything else —
   `Edit`/`MultiEdit`, frontmatter-less content, any projection fault — fails open.

**Hook discipline** *(one operator-required rule beyond fail-open, which is invariant 7)*

10. **Quiet on success.** A hook exits 0 with no output on the pass path. `stderr` is reserved for
    actionable failure only. No status/progress lines ever feed Claude's context.

> **Not invariants here, by deliberate correction:** "hooks must be cheap / POSIX+jq" is a *finding*
> under the punch-per-pound directive (§0), not a rule. "Never writes permissions" is a *security
> constraint* (§0), not a hook-discipline rule. Both were previously over-baked into a single
> "iron rules" list; this spec un-bakes them — the directive/constraint is the contract, the
> mechanism is evidence.

**Engine substrate**

11. **Stdlib-only, atomic mutation.** The engine is stdlib-only (§0). All store mutations go through
    an atomic write primitive (write-temp-then-rename), so a crash never leaves a half-written memory
    or catalog.

---

## §3 — Data model

**Memory file.** A memory is one markdown file: a YAML frontmatter block (`---…---`) followed by a
free-text body. Routing-relevant metadata lives under a `metadata:` key in frontmatter.

**The `triggers:` block.** The author's behavioral evidence lives at `metadata.triggers` — never at
the top level. It holds four string arrays:

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

**Placement model.** Every memory belongs to a store, classified by subject:

- Stores recognized: the **box-brain** store, any **Claude project** store
  (`*/.claude/projects/*/memory/*.md`), and **repo** `memory/` dirs (`*/memory/*.md`).
- **Infra files are exempt first** — underscore-prefixed files, `MEMORY.md`, `_grammar.md` are
  checked and excluded *before* any placement logic.
- `_classify_target` returns `box | project-store | repo-memory | other` via realpath-normalized
  prefix comparison (which also prevents `../` escape — see §5.x). The grammar's `box|project|either`
  hint feeds this. Enforcement of placement is graduated and lives in §6.

---

## §4 — The catalog (rebuildable build artifact)

**One artifact.** `_memory_catalog.json` is the compiled routing index. It is **not** a separate
`_routing_index.json`, and it is **not** SQLite/FTS5. (SQLite was rejected on the read path: its
cold-open+query is ~50 µs, but Python startup ~19–30 ms dominates either way, so SQLite saves nothing
while costing connection/lock/migration burden and shell-inspectability. The catalog stays
jq-queryable.) *(Store-is-source/index-is-binary, §2.2.)*

**The `triggerIndex` section.** Inside the catalog, the inverted index is five tables:
`byCommand` / `byPath` / `byArg` / `bySynonym` / `byMemoryId`. Grammar tag-level evidence and
per-memory `triggers:` frontmatter are folded into the shared `byCommand`/`byPath`/`byArg`/`bySynonym`
buckets; `byMemoryId` carries direct/derived routing for a specific memory. `compile_trigger_index()`
builds it.

**`rebuild()` rules.** `rebuild()` reconstructs the entire catalog from store contents in one
command. It is **never hand-edited**. Consistency is *structural*:

- Every engine mutator (`add_tag`/`link`/`unlink`, via `_mutate_then_validate`) calls `rebuild()`
  **before returning**.
- The PostToolUse catalog-refresh rebuilds on **any** store `.md` write (grammar/taxonomy writes
  arriving through the lab-side symlink are resolved via `readlink -f` — see §5.x).
- **Ranking-metadata writes do NOT rebuild.** `lastReviewed` / `declineCount` are *ranking* inputs,
  not *routing* inputs; rebuilding on them would cost ~3–5 ms per write for zero routing benefit.
  This is the one apparent-inconsistency the design deliberately leaves unplugged — the
  routing-input-vs-ranking-input distinction is what makes it correct, not a gap. *(If a future field
  is ever both a ranking and routing input, this partition breaks; the §11 drift guardrail is where
  that would surface.)*

**Reports & staleness.**

- `rebuild()` emits a **`routabilityReport`** — the count and ids of any unroutable memories — on
  stderr and in catalog metadata. The cutover gate was a literal `routabilityReport: 0 unroutable`
  from a real rebuild, not an assertion.
- `fingerprint()` hashes `_grammar.md` into the catalog (`sourceFingerprint`) so staleness is
  mechanically detectable.

**Single reader.** `_load_catalog` is the one function that loads the catalog. It **rejects a
malformed-but-parseable catalog** (e.g. `memories` not a list of dicts) by returning `None`, so every
consumer fails open/closed *cleanly* rather than raising an uncaught `TypeError`.

---

## §5 — Read path / recall

**What it does.** On each tool call, recall extracts behavioral evidence from `tool_input` — command
basenames, canonicalized paths, arg tokens — and looks them up in the precomputed `triggerIndex`.
No LLM, no embeddings. A single matcher, `search()`, walks both levels (grammar-tag evidence and
per-memory triggers) over the catalog's `triggerIndex`/`recallVocab` only; post-cutover it **never**
reads `parse_tags_md`/`parse_tag_links` (those are write-path-only).

**Scoring & gate** *(magnitudes are tunables, §10; the rule forms are frozen, §2):*

- `TIER_WEIGHTS = {strong: 10, medium: 6, weak: 3}` (config-overridable via `tierWeights`). The
  type→tier map (command/path/unit/tag = strong, arg = medium, synonym = weak) is **hardcoded**.
- **Surface gate:** a candidate set fires iff it has **≥ 1 strong-tier tuple OR ≥ 2 tuples total**;
  otherwise **silence** (`_meets_min_candidate`). No-evidence tool calls emit nothing.
- **Score penalties (hardcoded, not config):** `-5 × stale` (one-shot when `lastReviewed` is stale)
  and `-2 × min(declineCount, 3)` (decline cap = 3, so max −6). These determine whether a high-tier
  match clears `confidenceHighThreshold = 10` — omitting them silently changes confidence labeling.
- Default false-fire suppressors: a `GENERIC_VERBS` stop-list (generic verbs like restart/install/
  check don't count as strong tokens) and a per-memory-id dedup window (`dedupeTtlSeconds = 900`,
  i.e. 15 min).

**Diagnosable fires (§2.6).** Each surfaced memory cites firing evidence inline as
`{tag} <- {trigger_type}:{matched_value}` (e.g. `nvidia <- command:nvidia-smi`).

**The HARD read-path invariant.** `search()` **must never rebuild and never load memory bodies.**
A missing/corrupt catalog makes `search()` return `None` (surfaces nothing) — **not** because recall
is non-advisory, but because calling `rebuild()` inside `search()` would read frontmatter/bodies on a
read-path call, violating the bodies-never-loaded rule. This is the single rule most likely to be
broken by a future "just rebuild if stale" change — it is forbidden.

**Fail-open layering (one of four fail-directions — see §6 for the full set).** Two different layers:

- The `search()` **function** returns `None` (a *closed* result: nothing surfaces) on a
  missing/corrupt catalog.
- The recall **hook process** still **exits 0** — the tool call is never blocked. With
  `_memory_catalog.json` missing the hook exits 0 and never rc 2.
- **`.surface-disabled`** in the store is a break-glass kill-switch: every memory hook exits 0 with
  empty stdout/stderr for any tool call (the whole recall pipeline is suppressed).

### §5.x — Path canonicalization is deliberately divergent — DO NOT UNIFY

Hooks and engine canonicalize paths **differently, on purpose**, and any edit touching store-path
classification must preserve the divergence:

- **Hooks** canonicalize **lexically** with `realpath -sm` (symlinks **not** resolved), because the
  store's infra files (`_grammar.md`, `_tags.md`, `_tag_links.md`) are *relative symlinks* into the
  lab (`synapse/memory/`); resolving them would break store-path gating.
- **Engine** resolves symlinks via `os.path.realpath` / `os.path.expanduser` (e.g. in
  `_classify_target`'s prefix-containment check that prevents `../` escape).

A target can therefore be classified one way by a hook and differently by the engine — **this is
intentional.** This is exactly the kind of non-obvious rule a well-meaning "simplification" would
collapse; it must not be unified.

---

## §6 — Write path

**In-context trigger derivation.** Saving a memory derives its triggers *in context at write time*.
The write-context injector composes **one** payload under a `WRITE_CONTEXT_BUDGET = 9500`-char budget:
the trigger-spec schema + a worked example, the grammar vocabulary (or a one-line-per-tag **digest
fallback** when over budget), the top-N dedup candidates, and placement guidance.

**The write-guard tiers, in order.** The guard runs `check-write` and applies these in sequence; all
but the noted boundaries fail open:

1. **Shape / evidence validation.** Frontmatter present and a valid `triggers:` block. → feeds the
   single fail-closed boundary below.
2. **Static degenerate gate** (corpus-free by design). Denies a trigger set whose only evidence is
   degenerate, across **two arms**: (1) only generic/low-signal commands, (2) only broad-glob paths
   (`**`, `~/**`). **Any narrowing/live lever rescues the set** — the gate accepts a routable arg
   (`byArg` OR `bySynonym`), a routable synonym (`bySynonym`), or a specific non-broad path (the
   live-lever definition, §1, defined once and shared with §7). The gate **fails open** when catalog
   vocabularies (`byArg`/`bySynonym`) are absent.
3. **Dedup backstop.** Similarity to existing memories is
   `score = 0.6 × tag_overlap + 0.4 × bag-of-words-cosine` (stdlib `collections.Counter` only — no
   sklearn/numpy; this *is* the no-embeddings non-goal in operational form, §12). Two layers:
   - *Advisory (Layer 1):* write-context injects the top-N most-similar memories
     (`DEDUP_CANDIDATE_FLOOR = 0.2`, a display floor, **not** a block) so the model consolidates.
   - *Backstop (Layer 2, fail-closed):* a **new-file** write whose best score
     `≥ DEDUP_BACKSTOP_THRESHOLD = 0.85` is denied, naming the existing file. **Fires only on new
     files** — writing into an existing file (consolidation) is always allowed.
   - **Stopword nuance:** `DEDUP_STOPWORDS` includes ~40 function words **plus store-domain noise**
     (`box`, `memory`, `memories`, `note`, `notes`, `lesson`, `lessons`). These are store-specific,
     not generic NLP stopwords; omitting them re-introduces false-consolidation denies (domain noise
     pushing distinct memories past 0.85). A reseed must carry this set.
4. **Collision tier** (corpus-aware). Reads the projection verdict (§7). BLOCK-degenerate hard-denies;
   GUIDE-broad is advisory; below the floor it is silent. **Fires only on new files** (same carve-out
   as the backstop). Any projection fault → fail open to the static gate's result.

**Placement enforcement** (graduated; from the §3 placement model). Guidance is always injected, but
the guard **denies only high-confidence misplacement** — a memory all of whose grammar-known tags
carry `placement: box` written to a non-box target — naming the correct box path. Ambiguous/mixed
placement (unknown tags, mixed or `project`/`either` hints) **fails open**.

**The single fail-closed boundary (§2.9), and the full fail-direction set.** On this otherwise
advisory system, the **closed** surfaces are exactly: shape/evidence + the static gate (when catalog
vocab is present) + a BLOCK-degenerate collision verdict + the new-file dedup backstop + the
high-confidence-misplacement deny. **Everything else fails open**, and there are four distinct
fail-directions a reseed must keep straight:

| # | Surface | Direction | Why |
|---|---------|-----------|-----|
| a | Read: `search()` function | **closed** (returns None) | calling rebuild on read would load bodies |
| a | Read: recall hook *process* | **open** (exit 0) | a tool call is never blocked |
| b | Write: shape/evidence + static gate | **closed** | embed triggers at write time… |
| b | …static gate w/ catalog vocab absent | **open** | …but can't judge without the index |
| c | Collision tier (any projection fault) | **open** to static gate | projection is best-effort |
| d | Collision BLOCK-degenerate | **closed** | the one collision verdict that hard-denies |

**`Edit`/`MultiEdit` fail open + the advisory routability flag.** The guard cannot reconstruct full
file content from a partial edit, so `Edit`/`MultiEdit` (and any frontmatter-less content) **always
pass** — preserving the fail-open invariant. The leak this opens (a memory mutated via `Edit` into a
no-valid-triggers state, never caught at write time) is made **observable, not enforced**: the
PostToolUse rebuild's `routabilityReport` (§4) flags any memory that ends up untriggered as an
advisory line. This is a guardrail, not a new fail-closed boundary.

---

## §7 — Collision projection

**Purpose.** `project_triggers()` reports which existing memories a *proposed* trigger set would
co-fire with, so the write path (§6) can judge whether a new memory is over-broad. It is the
write-time quality signal.

**Reuse the one matcher (§2.4).** Projection walks the **same** `_walk_index` as recall — extracted
into one shared helper that both `search()` and `project_triggers()` call. The difference is only on
two axes: projection calls it **ungated** (no surface gate — so it sees even single-weak-tier
co-fires, exactly the over-broad cases it exists to catch) and **unscored**. Any change to matching
logic must go through `_walk_index` to keep recall and projection byte-consistent. (Re-synthesizing a
Bash event through `extract_tokens` was rejected — its tokenization/`GENERIC_BASH` drops would
misclassify proposed triggers.)

**The projection's fields.**

- `collisions` — the co-firing memories.
- `per_trigger` — the per-axis (command/path/arg/synonym) contribution table. **Not a sum** — the
  axis-resolved breadth.
- **`distinct_count`** (= **co-fire breadth**, §1) — how many other memories the set matches.
- **`live_levers`** — the author levers that would actually **route** the proposed memory at recall.
  This is the load-bearing field (below).

**Liveness = routability** *(the canonical model — supersedes the earlier co-fire-count model; see
Appendix A).* A lever is "live" iff it would route the memory, computed inside the single matcher's
walk, independent of how many *other* memories it touches:

- **arg** → live if in `byArg` **OR** `bySynonym` (a grammar-tag-name route is excluded as decorative).
- **path** → live if **specific** (not a broad glob); needs no catalog membership.
- **synonym** → live if in `bySynonym`.

Liveness applies the matcher's own `_norm` (strip + lowercase + `TAG_RE` filter) to the proposed
lever and tests **exact** membership against raw catalog keys — exactly `_walk_index`'s
`by_arg.get(_norm(arg))` lookup — so unroutable forms (`--bare`, `-p`, mixed-case keys) are correctly
**not** live and never over-credited. The verdict stays in lockstep with the matcher.

**Verdict semantics** (a pure read of the projection; the live-lever and static-gate definitions are
unified, so the two tiers can never disagree):

- **PASS** — `distinct_count == 0`, or `≤ collisionGuideFloor`.
- **BLOCK-degenerate** — co-fire breadth **strictly greater than** `collisionGuideFloor` **AND**
  `live_levers` is **empty**. (Breadth carried entirely by an axis with no author-controlled
  narrowing — the degenerate case. Strictly `>`, not `≥`, is load-bearing.)
- **GUIDE-broad** — breadth above the floor but **an author-controlled lever is live** (e.g. a
  deliberately broad `~/.claude/...` path). Advisory note, **never** a hard block.

**Consolidation/update is exempt.** The collision tier fires only for **new** files (`target` None or
non-existent), like the dedup backstop. Re-writing an already-curated memory (append a finding, fix a
typo) is always allowed — this is what keeps the verdict from false-denying curated work.

**Fails open.** Any projection fault returns an empty projection (`_empty_projection()`, which carries
`live_levers`), and the write proceeds under the static gate only.

---

## §8 — Self-curation

**Zero human curation (§2 core value).** The store curates itself from usage telemetry; there is no
standing human-review ritual. Curation is garbage collection — if store health needed a recurring
human game, that would mean write-time capture was insufficient, and the fix belongs at write time.
(The former "Memory Roulette" human-review game was retired — but only *after* a shadow validation
proved the automated pass wouldn't demote a human-kept memory.)

**Telemetry capture.** Every recall fire appends one JSONL record
`{ts, qid, mems:[{id,tag,type,val}], conf}` to `_recall_telemetry.jsonl` **after** the advisory
emission, fail-open (`|| true`) — a telemetry fault never blocks recall. A read-confirmation record
`{ts, id, signal:"read"}` is appended when a `Read` targets a store memory with a **live (<15 min)
dedup mark** — *the mark's presence IS the fire↔read correlation* (no timestamp join). Fire-append is
gated on at least one dedup mark having persisted, so unloggable reads produce **zero-fire** (never
demoted) rather than fires-without-possible-reads.

- **Rotation:** at `_TEL_MAX = 1 MB` (1048576 bytes) the file rotates to `_recall_telemetry.jsonl.1`
  (one generation, atomic `mv`; ~2 MB total).
- **Window read order (WR-04):** the `.1` generation is read **first**, then the live file.
- **Bad-ts symmetry (WR-05):** both fires **and** reads drop on an unparseable timestamp — keeping
  only one side would inflate read-rate.

**The maintenance pass.** `maintenance()` runs at SessionStart only when `_recall_telemetry.jsonl`
has grown **≥ 50 records** since the last pass (tracked in `_maintenance_state.json`), under
`timeout 2 || true` so it never blocks session start, serialized on an `O_EXCL` lock with
atomic-rename stale-reclaim (`_MAINT_LOCK_STALE_SECS = 300`; a corpse is reclaimed by atomic
rename-to-corpse, **not** stat→unlink→create). It scores each memory over a **rectangular**
`telemetryWindowDays = 30` window (records inside count equally, older count zero — chosen over
exponential decay so the window stays jq-auditable):

- `read_rate ≥ promoteThreshold (0.4)` → clears `declineCount`.
- `read_rate ≤ demoteThreshold (0.05)` → increments `declineCount`.
- It **never** deletes/moves/rewrites content — only frontmatter `declineCount`, via
  `parse_frontmatter → generate_frontmatter → write_atomic`.

Read-rate is treated as a **deliberately-conservative lower bound** on usefulness (the agent often
acts on inline advisory text *without* a `Read`; a live spot-check measured ~91% read-divergence). The
low `0.05` demote threshold absorbs the undercount.

**Two concurrency rules (the pass is not idempotent).**

- **WR-01 claim-before-mutate:** the pass must **claim** state (`_update_maintenance_state`) **before**
  applying any `declineCount` mutation — because per-file writes are atomic but the *pass* is not, and
  a mid-loop failure or the `timeout 2` SIGTERM (which skips Python's `finally`) would otherwise replay
  the pass and re-increment already-demoted memories (this is the failure class behind the historical
  22-demotion incident).
- **WR-02 recheck-under-lock:** the ≥50-record trigger count is re-verified **under the lock** to
  close the hook's read-then-act race.

**The three floors** (all three are load-bearing; a reseed must enumerate exactly these):

1. **Zero-fire floor (D-43):** `fire_count == 0` → **never demoted** (precedes rate computation —
   absence of fires is not evidence of dispensability).
2. **Minimum-evidence guard:** no real mutation until **≥ 10 distinct session-days OR ≥ 30 days span**
   (`minEvidenceSessions = 10`, counting distinct session-*days* not raw markers; `minEvidenceDays =
   30`, an **OR** of the two arms). Added after the 22-demotion incident; a refusal to mutate young
   telemetry IS the system running correctly, not a failure.
3. **Seat dual-gate:** a seat is proposed for demotion only when **both** a probe payload demonstrably
   surfaces it through the live recall hook **AND** telemetry shows it fired
   (`seatPromoteMinFires = 5`).

**Seat governance.** `MEMORY.md` router-seat membership is machine-governed inside the pass
(`seats()`), never hand-audited. Because the box-brain store is **not** git-tracked, proposed changes
are emitted as a human-vetoable `PENDING-SEAT-CHANGES` HTML-comment block prepended to `MEMORY.md`
(delete the block to approve); non-block content stays byte-identical and re-runs replace rather than
stack. With current always-on seats carrying no `triggers:`, probes return `covered:false` and the
engine proposes **zero** demotions — *the absence of a derivable probe is itself proof the seat
belongs in the router*.

---

## §9 — Performance contract

**The gate is regression-relative, not an absolute cliff.** As the corpus grows, recall p95 drifts
(dominated by ~30 ms Python startup + a larger catalog load) with *no actual read-path regression*; an
absolute cliff would then sit permanently red and its verdict would mean "the corpus is big," not "the
read path regressed." So the gate (`bench_recall.sh`) checks against a **committed baseline**:

```
ceiling = baseline + max(25%, 15 ms)
```

A committed integer file holds the accepted steady-state p95 (the baseline; a tunable, §10). Four
verdicts:

- **PASS** — `p95 ≤ ceiling` and within the design budget.
- **WARN** — over the design budget but `≤ ceiling`; advisory, **exit 0** (does not block); prints how
  to accept drift.
- **REGRESSED** — `p95 > ceiling`; a genuine structural slowdown, **exit 1** (blocks).
- **NOBASELINE** — no baseline file; measure-only, exit 0.

Accepting legitimate corpus-growth drift is a deliberate auditable act: `--update-baseline` rewrites
the baseline file as a reviewable diff. The script is **its own judge** (the old MVR/GSD judge is
retired, §12) — a true REGRESSED now actually blocks.

**The two "55 ms" — never conflate them.**

- The **live advisory design budget** (`BUDGET_MS = 55`): exceeding it produces **WARN / exit 0**.
  This is the number §9 carries. *(Rationale: the ~30 ms Python-startup floor is irreducible without a
  daemon, which is rejected on fail-open grounds; perf work targets the shell/jq layer, where
  consolidating jq spawns 7→3 already recovered ~6 ms.)*
- The **retired absolute cliff** (a hard `p95 ≤ 55 ms`): **dead.** It lives only in Appendix A as
  superseded rationale. Same number, opposite status.

---

## §10 — Tunables vs. invariants

The contract is **shape** (§2), never **number**. Every magnitude below is a tunable — the spec
states its default and links its rationale; the *number* is never the contract. Config-tunable values
live in `_memory_surface_config.json` (`DEFAULT_CONFIG`); a few are hardcoded engine constants, noted
as such (changing those is a code change, not a config change).

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

**Operator-CLI inversion (the one sanctioned exception to §2's hook rules).** Operator-*invoked* CLIs
(e.g. `scripts/lint.sh`) deliberately **invert** the hook discipline: they are **loud on success** and
**fail-CLOSED** on a missing dependency (exit non-zero + an install hint). The boundary is structural
— hook vs. operator-CLI — not per-script taste: output-on-success is the point of a CLI, and a missing
`shellcheck` is an actionable error, not something to swallow. This inversion is the sanctioned
exception to invariants §2.10 (quiet) and §2.7 (fail-open), which apply to *hooks*.

---

## §11 — Drift guardrail

The corpus-relative safety claims in this spec (the static gate's degenerate set, `live_levers`
liveness against `byArg`/`bySynonym` vocabulary, and `collisionGuideFloor`) are validated against the
*current* corpus. History shows this can silently go stale — the earlier co-fire collision model was
proven safe at ~9 memories and broke at 165 (Appendix A). For a system whose stated problem is drift,
that staleness must be made **loud**, not implicit.

**One fail-open invariant-check**, run at rebuild or SessionStart (where the catalog is already being
touched), emitting a single advisory line if violated — a **guardrail, not a gate**, with **no
per-corpus re-tuning of constants**:

- assert no existing trigger-bearing memory is a bare-degenerate-only set (would be denied by the
  static gate today), and
- assert no curated memory would be BLOCK-degenerate under the current verdict.

It never blocks; it only surfaces that a point-in-time assumption has drifted. (This is also where the
§4 routing-vs-ranking partition would surface if a future field ever became both.) The existing
structural-consistency machinery it builds on: every mutator rebuilds before returning, the PostToolUse
refresh rebuilds on any store `.md` write, `_load_catalog` centralizes shape validation, and
`fingerprint()` detects grammar staleness.

---

## §12 — Non-goals / out of scope

The reseed deliberately **does not** include, and must not re-import:

- **No GSD / planning spine.** GSD was removed entirely; its `.planning/` tree was distilled to ADRs +
  capability specs + box-brain memory and backed up at tag `gsd-archive-pre-removal`. Do not
  reintroduce a per-edit workflow-enforcement layer. *(This splintering is what the reseed escapes.)*
- **No SQLite / FTS5** on the routing path (§4).
- **No embeddings, no LLM on the read path** (§1/§5). Dedup similarity is stdlib `Counter` only.
- **No prompt-keyword routing** (§2.3) — it was implemented once and rolled back as noise.
- **No human-review ritual** (§8) — no Memory Roulette.
- **No bulk-LLM trigger derivation** over the corpus; legacy memories were made routable mechanically.
- **No second matcher** (§2.4) — recall and projection share `_walk_index`.
- **No per-corpus block cutoff** beyond the single `collisionGuideFloor` (§7) — nothing to drift.
- **No permissions writes** (§0 security constraint) — posture is the operator's alone.

**Fenced-out apparatus (named, not specified):**

- The **generic Claude-Code base-harness** (non-memory hooks: config-drift-guard, handoff-index,
  lab-scope, system-fingerprint, syntax-check-touched, etc.) — a separate concern. The permissions
  *guard* and install manifest live here, not in the memory engine.
- **corpusforge** — the seeker/helper-duel corpus-generation apparatus described in synapse's
  CONTEXT.md — is an entirely separate lab apparatus and contributes no term or capability here.

---

## §13 — Reseed / bootstrap procedure  **[DESIGNED]**

> **Provenance:** this section is **designed, not distilled** — the sources leave fresh-box bootstrap
> an explicit open gap (synapse's ADR-0013 documents that a fresh store's `_grammar.md`/`_tags.md`
> provenance is not install-managed). It is the part of the reseed that must be *built*, and it is
> the most likely to need revision once first exercised. Treat it as a proposal.

**Goal.** Bring up the memory subsystem on a clean box (or after a deliberate clean-slate reset) such
that the very first session has a valid, routable store and an empty-but-correct catalog.

**Cold-start sequence (proposed):**

1. **Seed the grammar.** Place `_grammar.md` in the lab and relative-symlink it into the box store
   (the install manifest manages exactly this one file — store taxonomy files like `_tags.md` are
   *data*, left in place, never staged/removed by the harness; see the install-manifest note below).
   A fresh box with no `_grammar.md` has no routing vocabulary and must start with at least an empty,
   schema-valid grammar.
2. **Seed/confirm the store.** Ensure the store directory exists with its infra files
   (`MEMORY.md` floor, any seat memories). The box-brain store is not git-tracked, so this is an
   operator-provisioned or scripted step, not a checkout.
3. **First rebuild.** Run `rebuild()` once. On an empty store this yields an empty-but-valid
   `_memory_catalog.json` with empty `triggerIndex` tables and a `routabilityReport: 0 unroutable`.
   The catalog is a build artifact (§4), so this is always safe to re-run.
4. **Mechanical legacy routability (if importing existing memories).** Grammar-covered memories route
   via tag-level evidence; any with no grammar coverage get one-time engine-side trigger derivation
   (`derive_fallback_triggers` extracts backtick-quoted command/path tokens from the body) written as
   index-side `byMemoryId` entries with `source = memory-derived` — **never** into frontmatter (so the
   store-is-source boundary holds). The cutover gate is a literal `routabilityReport: 0 unroutable`.
5. **Verify fail-open.** Confirm `.surface-disabled` suppresses cleanly and a missing catalog exits 0
   — the system must be safe before it is useful.

**Install-manifest boundary (carried from synapse's ADR-0013).** The harness install set manages
**only `_grammar.md`**; store taxonomy/data files are unmanaged and left in place (removing the
`_tags.md` symlink would break `validate`/`check_write`). "Stores are data, not install-managed code"
is the boundary the bootstrap honors.

**Open in this section (to resolve when built):** the empty-grammar schema minimum; whether step 2 is
scripted or manual; reconciling the step-4 `byMemoryId` fallback entries once natural triggers later
accrue.

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

## Appendix B — Open items (acknowledged, not yet resolved)

- **"Specific path" vs. "broad glob" boundary.** Path-liveness (§7) and the static gate's
  broad-glob-only arm (§6) both hinge on this distinction, which is asserted ("broad = a glob rooted
  at/above `$HOME`") but never given an exact, tested threshold anywhere in the sources. The one soft
  spot inside the otherwise-settled collision model.
- **Fresh-box bootstrap specifics** — see §13's "Open in this section."
- **One-time live-engine cutover recipe** (synapse's ADR-0016: dual flag-gate → atomic single-commit
  flip → zero config residue) is *historical migration*, not steady-state behavior; recorded here, not
  in the operative spec. Its `.surface-disabled` kill-switch lives in §5 independently.

