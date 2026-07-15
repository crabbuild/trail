#![cfg(target_os = "macos")]

#[test]
fn real_apfs_file_events_are_durable_and_fenced() {
    trail::test_support::changed_path_macos_real_apfs_file_events().unwrap();
}

#[test]
fn all_fsevents_gap_flags_revoke_cursor_resume() {
    trail::test_support::changed_path_macos_gap_flag_matrix().unwrap();
}

#[test]
fn fsevents_restart_root_cursor_overflow_and_worker_death_fail_closed() {
    trail::test_support::changed_path_macos_continuity_fault_matrix().unwrap();
}

#[test]
fn synchronous_flush_orders_callbacks_through_durable_event_id() {
    trail::test_support::changed_path_macos_fence_ordering().unwrap();
}
