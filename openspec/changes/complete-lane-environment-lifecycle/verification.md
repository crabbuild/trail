# Verification evidence

Verified on macOS on 2026-07-28.

## Lane environment inheritance

| Scenario | Direct evidence |
|---|---|
| Compatible parent generation | `lane_fork_inherits_verified_immutable_layer_with_fresh_private_uppers` verifies the child generation and view reference the parent's verified layer ID. The native Node NFS-COW acceptance extends this through a real fork and module load. |
| Mixed compatibility | `lane_fork_inherits_verified_immutable_layer_with_fresh_private_uppers` inherits `node` and rejects a missing/corrupt cache layer with `layer_verification_failed`. |
| No active parent generation | `fork_without_parent_generation_is_safe_and_reports_why_reuse_was_skipped`. |
| Parent and child write the same generated path | The inheritance integration test verifies distinct source/generated/scratch uppers. `two_views_share_one_layer_but_copy_writes_to_private_generated_uppers` and the native Node fork acceptance verify private generated writes do not alter the shared immutable layer. |
| Parent runtime is active | The inheritance integration test seeds a running parent allocation and verifies the child generation contains no inherited runtime resources. |
| Inspect fork generation | The inheritance integration test verifies `predecessor_generation_id` and the durable `lane_environment_inheritance` event's inherited/rejected component reasons. It also covers four concurrent forks and continued child use after parent removal. |

## Lane retirement

| Scenario | Direct evidence |
|---|---|
| Archive is reversible | `archive_is_reversible_and_preserves_lane_identity`, `archive_http_routes_are_reversible`, and `archived_lane_is_not_execution_eligible`. |
| Remove disposes lane-private state | `completed_removal_discards_private_uppers_and_generation_bindings` plus `lane_retirement_removes_provider_objects_even_after_runtime_was_stopped`. |
| Purge erases the compact tombstone | `purge_requires_force_and_exact_lane_id_then_erases_tombstone`. |
| Process exits during removal | `killing_lane_removal_at_every_durable_phase_recovers_to_completion` and `open_recovers_an_interrupted_binding_retirement_to_completion`. |
| Cleanup cannot complete | `cleanup_failure_records_repair_phase_and_resumes_from_exact_cut` verifies the structured repair phase, exact resume cut, confinement, and convergence. |
| Layer is unique to removed lane | `removal_makes_unique_layers_collectable_but_preserves_shared_layers` and the native Node remove/GC acceptance. |
| Layer is shared with another lane | The same integration and native acceptance verify GC preserves the layer while another lane or child references it. |
| Respawn after completed removal | `successful_removal_deletes_initialization_and_allows_name_reuse`. |
| Respawn during incomplete removal | `cleanup_failure_records_repair_phase_and_resumes_from_exact_cut` verifies spawn first retries retirement, returns the retirement's structured committed-repair identity when cleanup still cannot converge, and creates no second lane. `open_recovers_an_interrupted_binding_retirement_to_completion` verifies automatic convergence. |
| Inspect removed lane by identity | `retirement_report_serializes_stable_kind_phase_and_compact_provenance` and the completed-removal integration verify the retained identity, roots, generation IDs, byte accounting, timestamps, and force state. |

## Managed execution lifecycle

| Scenario | Direct evidence |
|---|---|
| Managed execution succeeds | `lane_exec_runs_the_ordered_managed_lifecycle_and_checkpoints_only_source`, `lane_test_uses_managed_lifecycle_and_checkpoints_after_command_failure`, `terminal_agent_uses_managed_lifecycle_and_returns_its_receipt`, `turn_envelope_acp_prompt_finish_checkpoint`, and the managed NFS exec acceptance cover exec, test/eval, terminal-agent, and ACP surfaces. |
| Preparation fails | `managed_preparation_failure_never_launches_the_command` verifies a failed sync prevents launch and records the failed phase. |
| Command fails | `lane_test_uses_managed_lifecycle_and_checkpoints_after_command_failure` and `missing_gate_program_still_finalizes_checkpoint_disposal_and_unmount`. |
| Source and generated files both change | `lane_exec_runs_the_ordered_managed_lifecycle_and_checkpoints_only_source` verifies source inclusion, generated exclusion, and generated dirty-path accounting. Native Node and Cargo acceptance also verify unchanged Git/source state while disposable output remains private. |
| Command and cleanup both fail | `command_and_cleanup_failures_are_both_retained_in_lifecycle_receipt`. |
| Inspect completed execution | The managed-execution integrations assert the ordered durable `managed_execution_phase` events and returned lifecycle receipts. |
| Interruption and checkpoint failure | `acp_relay_shutdown_finalizes_managed_prompt_and_checkpoints_source` verifies interruption checkpoint/dispose/unmount and a resumable identity; `acp_checkpoint_failure_is_durable_and_not_no_changes` verifies durable checkpoint failure reporting. |

## Native framework acceptance

- Node: NFS-COW layer reuse, private generated writes, child fork inheritance, fresh uppers, parent/peer removal, and final GC all pass.
- Cargo: two lanes reuse one target layer; producer-private bytes remain isolated; `cargo clean` cannot alter the shared layer or peer lane.
- Vite: a real optimized build passes over a 410-entry, approximately 37 MB layer; the second lane remains clean.
- Next.js 16: a real Turbopack build passes over a 10,560-entry, approximately 305 MB logical layer; the second lane remains clean.
- Runtime cleanup: provider objects are removed even when the runtime was already stopped.

## Suite results

- Library: 770 passed, 1 ignored.
- Lane initialization: 18 passed; initialization fault recovery: 28 passed.
- Retirement: 10 passed; inheritance: 2 passed; managed execution: 5 passed.
- Schema v18/v20/v21: 25 passed. Schema-v19: 12 passed, with two legacy-binary compatibility bodies explicitly skipped when the untracked pinned schema-v18 executable is absent.
- ACP integration group: 38 passed; two optional external-peer probes reported unavailable.
- E2E: 220 passed. The initial run exposed ACP finalization contention and map-inspector configuration defects; the final full run is green after both repairs.
