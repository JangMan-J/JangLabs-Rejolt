# fixtures/

Conformance fixtures for the `rejolt` engine, laid out to enforce **G2**
(`WORKFLOW.md` §6): *a conformance check's verdict does not count until it has
failed a known-bad fixture AND passed a known-good one.*

## Layout convention (every packet follows this)

```
fixtures/<area>/good/   # conformant inputs — the check MUST accept each
fixtures/<area>/bad/    # non-conformant inputs — the check MUST reject each
```

- `<area>` names the check family (e.g. `frontmatter`, `flat-index`,
  `write-guard`). Pick one area per conformance surface.
- Every regular file directly under `good/` or `bad/` is one fixture. The G2
  harness (`rejolt::conformance`) lists them non-recursively and sorted.
- **A check with no `bad/` fixture (or no `good/` fixture) does not count** —
  the harness returns `Verdict::Undisciplined`. Landing a check means landing
  its known-good *and* its known-bad fixture in the same packet.
- The predicate convention is: `true` = the check accepts the fixture,
  `false` = it rejects it. Good fixtures must be accepted; bad fixtures must be
  rejected.

## Areas

- `selftest/` — WP-0's proof that the harness itself works. `good/nonempty.txt`
  is non-empty; `bad/empty.txt` is a zero-byte file. The reference check
  ("fixture file is non-empty") accepts the good and rejects the bad, so its
  verdict counts. See `tests/conformance_selftest.rs`. Do not repurpose this
  area — later packets add their own.
- `frontmatter/` — WP-1 (P2). The bespoke constrained-YAML dialect (D21, A3).
  `good/` are in-subset memory frontmatters (the §3 worked example, flow/block
  sequences, quoted scalars, ranking fields, a minimal one); `bad/` is one
  fixture per Appendix B2 reject rule (anchors, aliases, type tags,
  multi-document, block scalars, flow mappings, multiline strings, tab
  indentation, duplicate keys, top-level `triggers:`, unknown keys) plus the
  D21 schema rejects (missing/empty/invalid tags, missing fences). The check
  `frontmatter-parse-valid` accepts iff parse + schema validation succeeds.
  Bads double as the vector-corpus oracle (each maps to its named error). See
  `tests/frontmatter.rs`.
- `grammar/` — WP-1 (P3). The serde-typed `grammar.toml` loader (D22, D23, D3,
  A6). `good/` = a valid multi-facet grammar and the empty seed (version line
  alone); `bad/` = the RB5 corpus (fourth facet table, duplicate-facet tag,
  synonyms-only tag, bad/missing `grammar-version`, bad `placement`, unknown
  entry field), each rejected as an exit-2 config/taxonomy error. The check
  `grammar-load-valid` accepts iff parse + validate succeeds. See
  `tests/grammar.rs`.
