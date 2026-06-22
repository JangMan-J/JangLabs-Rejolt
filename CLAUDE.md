# bolt — agent conventions

> **Lab scope — `bolt/`** · nested repo [`JangLabs-Bolt`](https://github.com/JangMan-J/JangLabs-Bolt). This file is the authority for work *inside this lab* and **overrides** the workspace root [`../CLAUDE.md`](../CLAUDE.md). Stay in this lab — don't reach into or edit sibling labs from here.

## Read first

1. [`README.md`](./README.md) — what this lab is and how it's laid out.

## What lives here

**Focus: the routed-memory reseed.** `bolt` is the clean-slate reseed of the tag-routed
memory subsystem that grew over-tooled and splintered in the [`synapse`](../synapse) lab. Its
primary artifact is [`CORE-SPEC.md`](./CORE-SPEC.md) — one self-contained spec
from which that subsystem can be cleanly reseeded (recall, write-guard, self-curation, catalog,
collision-projection). [`CONTEXT.md`](./CONTEXT.md) is the one-screen domain glossary.

The spec is **distilled** from sources that live in the sibling `synapse` lab — its ADRs
(`../synapse/docs/adr/`), seed inventory (`../synapse/openspec/specs/_PENDING-FROM-GSD.md`), the two
promoted OpenSpec specs, and the live engine (`../synapse/lib/memory_surface.py`, the tiebreaker on
conflicts). Those are referenced **by path, never vendored** — honoring the workspace rule against
copying a sibling lab's files. Host-runtime base harnesses and the corpusforge apparatus are
explicitly **out of scope** for this reseed.

## Conventions

- **Branch for PRs:** `main`.
- This lab is its own git repo. Commit and push *inside* `bolt/`, then bump the pinned SHA at the
  JangLabs root (`git add bolt && git commit`).

## Agent skills

### Issue tracker

Issues and PRDs live as GitHub issues in [`JangMan-J/JangLabs-Bolt`](https://github.com/JangMan-J/JangLabs-Bolt), via the `gh` CLI. See [`docs/agents/issue-tracker.md`](./docs/agents/issue-tracker.md).

### Triage labels

Canonical triage vocabulary (`needs-triage` / `needs-info` / `ready-for-agent` / `ready-for-human` / `wontfix`), used as-is. See [`docs/agents/triage-labels.md`](./docs/agents/triage-labels.md).

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See [`docs/agents/domain.md`](./docs/agents/domain.md).
