# CONTEXT — bolt (routed-memory reseed)

Domain glossary for the `bolt` lab. The lab's full deliverable is [`CORE-SPEC.md`](./CORE-SPEC.md) —
the consolidated reseed spec for the tag-routed memory subsystem distilled from the sibling
[`synapse`](../synapse) lab. **Read `CORE-SPEC.md` for the actual contract;** this file is the
one-screen vocabulary the engineering skills read before exploring.

## Scope

In scope: the memory subsystem — recall, write-guard, self-curation, catalog, collision-projection.
Out of scope (named, not specified): host-runtime base harnesses and the corpusforge apparatus.

## Ubiquitous language

| Term | Meaning |
|------|---------|
| **host runtime** | The agent environment that supplies lifecycle events, structured tool-operation events, filesystem access, and allow/deny behavior. |
| **adapter** | Host-specific glue that maps runtime events into normalized memory operations and maps engine results back to host allow/advisory/deny behavior. |
| **normalized operation** | The adapter-facing event shape used by the engine: operation kind, tool name, structured input, target paths, command text/args, and full proposed file content when guarding a write. |
| **store** | The directory of memory files + infra files (`_grammar.md`, `MEMORY.md`). The source of truth. |
| **memory** | One markdown file: frontmatter (`metadata:` with a `triggers:` block) + body. |
| **grammar** | `_grammar.md` — the trigger grammar. *A tag **is** its evidence patterns.* |
| **placement hint / placement model** | A tag's store target (`box`/`project`/`either`) plus target classification for write guidance and high-confidence misplacement denial. |
| **trigger** | A behavioral evidence pattern that routes a memory: command, path, arg token, or synonym. |
| **catalog** | `_memory_catalog.json` — the rebuildable build artifact holding the routing index. Never the source of truth. |
| **triggerIndex** | The inverted-index in the catalog: `byCommand`/`byPath`/`byArg`/`bySynonym`/`byMemoryId`. |
| **routabilityReport** | The rebuild report listing unroutable memories; clean cutover/bootstrap requires `0 unroutable`. |
| **memory-derived route** | A catalog-only `byMemoryId` fallback for legacy memories with no grammar/frontmatter route; never frontmatter. |
| **recall / fire** | A per-operation lookup; a surfaced memory has *fired*. |
| **tier** | Strength of a matched trigger: command/path = strong, arg = medium, synonym = weak. |
| **surface gate** | The precision threshold a candidate set must clear to fire; below it, silence. |
| **fail open / fail closed** | Failure direction: fail open proceeds silently; fail closed denies, and only the write-guard boundary hard-denies memory writes. |
| **co-fire breadth** (`distinct_count`) | How many *other* memories a proposed trigger set would also match — the collision quantity. |
| **broad path / specific path** | A path trigger classified by the shared lexical `is_broad_path()` rule. Broad paths are root/home/current-dir catchalls with no concrete narrowing segment. |
| **narrowing / live lever** | An explicitly declared trigger that actually routes the memory (routable arg/synonym, or a specific path). |
| **full-file write / partial edit** | A full-file write supplies complete proposed file content before commit. A partial edit supplies only a diff, patch, range, shell command, or incomplete post-edit content. |
| **write-guard boundary** | The only fail-closed write surface: a full-file write of a frontmatter-bearing memory. Deny reasons live in the write-path spec. |
| **verdict** | The write-time collision outcome: PASS / GUIDE-broad / BLOCK-degenerate. |
| **bootstrap** | Clean-store reseed via `bootstrap-store`: seed grammar/router files, rebuild catalog, verify fail-open checks. |
| **seat** | A memory holding a slot in `MEMORY.md`, the always-loaded router floor; machine-governed. |

## Source material (in the sibling synapse lab, referenced by path, never vendored)

- `../synapse/docs/adr/` — the 19 ADRs (the *why*)
- `../synapse/openspec/specs/_PENDING-FROM-GSD.md` — the current-state capability inventory
- `../synapse/openspec/specs/{write-guard,collision-projection}/` — the two promoted capability specs
- `../synapse/lib/memory_surface.py` — the live engine (tiebreaker on any conflict)
