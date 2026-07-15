#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn status_and_record_use_one_continuous_fenced_candidate_flow() {
    trail::test_support::changed_path_command_flow().unwrap();
}
