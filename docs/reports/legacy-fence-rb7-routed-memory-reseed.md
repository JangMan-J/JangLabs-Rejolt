# RB7 — legacy corpus disposition (WP-8c, P18)

**Packet:** WP-8c (P18 — the legacy fence) · **Stage:** S3 `/build` · **Tier:** T2
**Plan:** `docs/frozen/routed-memory-reseed-plan-20260704.md` (FROZEN v1)
**Spec:** `docs/frozen/routed-memory-reseed-decisions-20260703.md` (D15, D17, D18)
**Handed to:** `/vet` as the RB7 disposition record (walk-back should treat this
as CLOSED, not re-open the measurement question).

## Disposition: ACCEPTED RISK

RB7 is closed as an **accepted risk**, not a defect to fix in this build. The
legacy synapse-era memory corpus does not migrate into the reseeded engine —
by design (D17) — and the owner accepted that consequence at freeze, with the
actual corpus size known and small.

## What was measured (cited, not re-derived)

Per the frozen plan's Risk register, RB7:

> Legacy corpus disposition — measured 2026-07-04: box-brain 30 memories, all
> project stores combined 70 (well under the 165-memory synapse milestone);
> empty bootstrap means the routable corpus is re-authored by hand or
> abandoned; owner accepted (D17), reconfirmed at freeze with the number
> known.

That is: **box-brain 30** memories + **project stores ~70** memories combined
(measured against the live synapse-engine corpus, `../synapse`), well under
the 165-memory milestone synapse had reached. This build does not re-measure
those stores — the plan's freeze-time count is the citation of record. (No
cheap read-only corroboration was attempted: the box-brain/project store
locations are synapse-lab/host-adapter concerns, out of scope for this reseed
per `AGENTS.md`'s "host-runtime base harnesses ... out of scope.")

## Why this is accepted, not fixed

- **D17 (no legacy import):** `--import-legacy` is dropped entirely; the
  reseed bootstraps an **empty** store. Existing synapse-era memories migrate
  by hand or not at all — this was an owner-recorded rationale ("third
  rewrite; focus the idea, not the parts"), not an oversight.
- **D18 (fallback trigger derivation removed):** even if old bodies were
  copied over verbatim, there is no mechanism left to route them — every
  route is now declared (grammar or frontmatter), never inferred from body
  text. A copied-over legacy file would sit in the store unrouted and surface
  only via `routabilityReport`'s advisory count.
- **D15 (clean-slate wire formats):** the old formats (`_tags.md`,
  `_tag_links.md`, the markdown `_grammar.md`) have no reader in the new
  engine at all (see `tests/legacy_fence.rs`, this same packet) — there is no
  format-level bridge to migrate *through* even if an import path existed.
- **Consequence, accepted at freeze:** empty bootstrap (P14) means the
  routable corpus after reseed is **zero** until someone re-authors memories
  by hand against the new grammar/frontmatter dialect. At 30 (box-brain) + 70
  (project stores) = 100 memories total, hand re-authoring is a bounded,
  finite task, not an unbounded one — this is the basis on which the owner
  accepted the risk rather than building any import path.

## Rejected alternatives (from D17)

- **One-time frontmatter conversion** — cut: still couples the new engine to
  a migration pass, and D18 removes the routing mechanism such a conversion
  would feed.
- **Wrap-in-place legacy reader** — cut: a permanent compat shim is the
  full-compat posture through the back door, which D15 explicitly rejected.
- **One-shot legacy body-copy migration helper** — cut in the plan's orphan
  ledger: "a body-copy tool is import-lite through the back door." Escalated
  to the owner as optional, outside this plan; not built here.

## Mechanical proof this build does not quietly reopen RB7

`tests/legacy_fence.rs` (this packet) asserts, over the actual `src/` tree,
that no legacy-import or legacy-format-parsing surface exists to carry the
corpus over implicitly:

- no `--import-legacy` flag/handling (source-level AND the built binary's
  `bootstrap --help`, which lists only `--store`/`--grammar`/`--print-hooks`);
- no `derive_fallback_triggers` / `source = memory-derived` / `byMemoryId`
  producer (D18's fallback lifecycle is fully absent, not merely unused);
- no `_tags.md` / `_tag_links.md` parser, no `parse_tags_md` /
  `parse_tag_links` (D15's superseded legacy formats);
- no legacy `_grammar.md` (markdown) reader — the only appearance of that
  filename anywhere in `src/` is the infra-file classifier recognizing it as
  a name to *skip* during a store scan, not a parse target.

All 8 fence tests pass against the current tree; the suite was verified to
actually trip (not a rubber stamp) by temporarily reintroducing a
`derive_fallback_triggers` function and observing the corresponding test fail,
then reverting.

## For /vet

- RB7 status: **ACCEPTED RISK, closed.** No further action expected in this
  build or in `/vet`; re-authoring the ~100-memory corpus by hand (or
  abandoning it) is the owner's post-reseed follow-up, not a build blocker.
  If `/vet` wants independent corroboration of the 30/70 count, that requires
  reaching into the `synapse` lab's live stores (out of scope here) — this
  record only cites the plan's freeze-time measurement.
- The legacy fence (`tests/legacy_fence.rs`) is the durable regression gate:
  if a future change reintroduces any of the surfaces above, this suite is
  where it trips, not a review comment.
