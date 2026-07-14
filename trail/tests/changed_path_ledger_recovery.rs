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
