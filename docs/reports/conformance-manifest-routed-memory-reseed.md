# routed-memory-reseed — §14 conformance manifest (WP-8a, plan P16)

**Packet:** WP-8 part A (§14 conformance matrix, plan item P16).
**Spec:** `CORE-SPEC.md` §14 (the conformance matrix) + the Memory Trigger Grammar section it follows.
**Plan:** `docs/frozen/routed-memory-reseed-plan-20260704.md`, item **P16** (+ the WP-8 row of the WP
decomposition table).
**Scope:** map every §14 row, plus every P16-named new row, to its covering automated check(s). Rows
already proven by an earlier packet's suite are cited as-is (no duplication, per the packet brief);
this WP-8a packet adds `tests/conformance_matrix.rs` only for the residual gaps identified below.

Convention: `file.rs::test_name` is an integration test under `tests/`; `src/foo.rs::mod::test_name`
is a `#[cfg(test)]` unit test co-located with the code it exercises.

## §14 conformance matrix — 23 rows

| # | Contract area | Covering test(s) | Notes |
|---|---|---|---|
| 1 | Read path is index-only | `tests/recall.rs::missing_index_is_silent_and_does_not_rebuild`, `::deleted_index_stays_deleted_after_recall`, `::surfaces_without_opening_the_memory_body` (missing/absent catalog); **`tests/conformance_matrix.rs::malformed_index_record_fails_open_and_leaves_files_untouched`, `::malformed_report_json_fails_open_and_leaves_files_untouched`** (present-but-corrupt catalog, closed in WP-8a) | Full row now closed: absent AND corrupt-but-present both proven to return silence with no rebuild. |
| 2 | Adapter fail-open layering | `tests/hook_dispatch.rs::never_exits_one_across_every_event_and_payload_shape`, `::malformed_payload_fails_open_silently`, `tests/cli_contract.rs::hook_with_no_store_configured_is_fail_open_never_exit_one` | |
| 3 | Quiet adapter success | `tests/hook_dispatch.rs::quiet_allow_on_a_non_matching_bash_command` (adapter silent); `tests/cli_smoke.rs::help_exits_zero_and_names_binary`, `tests/cli_contract.rs::usage_errors_exit_two`, `::rebuild_human_and_json`, `::validate_clean_findings_and_taxonomy` (direct CLIs loud, fail-closed on missing deps) | |
| 4 | Store source / catalog artifact | `tests/flat_index.rs::rebuild_is_idempotent_and_writes_consistent_pair`, `::rb4_torn_pair_detected_fail_open`; **`tests/conformance_matrix.rs::rebuild_overwrites_direct_catalog_edits_not_merges`** (direct catalog edits overwritten, closed in WP-8a), **`::malformed_index_record_fails_open_and_leaves_files_untouched`, `::malformed_report_json_fails_open_and_leaves_files_untouched`** (malformed-but-parseable → `None`/fail-open, closed in WP-8a) | |
| 5 | Observable routing only | `tests/recall.rs::malformed_and_unknown_payloads_are_silent_no_ops` (prompt-only never routes — no command/path/arg/synonym evidence, silence); `tests/grammar.rs::known_bads_distinct_classifiable_errors` (`synonyms-only` fixture: a tag with no behavioral evidence fails grammar validation) | |
| 6 | Single matcher | `tests/flat_index.rs::rb9_recall_equals_projection_single_walk`, `tests/projection.rs::rb9_projection_uses_the_same_walk_as_recall` | Same fixture trigger set through `Index::walk`, both call sites. |
| 7 | Surface gate and scoring | `tests/recall.rs::one_strong_tuple_fires`, `::one_weak_tuple_is_silent`, `::two_tuples_fire`, `::generic_verb_does_not_count_as_strong`, `::generic_command_never_shadows_a_specific_one_order_independent`, `::web_keywords_are_synonym_only_never_byarg`; `src/recall.rs::tests::surface_gate_matrix`, `::confidence_thresholds` (stale/decline penalty → confidence label) | |
| 8 | Diagnosable fires | `tests/recall.rs::advisory_renders_the_grammar_route_tag_citation`; `src/recall.rs::tests::citation_renders_frozen_form` | |
| 9 | Path specificity | `tests/path_specificity.rs::every_fixture_example_classified_exactly`, `::g2_path_specificity`; `src/path_class.rs::tests::every_section_3x_broad_example_is_broad`, `::every_section_3x_specific_example_is_specific`; `src/guard.rs::tests::static_gate_rescue_equals_shared_live_levers` (name literally proves §6/§7 share one `live_levers` classifier) | |
| 10 | Path canonicalization divergence | `src/normalize.rs::tests::lexical_canonicalization_resolves_dot_dot_without_symlinks`, `::lexical_canonicalization_expands_home_anchor` (adapter-lexical mechanics); **`tests/conformance_matrix.rs::post_op_grammar_symlink_write_matched_via_pure_lexical_dotdot_collapse`** (adapter-lexical recognizes the symlinked store grammar through a pure `..` detour, no filesystem access, and does not false-positive on a look-alike path — closed in WP-8a); **`tests/conformance_matrix.rs::engine_realpath_containment_blocks_symlink_based_escape`** (engine-realpath containment is not fooled by a symlink-based `../`-style escape — closed in WP-8a) | This row had the weakest prior coverage; both halves needed a new end-to-end fixture. |
| 11 | Write-guard boundary | `tests/write_guard.rs::grammar_partial_edit_fails_open_and_bootstrap_allows`, `::fail_open_partial_edit_frontmatterless_and_infra`, `::static_gate_fails_open_when_catalog_vocab_absent` | |
| 12 | Write deny reasons | shape/evidence: `::shape_evidence_denies_invalid_frontmatter_allows_valid`; static degeneracy: `::static_degeneracy_denies_with_vocab_rescues_allow`; dedup backstop: `::dedup_backstop_denies_new_duplicate_allows_consolidation_and_near_miss`; BLOCK-degenerate: `::collision_block_degenerate_denies_guide_broad_allows`; misplacement: `::misplacement_denies_all_box_to_non_box_allows_mixed_and_box_target` (all in `tests/write_guard.rs`) | All five deny reasons individually fixtured, each with a near-miss allow. |
| 13 | Dedup semantics | `tests/write_guard.rs::dedup_backstop_denies_new_duplicate_allows_consolidation_and_near_miss`, `::consolidation_via_noncanonical_tilde_path_is_exempt_from_new_file_tiers`; `src/guard.rs::tests::bow_cosine_and_dedup_score_bounds` | |
| 14 | Collision projection | `tests/projection.rs::verdict_strict_greater_than_floor`, `::live_levers_are_index_key_membership`, `::empty_projection_carries_levers_and_passes` | |
| 15 | Placement enforcement | `tests/write_guard.rs::misplacement_denies_all_box_to_non_box_allows_mixed_and_box_target`; `src/guard.rs::tests::infra_and_grammar_target_detection` | |
| 16 | Telemetry capture | `tests/telemetry.rs::correlation_fire_logged_when_mark_persists_zerofire_when_not`, `::rotation_at_max_and_reads_dotone_before_live`, `::bad_ts_drops_symmetrically` | |
| 17 | Maintenance concurrency | `src/curation.rs::tests::wr02_recheck_under_lock_catches_a_racing_claim`, `::claim_before_mutate_advances_state_even_on_insufficient_evidence`, `::lock_fresh_is_busy_stale_is_reclaimed` (atomic rename-to-corpse); `tests/curation.rs::maintain_no_ops_while_another_pass_holds_the_lock` | |
| 18 | Curation floors | `tests/curation.rs::zero_fire_floor_never_demotes_while_low_read_rate_does_once_evidence_is_sufficient`, `::min_evidence_span_leg_fires_from_unwindowed_telemetry_not_the_30d_window`, `::seat_dual_gate_demotes_only_when_covered_and_fires_meet_threshold`, `::seat_promote_min_fires_zero_does_not_panic_and_never_demotes_a_zero_fire_seat` | |
| 19 | Seat governance | `tests/curation.rs::seats_propose_twice_preserves_memory_md_leading_blank_lines` (`PENDING-SEAT-CHANGES` replace-not-stack + non-block byte-identity); `src/curation.rs::tests::pending_block_replaces_not_stacks_and_preserves_non_block_content`; the never-rewrites-a-body contract (D7) is asserted directly in `tests/curation.rs` via its `body_of`/`read_memory` helpers — see e.g. lines asserting `zero_fire_after == zero_fire_original` and `body_of(mutated) == "BODY for … — must never change."` across the floors AND the seat-governance test | Verified by reading the test bodies, not just names: every curation test that demotes/promotes/proposes also re-reads the memory file and asserts its body is byte-identical. |
| 20 | Performance gate | `tests/bench.rs::nobaseline_is_measure_only_exit_zero` (NOBASELINE), `::calibrate_writes_reviewable_env_fingerprinted_baseline`, `::env_fingerprint_mismatch_is_loud_measure_only`; `src/bench.rs::tests::verdicts_pass_warn_regressed_under_matching_env` (PASS/WARN/REGRESSED via the pure `verdict_of` formula) | `BenchOutcome::exit_code()` is a 2-arm match (`Regressed => 1, _ => 0`) — structurally guaranteed once each verdict is independently proven reachable, which the cited tests do. |
| 21 | Drift guardrail | `tests/flat_index.rs::drift_guardrail_fires_on_degenerate_and_is_silent_otherwise`; `tests/hook_dispatch.rs::session_start_emits_routability_and_drift_advisories_when_not_at_home` (advisory-only, never blocks — session-start always exits 0) | |
| 22 | Security boundary | `src/hooks.rs::tests::block_carries_no_permission_policy_keys`; `tests/cli_contract.rs::print_hooks_emits_json_and_writes_no_host_settings` | |
| 23 | Bootstrap | `tests/bootstrap.rs::bootstrap_seeds_expected_files_and_is_idempotent`, `::bootstrap_never_overwrites_user_files`, `::bootstrap_verification_rows_hold` (`.surface-disabled` verified, missing-catalog fail-open without rebuild verified), `::empty_grammar_seed_is_version_line_alone_and_validates`, `::bootstrap_rejects_a_preexisting_invalid_grammar` | **Residual, not a test gap:** the row's "applies the `byMemoryId` lifecycle" clause names a feature the FROZEN PLAN deliberately cut — the plan's orphan ledger: *"`byMemoryId` table in the flat index — cut: dead on the old read path, producer removed by D18 (A2)"*. There is no `byMemoryId`/memory-derived-route producer in this codebase to exercise; CORE-SPEC §14 still names it because §13 was distilled before that cut. This is a spec-vs-plan wording drift, not a missing check — flagged for the owner/`/vet`, not closeable by a test. |

## P16 new rows

| Row | Requirement | Covering test(s) | Notes |
|---|---|---|---|
| A | Write-guard deny contract: exit 2 + stderr from the guard branch, exit 0 quiet elsewhere | `tests/hook_dispatch.rs::deny_contract_short_circuits_stderr_exit_2_no_stdout`, `::allowed_memory_write_emits_write_context_exit_0`, `::never_exits_one_across_every_event_and_payload_shape` | Engine/CLI half only. The LIVE-host half is **RB1(b)**, owner-discharged at `/vet` (per plan Budget + risk register) — out of scope for this packet. |
| B | Malformed payload passes silently | `tests/hook_dispatch.rs::malformed_payload_fails_open_silently` (end-to-end, built binary); `tests/recall.rs::malformed_and_unknown_payloads_are_silent_no_ops` (engine) | |
| C | Fourth facet table + duplicate-facet tag → exit 2 | `tests/grammar.rs::known_bads_distinct_classifiable_errors` (`fourth-table` and `duplicate-facet` fixtures, both asserted `exit_code() == 2` with distinct typed `GrammarError` variants) | |
| D | One-line-per-entry under hostile field content | `tests/flat_index.rs::rb2_one_record_per_line_under_hostile_content`, `::hostile_filename_excludes_whole_memory_and_keeps_index_wellformed`, `::lastreviewed_tab_is_sanitized_not_line_splitting` | |
| E | Cross-artifact generation match | `tests/flat_index.rs::rebuild_is_idempotent_and_writes_consistent_pair`, `::rb4_torn_pair_detected_fail_open` (asserts `header.generation == report.generation` on a fresh rebuild via `ArtifactRead::Consistent`) | |
| F | Unknown-tool fail-open | `tests/recall.rs::malformed_and_unknown_payloads_are_silent_no_ops` (a `SomeFutureTool` normalizes but extracts no routable token → silent); `src/normalize.rs::tests::fail_open_on_malformed_and_unknown` | |
| G | recall≡projection same-hit-set | `tests/flat_index.rs::rb9_recall_equals_projection_single_walk`, `tests/projection.rs::rb9_projection_uses_the_same_walk_as_recall` | Same as row 6 above (one contract, two names in the plan/spec). |
| H | End-to-end byPath glob fire | `tests/recall.rs::bash_path_routing_end_to_end` (full parse→recall pipeline over a real host-event JSON payload); `tests/flat_index.rs::rb11_bypath_glob_survives_build_and_fires_correctly` (glob survives the build step) | Covered at the full engine-API level (`parse_host_event` → `recall`), which is the "public API" half of the packet brief's "drive the built binary or the public API" instruction. |
| I | Citation renders the grammar route_tag | `tests/recall.rs::advisory_renders_the_grammar_route_tag_citation` | |
| J | Recall fail-open on missing/corrupt catalog (allow, surface nothing, NO rebuild) | Missing: `tests/recall.rs::missing_index_is_silent_and_does_not_rebuild`, `::deleted_index_stays_deleted_after_recall`. **Corrupt (present-but-unparseable): `tests/conformance_matrix.rs::malformed_index_record_fails_open_and_leaves_files_untouched`, `::malformed_report_json_fails_open_and_leaves_files_untouched`** (closed in WP-8a — this half was a genuine gap: every prior test exercised only an *absent* catalog, never a *present-but-corrupt* one) | |
| K | `.surface-disabled` kill-switch — every adapter path → allow, silent | `tests/hook_dispatch.rs::kill_switch_suppresses_every_event_including_a_deny` (write-deny / session-start / post-op-read branches). **`tests/conformance_matrix.rs::kill_switch_suppresses_a_would_be_recall_fire`, `::kill_switch_suppresses_a_would_be_write_context_emission`, `::kill_switch_suppresses_post_op_rebuild_refresh`** (the three remaining branches — a would-be recall advisory, a would-be write-context emission, and a would-be rebuild-refresh — closed in WP-8a) | Each new test carries an inline GOOD contrast proving the suppressed branch would otherwise have fired/emitted/rebuilt — so the kill-switch proof isn't vacuous. |

## Coverage summary

- **23 / 23** §14 rows have ≥1 automated check in the default `cargo test` path.
- **11 / 11** P16 new rows have ≥1 automated check.
- **One accepted residual, not a test gap:** §14 row 23's "`byMemoryId` lifecycle" clause names a
  feature the frozen plan's orphan ledger explicitly cut (D18/A2 removed the legacy memory-derived
  fallback-route producer). There is nothing in the codebase to exercise; this is a CORE-SPEC-vs-plan
  wording drift pre-dating this build, flagged here for the owner / `/vet`, not closeable by adding a
  test.
- **One out-of-scope-by-design residual:** row A / RB1 / plan Budget's live-host half (RB1(b)) —
  actually observing the host block on the proven exit-2/stderr mechanism is a human-only step,
  owner-discharged at build START per the risk register, and re-verified at `/vet`. The engine/CLI
  contract half (exit 2 + stderr from the guard branch, exit 0 quiet elsewhere, never exit 1) is fully
  covered above.

## New tests added in WP-8a (`tests/conformance_matrix.rs`, 8 tests)

1. `malformed_index_record_fails_open_and_leaves_files_untouched` — rows 1, 4, J.
2. `malformed_report_json_fails_open_and_leaves_files_untouched` — rows 1, 4, J.
3. `rebuild_overwrites_direct_catalog_edits_not_merges` — row 4.
4. `kill_switch_suppresses_a_would_be_recall_fire` — row K.
5. `kill_switch_suppresses_a_would_be_write_context_emission` — row K.
6. `kill_switch_suppresses_post_op_rebuild_refresh` — row K.
7. `engine_realpath_containment_blocks_symlink_based_escape` — row 10.
8. `post_op_grammar_symlink_write_matched_via_pure_lexical_dotdot_collapse` — row 10.

Every other §14/P16 row cited above was already covered by an earlier packet's suite and is cited,
not duplicated, per the WP-8a packet brief.
