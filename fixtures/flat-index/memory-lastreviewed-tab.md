---
name: rg-notes
description: ripgrep tips
metadata:
  tags: [ripgrep]
  triggers:
    commands: [rg]
  lastReviewed: "a\tb"
---
An ordinary, well-formed memory whose `lastReviewed` scalar decodes (via the
double-quote `\t` escape) to a real tab. It must index normally — the tab is
sanitized to a space at build so it cannot split the flat-index line. This file
lives directly under `flat-index/` (not `good/` or `bad/`) so the G2 index-loader
check does not pick it up; `tests/flat_index.rs` loads it by path.
