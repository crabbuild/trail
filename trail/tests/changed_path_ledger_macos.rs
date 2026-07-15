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

#[test]
fn fence_cursor_never_overclaims_a_paused_callback_batch() {
    trail::test_support::changed_path_macos_paused_callback_fence().unwrap();
}

#[test]
fn resume_is_bound_to_actual_device_history_and_relative_root() {
    trail::test_support::changed_path_macos_history_authority().unwrap();
}

#[test]
fn late_native_start_is_cancelled_without_blocking_timeout_return() {
    trail::test_support::changed_path_macos_startup_cancellation().unwrap();
}

#[test]
fn malformed_callback_arrays_revoke_but_zero_count_is_safe() {
    trail::test_support::changed_path_macos_malformed_callbacks().unwrap();
}

#[test]
fn every_root_revalidation_failure_revokes_globally() {
    trail::test_support::changed_path_macos_root_revalidation_failures().unwrap();
}

#[test]
fn null_context_generation_revokes_every_live_proof_path() {
    trail::test_support::changed_path_macos_null_context_generation().unwrap();
}

#[test]
fn database_uuid_is_revalidated_after_start_and_before_proof() {
    trail::test_support::changed_path_macos_uuid_revalidation().unwrap();
}
