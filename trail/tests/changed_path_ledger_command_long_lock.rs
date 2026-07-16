#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn observed_record_lock_longer_than_old_timeout_preserves_observer_authority() {
    trail::test_support::changed_path_command_long_lock_flow().unwrap();
}
