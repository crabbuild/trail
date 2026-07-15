#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn materialized_lane_snapshot_pins_lane_root_and_reconciles_missing_marker() {
    trail::test_support::changed_path_materialized_lane_snapshot_flow().unwrap();
}
