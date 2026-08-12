use std::fs;
use std::path::PathBuf;
use std::process::Command;

use trail::{InitImportMode, LaneWorkdirMode, Trail};

#[test]
fn lane_exec_runs_the_ordered_managed_lifecycle_and_checkpoints_only_source() {
    if !layered_acceptance_enabled() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "managed",
        Some("main"),
        layered_mode(),
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    let report = db
        .exec_lane_workspace(
            "managed",
            &[
                "/bin/sh".into(),
                "-c".into(),
                "printf durable > source.txt; mkdir -p target; printf disposable > target/build.bin; exit 7"
                    .into(),
            ],
        )
        .unwrap();

    assert_eq!(report.exit_code, 7);
    assert_eq!(report.lifecycle.surface, "lane_exec");
    let checkpoint = report
        .lifecycle
        .checkpoint
        .as_ref()
        .expect("command failure must still checkpoint source changes");
    assert!(checkpoint
        .source_paths
        .iter()
        .any(|path| path == "source.txt"));
    assert!(checkpoint
        .source_paths
        .iter()
        .all(|path| !path.starts_with("target/")));
    assert!(checkpoint.generated_dirty_paths > 0);

    let mut phases = db
        .list_lane_events(
            Some("managed"),
            None,
            None,
            Some("managed_execution_phase"),
            100,
        )
        .unwrap();
    phases.reverse();
    let phases = phases
        .into_iter()
        .filter_map(|event| {
            event
                .payload
                .and_then(|payload| payload["phase"].as_str().map(str::to_string))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [
            "resolve",
            "discover_plan",
            "sync_all",
            "reconcile",
            "mount",
            "execute",
            "checkpoint",
            "dispose",
            "unmount",
        ]
    );
}

#[test]
fn lane_test_uses_managed_lifecycle_and_checkpoints_after_command_failure() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "managed-gate",
        Some("main"),
        LaneWorkdirMode::PortableCopy,
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    let report = db
        .run_lane_test(
            "managed-gate",
            vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf durable > gate-source.txt; exit 9".into(),
            ],
            None,
            30,
        )
        .unwrap();

    assert!(!report.success);
    assert_eq!(report.exit_code, Some(9));
    assert_eq!(report.lifecycle.surface, "lane_test");
    let preparation = report.lifecycle.preparation.as_ref().unwrap();
    assert_eq!(
        serde_json::to_value(preparation.missing_resolution_policy).unwrap(),
        "explicit"
    );
    assert!(preparation.resolution_pins.is_empty());
    assert!(preparation.output_pins.is_empty());
    assert!(report
        .lifecycle
        .checkpoint
        .as_ref()
        .unwrap()
        .source_paths
        .iter()
        .any(|path| path == "gate-source.txt"));
    assert_eq!(
        report
            .lifecycle
            .phases
            .iter()
            .map(|receipt| receipt.phase.as_str())
            .collect::<Vec<_>>(),
        [
            "resolve",
            "discover_plan",
            "sync_all",
            "prefetch",
            "reconcile",
            "mount",
            "execute",
            "checkpoint",
            "dispose",
            "unmount",
        ]
    );
    let finalization = report.lifecycle.finalization.as_ref().unwrap();
    assert!(finalization.complete);
    assert!(finalization.source_changed);
    assert_eq!(finalization.checkpoint_status, "succeeded");
    assert_eq!(finalization.disposal_status, "skipped");
    assert_eq!(finalization.unmount_status, "skipped");
}

#[test]
fn root_managed_preparation_ignores_unrelated_nested_environment_components() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    fs::create_dir_all(root.path().join("nested")).unwrap();
    fs::write(
        root.path().join("nested/package.json"),
        r#"{"name":"nested-only","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("nested/package-lock.json"),
        r#"{"name":"nested-only","version":"1.0.0","lockfileVersion":3,"packages":{"":{"name":"nested-only","version":"1.0.0"}}}"#,
    )
    .unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "root-only",
        Some("main"),
        LaneWorkdirMode::PortableCopy,
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    let context = db
        .prepare_managed_lane_execution(
            "root-only",
            "lane_exec",
            &["/bin/sh".into(), "-c".into(), "printf root".into()],
        )
        .unwrap();
    assert_eq!(context.lane, "root-only");
    assert!(context.environment_generation.is_none());
}

#[test]
fn missing_gate_program_still_finalizes_checkpoint_disposal_and_unmount() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "managed-launch-failure",
        Some("main"),
        LaneWorkdirMode::PortableCopy,
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    let report = db
        .run_lane_eval(
            "managed-launch-failure",
            vec!["trail-command-that-does-not-exist".into()],
            None,
            30,
        )
        .unwrap();
    assert!(!report.success);
    assert!(report.stderr_preview.contains("No such file"));

    let mut events = db
        .list_lane_events(
            Some("managed-launch-failure"),
            None,
            None,
            Some("managed_execution_phase"),
            100,
        )
        .unwrap();
    events.reverse();
    let phases = events
        .into_iter()
        .map(|event| {
            let payload = event.payload.unwrap();
            (
                payload["phase"].as_str().unwrap().to_string(),
                payload["status"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert!(phases.contains(&("execute".into(), "failed".into())));
    assert!(phases.iter().any(|(phase, _)| phase == "checkpoint"));
    assert!(phases.iter().any(|(phase, _)| phase == "dispose"));
    assert!(phases.iter().any(|(phase, _)| phase == "unmount"));
}

#[test]
fn managed_preparation_failure_never_launches_the_command() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"managed-preparation","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("package-lock.json"),
        r#"{"name":"managed-preparation","version":"1.0.0","lockfileVersion":3,"packages":{"":{"name":"managed-preparation","version":"1.0.0"}}}"#,
    )
    .unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "managed-preparation-failure",
        Some("main"),
        LaneWorkdirMode::PortableCopy,
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();
    let workdir = PathBuf::from(
        db.lane_details("managed-preparation-failure")
            .unwrap()
            .branch
            .workdir
            .unwrap(),
    );

    let error = db
        .run_lane_test(
            "managed-preparation-failure",
            vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf should-not-run > PREPARATION_RAN".into(),
            ],
            None,
            30,
        )
        .unwrap_err();
    assert!(error.to_string().contains("uses a materialized workdir"));
    assert!(error.to_string().contains("--workdir-mode auto"));
    assert!(!workdir.join("PREPARATION_RAN").exists());

    let phases = db
        .list_lane_events(
            Some("managed-preparation-failure"),
            None,
            None,
            Some("managed_execution_phase"),
            100,
        )
        .unwrap();
    assert!(phases.iter().any(|event| {
        event.payload.as_ref().is_some_and(|payload| {
            payload["phase"] == "discover_plan" && payload["status"] == "failed"
        })
    }));
    assert!(!phases.iter().any(|event| {
        event
            .payload
            .as_ref()
            .is_some_and(|payload| payload["phase"] == "execute")
    }));
}

#[test]
fn terminal_agent_uses_managed_lifecycle_and_returns_its_receipt() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = Command::new(trail_bin())
        .args(["--workspace"])
        .arg(root.path())
        .args([
            "--json",
            "agent",
            "start",
            "codex",
            "--name",
            "managed terminal",
            "--workdir-mode",
            "portable-copy",
            "--",
            "/bin/sh",
            "-c",
            "printf agent-source > AGENT.md",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "terminal agent failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "completed");
    assert_eq!(report["lifecycle"]["surface"], "terminal_agent");
    assert_eq!(
        report["lifecycle"]["preparation"]["missing_resolution_policy"],
        "explicit"
    );
    assert!(report["lifecycle"]["preparation"]["resolution_pins"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        report["lifecycle"]["finalization"]["checkpoint_status"],
        "succeeded"
    );
    assert_eq!(report["lifecycle"]["finalization"]["complete"], true);
    assert!(report["lifecycle"]["checkpoint"]["source_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "AGENT.md"));
}

fn layered_mode() -> LaneWorkdirMode {
    if cfg!(target_os = "macos") {
        LaneWorkdirMode::NfsCow
    } else if cfg!(target_os = "windows") {
        LaneWorkdirMode::DokanCow
    } else {
        LaneWorkdirMode::FuseCow
    }
}

fn layered_acceptance_enabled() -> bool {
    if cfg!(target_os = "macos") {
        std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_some()
    } else if cfg!(target_os = "windows") {
        std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").is_some()
    } else {
        std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").is_some()
    }
}

fn trail_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_trail")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/trail"))
}
