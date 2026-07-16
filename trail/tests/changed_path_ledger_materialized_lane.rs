#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn materialized_lane_snapshot_pins_lane_root_and_reconciles_missing_marker() {
    trail::test_support::changed_path_materialized_lane_snapshot_flow().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn materialized_candidate_lifecycle_is_exact_and_does_not_accumulate_internal_paths() {
    trail::test_support::changed_path_materialized_candidate_lifecycle_flow().unwrap();
}
