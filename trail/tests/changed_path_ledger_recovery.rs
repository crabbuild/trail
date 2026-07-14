#[test]
fn concurrent_same_path_event_survives_intent_acknowledgement() {
    trail::test_support::changed_path_intent_acknowledgement_race().unwrap();
}

#[test]
fn prepared_target_is_a_gc_root_until_terminal_recovery() {
    trail::test_support::changed_path_intent_gc_root_lifecycle().unwrap();
}

#[test]
fn deterministic_crash_boundaries_recover_without_false_clean_results() {
    trail::test_support::changed_path_intent_crash_matrix().unwrap();
}

#[test]
fn backup_and_restore_never_transfer_trusted_filesystem_identity() {
    trail::test_support::changed_path_backup_restore_rotation().unwrap();
}

#[test]
fn unqualified_or_stale_filesystem_proof_cannot_publish_a_baseline() {
    trail::test_support::changed_path_qualified_proof_revalidation().unwrap();
}

#[test]
fn ambiguous_recovery_requires_reconciliation_before_another_intent() {
    trail::test_support::changed_path_ambiguous_recovery_gate().unwrap();
}

#[test]
fn failed_backup_overwrite_retains_the_previous_valid_tree() {
    trail::test_support::changed_path_backup_overwrite_rollback().unwrap();
}

#[test]
fn retirement_validates_paths_and_waits_for_preexisting_readers() {
    trail::test_support::changed_path_retirement_barrier().unwrap();
}

#[test]
fn lane_deletion_retires_changed_path_scope_before_filesystem_removal() {
    trail::test_support::changed_path_lane_deletion_retirement().unwrap();
}

#[test]
fn metadata_only_intent_proof_without_a_real_sidecar_is_rejected() {
    trail::test_support::changed_path_missing_sidecar_rejection().unwrap();
}

#[test]
fn authenticated_intent_cut_remains_a_prefix_after_later_observer_advance() {
    trail::test_support::changed_path_advanced_prefix_recovery().unwrap();
}
