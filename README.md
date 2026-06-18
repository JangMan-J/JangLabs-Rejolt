# JangLabs-Bolt

A **JangLabs lab** — an independent repo wired into the [JangLabs](https://github.com/JangMan-J/JangLabs)
multi-lab workspace as a git submodule (path `bolt/`).

**Focus: TBD.** This lab was scaffolded fresh; its purpose hasn't been decided yet.

## Layout

- [`CLAUDE.md`](./CLAUDE.md) — agent conventions and the lab-scope banner. Read first when working here.
- [`docs/agents/`](./docs/agents/) — per-repo configuration for the engineering skills (issue tracker,
  triage labels, domain docs).

## Working here

This is its own git repo. Make changes here, commit and push inside `bolt/`, then back at the JangLabs
root bump the pinned submodule SHA (`git add bolt && git commit`).
