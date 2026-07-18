#![cfg(debug_assertions)]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use trail::Actor;
use trail::{InitImportMode, Trail};

static ACTIVATION_STATE: OnceLock<Mutex<()>> = OnceLock::new();

fn serial() -> MutexGuard<'static, ()> {
    ACTIVATION_STATE
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn authority_requires_every_checked_gate_and_supported_platform() {
    let complete = trail::test_support::changed_path_activation_evidence().unwrap();
    for gate in [
        "schema_hard_cutover",
        "producer_inventory_complete",
        "linux_native_suite",
        "macos_native_suite",
        "crash_matrix",
        "corruption_matrix",
        "scale_gates",
        "metrics_jsonl",
        "exact_sha_tag_gate",
        "exact_sha_publish_gate",
    ] {
        assert_eq!(complete[gate], true, "activation gate `{gate}` is absent");
    }
    assert_eq!(
        complete["producer_inventory_sha256"],
        "a13fa0330d89ad442a4f796a5fd37b55177ab4fdf7805354925b99fc18199d0e"
    );
    assert_eq!(
        complete["raw_mutation_inventory_sha256"],
        "cc7ee1525ede98b278c347e79f10e635666b3034d758451e76afa2f5a2a64cf2"
    );
    assert_eq!(
        complete["activation_audit_sha256"],
        "4ab0b441355b9a7a55f53ead7317da77b314f5dc3afb0b9c51aae79a3b87e75e"
    );
    assert!(!trail::test_support::changed_path_authority_enabled_for("windows").unwrap());
    assert!(!trail::test_support::changed_path_authority_enabled_for("freebsd").unwrap());
    assert_eq!(
        trail::test_support::changed_path_production_authority_default(),
        cfg!(any(target_os = "linux", target_os = "macos"))
    );
}

#[test]
fn recovery_corruption_and_native_fault_matrix_remains_fail_closed() {
    trail::test_support::changed_path_intent_crash_matrix().unwrap();
    trail::test_support::changed_path_qualified_proof_revalidation().unwrap();
    trail::test_support::changed_path_missing_sidecar_rejection().unwrap();
    trail::test_support::changed_path_ambiguous_recovery_gate().unwrap();
    trail::test_support::changed_path_backup_restore_rotation().unwrap();

    #[cfg(target_os = "linux")]
    {
        trail::test_support::changed_path_linux_fault_revocation_matrix().unwrap();
        trail::test_support::changed_path_linux_raw_decoder_faults().unwrap();
        trail::test_support::changed_path_linux_owner_death_and_root_replacement().unwrap();
        trail::test_support::changed_path_linux_unsupported_filesystem_rejection().unwrap();
    }
    #[cfg(target_os = "macos")]
    {
        trail::test_support::changed_path_macos_continuity_fault_matrix().unwrap();
        trail::test_support::changed_path_macos_gap_flag_matrix().unwrap();
        trail::test_support::changed_path_macos_malformed_callbacks().unwrap();
        trail::test_support::changed_path_macos_root_revalidation_failures().unwrap();
        trail::test_support::changed_path_macos_unsupported_filesystem_rejection().unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_observer_process_owner_child() {
    let Ok(root) = std::env::var("TRAIL_LINUX_OBSERVER_CHILD_ROOT") else {
        return;
    };
    trail::test_support::changed_path_linux_process_owner_child(&root).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn fsevents_restart_root_cursor_overflow_and_worker_death_fail_closed() {
    if std::env::var_os("TRAIL_MACOS_OBSERVER_OWNER_CHILD_ROOT").is_some() {
        trail::test_support::changed_path_macos_continuity_fault_matrix().unwrap();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn first_authoritative_status_starts_and_reconciles_the_workspace_daemon() {
    let _guard = serial();
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "--quiet"]);
    git(temp.path(), &["config", "user.name", "Trail Activation"]);
    git(
        temp.path(),
        &["config", "user.email", "trail-activation@example.invalid"],
    );
    fs::write(temp.path().join("tracked.txt"), b"base\n").unwrap();
    fs::write(temp.path().join(".gitignore"), b".trail/\n").unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "--quiet", "-m", "base"]);
    Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    let db = Trail::open(temp.path()).unwrap();
    fs::write(temp.path().join("tracked.txt"), b"changed\n").unwrap();

    trail::test_support::set_changed_path_authority_override(true);
    let result = db.status(None);
    trail::test_support::set_changed_path_authority_override(false);
    let report = result.unwrap();
    assert!(
        report
            .changed_paths
            .iter()
            .any(|change| change.path == "tracked.txt"),
        "automatic reconciliation omitted the pre-start change"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn tracked_gitignored_file_remains_clean_after_git_import() {
    let _guard = serial();
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "--quiet"]);
    git(temp.path(), &["config", "user.name", "Trail Activation"]);
    git(
        temp.path(),
        &["config", "user.email", "trail-activation@example.invalid"],
    );
    fs::create_dir(temp.path().join("generated")).unwrap();
    fs::write(temp.path().join("generated/tracked.txt"), b"tracked\n").unwrap();
    fs::write(temp.path().join(".gitignore"), b".trail/\ngenerated/\n").unwrap();
    git(temp.path(), &["add", ".gitignore"]);
    git(temp.path(), &["add", "--force", "generated/tracked.txt"]);
    git(temp.path(), &["commit", "--quiet", "-m", "base"]);
    Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();

    trail::test_support::set_changed_path_authority_override(true);
    let result = db.status(None);
    trail::test_support::set_changed_path_authority_override(false);

    let status = result.unwrap();
    assert!(
        status.changed_paths.is_empty(),
        "clean Git-tracked ignored files must remain visible to Trail: {:?}",
        status.changed_paths
    );
    assert!(db.diff_dirty(false, false).unwrap().files.is_empty());
    assert!(db
        .record(
            None,
            Some("clean tracked-ignore record".into()),
            Actor::human(),
            false,
        )
        .unwrap()
        .operation
        .is_none());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn activated_non_git_workspace_uses_ledger_without_git_qualification() {
    let _guard = serial();
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("tracked.txt"), b"base\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    fs::write(temp.path().join("tracked.txt"), b"changed\n").unwrap();

    trail::test_support::set_changed_path_authority_override(true);
    let result = (|| {
        let status = db.status(None)?;
        let diff = db.diff_dirty(false, false)?;
        let record = db.record(
            Some("main"),
            Some("activated non-git record".into()),
            Actor::human(),
            false,
        )?;
        Ok::<_, trail::Error>((status, diff, record))
    })();
    trail::test_support::set_changed_path_authority_override(false);
    let (status, diff, record) = result.unwrap();
    assert!(status
        .changed_paths
        .iter()
        .any(|path| path.path == "tracked.txt"));
    assert!(diff.files.iter().any(|path| path.path == "tracked.txt"));
    assert!(record
        .changed_paths
        .iter()
        .any(|path| path.path == "tracked.txt"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn second_direct_handle_cannot_evict_a_live_workspace_observer() {
    let _guard = serial();
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("tracked.txt"), b"base\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let first = Trail::open(temp.path()).unwrap();

    trail::test_support::set_changed_path_authority_override(true);
    first.status(None).unwrap();
    let second = Trail::open(temp.path()).unwrap();
    let error = second.status(None).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("observer owner is still live; refusing unverified authority replacement"),
        "second handle failed for the wrong reason: {error}"
    );
    fs::write(temp.path().join("tracked.txt"), b"changed\n").unwrap();
    let report = first.status(None);
    trail::test_support::set_changed_path_authority_override(false);
    assert!(report
        .unwrap()
        .changed_paths
        .iter()
        .any(|path| path.path == "tracked.txt"));
}

#[test]
fn performance_metrics_file_emits_complete_append_only_jsonl() {
    let _guard = serial();
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("tracked.txt"), b"base\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let metrics = temp.path().join("operation-metrics.jsonl");
    // This test owns the process-global activation lock for the full lifetime
    // of the environment mutation and opened Trail handle.
    unsafe { std::env::set_var("TRAIL_PERFORMANCE_METRICS_FILE", &metrics) };
    let db = Trail::open(temp.path()).unwrap();
    db.status(None).unwrap();
    let _ = db.diff_range("invalid", false);
    unsafe { std::env::remove_var("TRAIL_PERFORMANCE_METRICS_FILE") };

    let lines = fs::read_to_string(metrics).unwrap();
    let reports = lines
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        reports.len(),
        2,
        "one JSON object is required per operation"
    );
    assert_eq!(reports[0]["operation"], "status");
    assert_eq!(reports[0]["outcome"], "success");
    assert_eq!(reports[1]["operation"], "diff");
    assert_eq!(reports[1]["outcome"], "error");
    assert_eq!(reports[0]["generation"], 1);
    assert_eq!(reports[1]["generation"], 2);
    assert!(reports.iter().all(|report| report["wall_time_ns"].is_u64()));
}
