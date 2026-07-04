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
