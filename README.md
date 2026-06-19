# JangLabs-Bolt

A **JangLabs lab** — an independent repo wired into the [JangLabs](https://github.com/JangMan-J/JangLabs)
multi-lab workspace as a git submodule (path `bolt/`).

**Focus: the routed-memory reseed.** `bolt` is the clean-slate reseed of the tag-routed memory
subsystem that grew over-tooled and splintered in the sibling [`synapse`](../synapse) lab. Its
deliverable is a single self-contained core spec from which that subsystem can be cleanly re-grown,
distilled from synapse's ADRs, seed inventory, OpenSpec specs, and live engine (referenced by path,
never vendored).

## Layout

- [`CLAUDE.md`](./CLAUDE.md) — agent conventions and the lab-scope banner. Read first when working here.
- [`CORE-SPEC.md`](./CORE-SPEC.md) — **the deliverable**: the consolidated reseed
  spec (recall, write-guard, self-curation, catalog, collision-projection; base-harness and
  corpusforge fenced out).
- [`CONTEXT.md`](./CONTEXT.md) — one-screen domain glossary (the single-context doc the engineering
  skills read before exploring).
- [`docs/agents/`](./docs/agents/) — per-repo configuration for the engineering skills (issue tracker,
  triage labels, domain docs).

## Working here

This is its own git repo. Make changes here, commit and push inside `bolt/`, then back at the JangLabs
root bump the pinned submodule SHA (`git add bolt && git commit`).
