#![cfg(target_os = "linux")]

#[test]
fn recursive_directory_creation_is_covered_before_children_can_be_clean() {
    trail::test_support::changed_path_linux_recursive_coverage().unwrap();
}

#[test]
fn reconciliation_can_qualify_only_through_a_durable_end_fence() {
    trail::test_support::changed_path_linux_reconciliation_interval_qualification().unwrap();
}

#[test]
fn content_mode_create_and_delete_are_durably_observed() {
    trail::test_support::changed_path_linux_content_mode_create_delete().unwrap();
}

#[test]
fn file_directory_and_case_renames_retain_both_endpoints() {
    trail::test_support::changed_path_linux_rename_matrix().unwrap();
}

#[test]
fn rename_storms_and_expired_cookies_remain_conservative() {
    trail::test_support::changed_path_linux_rename_storm_and_cookie_expiry().unwrap();
}

#[test]
fn delayed_backlog_is_drained_before_the_fence_returns() {
    trail::test_support::changed_path_linux_delayed_backlog().unwrap();
}

#[test]
fn nonce_fence_orders_durable_create_before_durable_delete() {
    trail::test_support::changed_path_linux_fence_ordering().unwrap();
}

#[test]
fn overflow_ignored_unknown_decode_watch_add_and_durability_fail_closed() {
    trail::test_support::changed_path_linux_fault_revocation_matrix().unwrap();
}

#[test]
fn owner_death_and_root_replacement_cannot_prove_clean() {
    trail::test_support::changed_path_linux_owner_death_and_root_replacement().unwrap();
}

#[test]
fn linux_observer_process_owner_child() {
    let Ok(root) = std::env::var("TRAIL_LINUX_OBSERVER_CHILD_ROOT") else {
        return;
    };
    trail::test_support::changed_path_linux_process_owner_child(&root).unwrap();
}
