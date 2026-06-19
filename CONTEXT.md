# CONTEXT — bolt (routed-memory reseed)

Domain glossary for the `bolt` lab. The lab's full deliverable is [`CORE-SPEC.md`](./CORE-SPEC.md) —
the consolidated reseed spec for the tag-routed memory subsystem distilled from the sibling
[`synapse`](../synapse) lab. **Read `CORE-SPEC.md` for the actual contract;** this file is the
one-screen vocabulary the engineering skills read before exploring.

## Scope

In scope: the memory subsystem — recall, write-guard, self-curation, catalog, collision-projection.
Out of scope (named, not specified): synapse's generic base-harness and the corpusforge apparatus.

## Ubiquitous language

| Term | Meaning |
|------|---------|
| **store** | The directory of memory files + infra files (`_grammar.md`, `MEMORY.md`). The source of truth. |
| **memory** | One markdown file: frontmatter (`metadata:` with a `triggers:` block) + body. |
| **grammar** | `_grammar.md` — the trigger grammar. *A tag **is** its evidence patterns.* |
| **trigger** | A behavioral evidence pattern that routes a memory: command, path, arg token, or synonym. |
| **catalog** | `_memory_catalog.json` — the rebuildable build artifact holding the routing index. Never the source of truth. |
| **triggerIndex** | The inverted-index in the catalog: `byCommand`/`byPath`/`byArg`/`bySynonym`/`byMemoryId`. |
| **recall / fire** | A per-tool-call lookup; a surfaced memory has *fired*. |
| **tier** | Strength of a matched trigger: command/path = strong, arg = medium, synonym = weak. |
| **surface gate** | The precision threshold a candidate set must clear to fire; below it, silence. |
| **co-fire breadth** (`distinct_count`) | How many *other* memories a proposed trigger set would also match — the collision quantity. |
| **narrowing / live lever** | An author-controlled trigger that actually routes the memory (routable arg/synonym, or a specific non-broad-glob path). |
| **verdict** | The write-time collision outcome: PASS / GUIDE-broad / BLOCK-degenerate. |
| **seat** | A memory holding a slot in `MEMORY.md`, the always-loaded router floor; machine-governed. |

## Source material (in the sibling synapse lab, referenced by path, never vendored)

- `../synapse/docs/adr/` — the 19 ADRs (the *why*)
- `../synapse/openspec/specs/_PENDING-FROM-GSD.md` — the current-state capability inventory
- `../synapse/openspec/specs/{write-guard,collision-projection}/` — the two promoted capability specs
- `../synapse/lib/memory_surface.py` — the live engine (tiebreaker on any conflict)
