# routed-memory-reseed — negative-contract sweep sheet (WP-8 part B, plan P17)

**Stage:** S3 `/build`, WP-8 part B · **Tier:** T2
**Spec:** `docs/frozen/routed-memory-reseed-decisions-20260703.md` ("Negative contract", N1–N14)
**Plan:** `docs/frozen/routed-memory-reseed-plan-20260704.md` — P17
**Executable half:** `tests/negative_contract.rs` (`cargo test --test negative_contract`)
**Built:** 2026-07-04, in an isolated worktree, disjoint from WP-8a/WP-8c

## How to run the sweep

```sh
cargo test --test negative_contract
```

Every row below is at least one `#[test]` in `tests/negative_contract.rs` named
`n<k>_...`. A future regression that reintroduces a forbidden pattern trips the
corresponding test (red `cargo test`), which is the gate `/vet` reads.

## The sweep

| N | Negative contract (verbatim) | Mechanical check | Test(s) | Result |
|---|---|---|---|---|
| N1 | No second matcher — recall and projection share one index walk (D4), in one language (D16). | (a) source grep: exactly one `fn walk(` definition exists under `src/`. (b) behavioral: one `Index` built from one record; drive it through `recall::recall` (via a `NormalizedOp`) AND through `projection::project` (via a `Triggers` set) and confirm both surface the SAME memory id. | `n1_exactly_one_walk_matcher_in_the_engine`, `n1_recall_and_projection_see_the_same_hit_through_the_one_walk` | **PASS** |
| N2 | No SQLite/FTS5 on the routing path; no embeddings or LLM on the read path. | Case-insensitive grep of `Cargo.toml` + every `src/*.rs` file for `sqlite`, `fts5`, `rusqlite`, `embedding` — assert zero hits anywhere in the engine. | `n2_no_sqlite_fts5_or_embeddings_anywhere_in_the_crate` | **PASS** |
| N3 | No prompt-keyword routing (D3). | (a) grep the two files that actually assemble routing tokens (`normalize.rs`, `recall.rs`) for the substring `prompt` — assert zero. (b) behavioral: feed `parse_host_event` a payload carrying an extra top-level `"prompt"` key and confirm the normalized op is byte-identical to the same payload without it (the field is simply not consulted). | `n3_no_prompt_text_field_feeds_routing`, `n3_normalized_op_carries_no_prompt_field` | **PASS** |
| N4 | No standing review ritual; curation never deletes or rewrites memory content (D7). | Behavioral, over real on-disk memory files + a real `Telemetry`/`maintain()` run: a zero-fire memory must come out **byte-identical** (D7 floor 1); a real fired-but-unread memory must demote via `declineCount` only, with its **body** byte-identical before/after; both memory files must still exist afterward (`maintain` never calls `remove_file` on a memory). | `n4_curation_demotes_by_frontmatter_only_body_and_zero_fire_files_never_touched` | **PASS** |
| N5 | No bulk-LLM trigger derivation — and per D18, no mechanical body-token derivation either: no inferred routes at all. | (a) grep every `src/*.rs` file for `derive_fallback`, `memory-derived`, `memory_derived`, `byMemoryId` — assert zero. (b) structural: `tier::Source` is matched exhaustively over exactly `{Tag, Memory}` — a third ("derived"/"fallback") provenance would fail to *compile*, which trips `cargo test` outright. | `n5_no_fallback_derivation_or_memory_derived_route_source`, `n5_route_source_is_closed_to_tag_or_declared_memory_trigger` | **PASS** |
| N6 | No per-corpus block cutoff beyond the single collision floor (D8). | (a) behavioral: two corpora of very different sizes (both just over the floor, one 500 records larger) must land on the SAME verdict boundary — the floor doesn't scale with corpus size. (b) grep confirming `guard.rs`/`rebuild.rs`/`config.rs` all *consume* `projection::COLLISION_GUIDE_FLOOR` rather than defining their own floor constant. | `n6_single_collision_floor_not_scaled_by_corpus_size`, `n6_no_second_breadth_cutoff_constant_defined_outside_projection` | **PASS** |
| N7 | No host permission-policy writes, including bootstrap (D13). | (a) behavioral: `hooks::hooks_settings_block`/`render_print_hooks` output never contains the JSON keys `"permissions"`, `"allow"`, `"deny"`, `"defaultMode"`. (b) `src/hooks.rs` (the module that BUILDS that output) contains no filesystem-write API call at all (`fs::write`, `fs::create_dir*`, `File::create`, `write_atomic`) — it is a pure builder, zero I/O. (c) behavioral: `bootstrap()`'s signature is `(store, grammar, config)` — no host-settings parameter exists for it to write through; a real call writes only under the two caller-given paths. | `n7_print_hooks_output_carries_no_permission_policy_keys`, `n7_hooks_module_performs_no_filesystem_writes`, `n7_bootstrap_writes_only_inside_the_caller_provided_store_and_grammar_paths` | **PASS** |
| N8 | No new facet axis, removal, or redefinition without a spec amendment; no facet-less tags; no tag-less memories (D21, D22). | Behavioral over the existing G2 fixture corpus: a 4th top-level grammar table (`fixtures/grammar/bad/fourth-table.toml`) must fail to even *deserialize* (`deny_unknown_fields`); a tag declared under two facet tables (`fixtures/grammar/bad/duplicate-facet.toml`) must fail `validate_grammar` with `DuplicateFacet`; a memory with no `metadata.tags` key or with `tags: []` (`fixtures/frontmatter/bad/{missing,empty}-tags.md`) must fail `frontmatter::parse` with `MissingTags`/`EmptyTags`. | `n8_a_fourth_top_level_facet_table_is_a_hard_deserialization_error`, `n8_duplicate_facet_tag_is_denied`, `n8_tagless_memory_is_denied_missing_and_empty` | **PASS** |
| N9 | No legacy-format parsing code in the engine; no import flag (D17). | (a) behavioral: `Cli::try_parse_from` on `rejolt bootstrap … --import-legacy` must fail to parse (clap: unrecognized argument) — the flag was never wired. (b) grep every `src/*.rs` file for `import_legacy`/`import-legacy`/`ImportLegacy` — assert zero; and confirm every line mentioning "legacy" (case-insensitive) is a `//`-comment line (currently exactly one, citing D17's rejection). | `n9_no_import_legacy_flag_on_the_cli`, `n9_no_legacy_parsing_code_in_the_engine` | **PASS** |
| N10 | No python (or any interpreter) on any engine path; no runtime dependencies beyond the static binary (D16). | (a) `Cargo.toml` `[dependencies]` table is exactly `{clap, libc, serde, serde_json, toml}` (all pure-Rust/thin-FFI, no interpreter, no process-spawning crate) and there is no `[dev-dependencies]` table. (b) grep every `src/*.rs` file for the literal `Command::new` (process spawn) — assert zero; the only `std::process::*` uses anywhere in `src/` are `std::process::id()` (temp-file/lock uniqueness) and `std::process::exit()` (the binary's own exit), never a subprocess spawn. | `n10_cargo_toml_dependencies_are_the_known_pure_rust_set`, `n10_no_process_spawn_anywhere_in_the_engine` | **PASS** |
| N11 | Recall never rebuilds, never loads memory bodies, never emits output on silence (D1, D19). | (a) behavioral: `recall()` against a store with NO index/report files returns `Silence`, and neither file exists afterward (no implicit rebuild). (b) behavioral: an index record whose `path` column points at a file that has never existed still fires a full advisory (proves the snippet/text come only from the pre-baked index columns, never a body read). (c) grep confirming `src/recall.rs` contains no `fs::`/`File::` call at all — it consumes only `catalog::read_artifacts`'s already-parsed result. | `n11_recall_never_rebuilds_a_missing_index`, `n11_recall_never_opens_the_memory_body_file`, `n11_no_body_read_call_on_the_recall_hot_path` | **PASS** |
| N12 | No vendoring of synapse files into bolt — reference by path only (workspace rule). | Recursively walk `src/`, `tests/`, `fixtures/`, `docs/` and assert no path contains the substring `synapse` and no file has a `.py` extension. (`git ls-files` was also checked by hand at authoring time: zero matches.) | `n12_no_synapse_files_or_python_sources_vendored_into_this_repo` | **PASS** |
| N13 | Adapter handlers never block a host operation on engine/store/index failure (D6). | Source check: `src/hook.rs` defines exactly two exit-code constants (`EXIT_OK=0`, `EXIT_DENY=2`); it never references `EXIT_FAIL` (the direct-CLI-only exit-1 constant from `cli.rs`) and contains no literal `return 1` / `-> 1` exit path. Every fault branch (unresolvable store, kill-switch, unparseable payload, unclassifiable op) returns `EXIT_OK`; the ONLY `EXIT_DENY` site is the write-guard's deliberate content-based `Deny`, which is a decision, not an engine/store/index *failure*. | `n13_hook_dispatch_defines_exactly_two_exit_codes_ok_and_deny` | **PASS** — full live-host behavioral confirmation is `tests/hook_dispatch.rs` (already existing, drives the binary end-to-end per the WP-5 verify pass); not re-driven here to avoid env-var-mutation test races in a parallel test binary. |
| N14 | No performance magnitude asserted without the D26 calibration protocol behind it. | (a) behavioral: with no committed baseline, `bench::verdict_of` returns `NoBaseline` and a non-empty (LOUD) advisory — no magnitude is asserted pre-calibration. (b) behavioral: **see finding below** — `bench::regression_ceiling` still folds the CORE-SPEC §9 static constants `CEILING_REL_SLACK`(0.25) / `CEILING_MIN_SLACK_MS`(15 ms) into the ceiling as an unconditional floor UNDER the A4-calibrated slack. | `n14_gate_is_measure_only_until_a_baseline_is_committed`, `n14_flagged_g4_the_ceiling_still_bakes_in_the_superseded_static_slack` | **MECHANICAL CHECK FOUND A CONTRADICTION — flagged G4, needs a /vet or owner decision (not fixed here; `src/bench.rs` is out of this packet's file scope).** |

## N14 finding, in full (G4 candidate — do not silently resolve)

**What the ledger says.** D9: "all magnitudes are deferred to the D26 calibration
protocol (**supersedes** CORE-SPEC §9's 55 ms budget and `max(25%, 15 ms)`
slack, which were python-stack calibrations)." A4(c) then pins the *replacement*
derivation as a closed formula: "ceiling slack floor = `max(3 × cross-run σ of
p95, observed min→max p95 band)` over ≥5 runs of ≥100 samples" — no static
minimum is mentioned anywhere in A4.

**What the code does.** `src/bench.rs::regression_ceiling`:

```rust
pub fn regression_ceiling(baseline_p95: f64, slack_floor: f64) -> f64 {
    let slack = (CEILING_REL_SLACK * baseline_p95)
        .max(CEILING_MIN_SLACK_MS)
        .max(slack_floor);
    baseline_p95 + slack
}
```

`CEILING_REL_SLACK = 0.25` and `CEILING_MIN_SLACK_MS = 15.0` are the exact two
numbers D9 says are superseded, still compiled in as an **unconditional lower
bound** on the ceiling — the real, A4-calibrated `slack_floor` can only ever
*widen* the ceiling, never narrow it below `baseline + 15 ms`. The module's own
doc comment concedes this is deliberate ("so on a noisy box whose measured
jitter exceeds the §9 static 15 ms, an in-jitter run does NOT trip a blocking
REGRESSED"), i.e. WP-7's builder chose to read "supersedes" as "the new slack
widens, it does not replace."

**Why it matters at this system's actual scale.** D16 measured the real Rust
recall path at 0.7–2.4 ms across 26→1000 memories. `n14_flagged_g4_…` pins the
consequence numerically: baseline `p95=1.0 ms`, a real calibrated slack of
`0.3 ms` (tiny, consistent with sub-ms jitter), and a measured `p95=10.0 ms` — a
**10× structural slowdown**, exactly the shape of regression the REGRESSED
verdict exists to catch — verdict comes back **`Pass`** (not even `Warn`),
because `ceiling = 1.0 + max(0.25, 15.0, 0.3) = 16.0 ms > 10.0 ms`. The static
15 ms floor is ~15–50× the actual measured recall latency at this scale, so it
does not function as a defensive minimum here; it disables the gate.

**Disposition needed (not decided here):** either (a) `regression_ceiling`
drops the static `CEILING_REL_SLACK`/`CEILING_MIN_SLACK_MS` floor so the
ceiling is purely calibration-derived per A4(c)'s literal formula, or (b) D9/A4
are amended to explicitly bless a retained static floor as intentional
defense-in-depth (and N14's wording is read as "no NEW uncalibrated magnitude,"
which this arguably isn't since it's the OLD one, not a new one). This sweep
takes no position between (a)/(b) — it only proves the current text and the
current code disagree, with an exact reproduction. `src/bench.rs` is outside
this packet's file scope (WP-8b writes only `tests/negative_contract.rs` and
this report), so no code change is made here.

## Notes for /vet

- All 14 rows have at least one currently-green mechanical test except the N14
  arithmetic pin, which is *intentionally* green while documenting a
  **contradiction** (it pins today's actual behavior, not the spec's ideal —
  see the assertion message in `n14_flagged_g4_the_ceiling_still_bakes_in_the_superseded_static_slack`
  for what a fix would need to change).
- N13's full live-host proof is `tests/hook_dispatch.rs`, not duplicated here
  (env-var mutation across a parallel test binary is a reliability risk this
  sweep declines to take on).
- N12 was also checked by hand against `git ls-files` at authoring time (zero
  `synapse`/`.py` matches); the in-test walk is the mechanical, repeatable form
  of that same check.
- No Nk in this sweep required a Cargo.toml or `src/*` change to pass (N1–N13
  hold cleanly today); only N14 surfaced the contradiction above.
