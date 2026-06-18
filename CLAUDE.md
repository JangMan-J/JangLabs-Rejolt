# bolt — agent conventions

> **Lab scope — `bolt/`** · nested repo [`JangLabs-Bolt`](https://github.com/JangMan-J/JangLabs-Bolt). This file is the authority for work *inside this lab* and **overrides** the workspace root [`../CLAUDE.md`](../CLAUDE.md). Stay in this lab — don't reach into or edit sibling labs from here.

## Read first

1. [`README.md`](./README.md) — what this lab is and how it's laid out.

## What lives here

**Focus: TBD.** `bolt` is a fresh JangLabs lab, scaffolded as its own repo and wired into the
workspace as a submodule. Its purpose hasn't been pinned down yet — update this section (and the
lab row in the root [`../README.md`](../README.md) and [`../CLAUDE.md`](../CLAUDE.md)) once the lab's
focus is decided.

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
