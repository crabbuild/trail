#![cfg(debug_assertions)]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use trail::Actor;
use trail::{InitImportMode, LaneWorkdirMode, Trail};

static ACTIVATION_STATE: OnceLock<Mutex<()>> = OnceLock::new();

fn serial() -> MutexGuard<'static, ()> {
    ACTIVATION_STATE
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

fn retry_native_observer_start<T>(
    mut operation: impl FnMut() -> trail::Result<T>,
) -> trail::Result<T> {
    const MAX_ATTEMPTS: usize = 5;
    for attempt in 1..=MAX_ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt < MAX_ATTEMPTS && retryable_native_observer_start(&error) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded native observer retry loop always returns")
}

fn retryable_native_observer_start(error: &trail::Error) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    match error {
        trail::Error::ChangeLedgerReconcileRequired { reason, .. } => {
            matches!(
                reason.as_str(),
                "fsevents_must_scan_subdirs" | "fsevents_null_callback_context_generation_changed"
            )
        }
        trail::Error::DaemonUnavailable(message) => {
            message.contains("fsevents_must_scan_subdirs")
                || message.contains("fsevents_null_callback_context_generation_changed")
        }
        _ => false,
    }
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
        "af2cca0566976a6d6f6cea00e99fe5089c91e357ca1d0a50fd5397edcda32833"
    );
    assert_eq!(
        complete["raw_mutation_inventory_sha256"],
        "50fbeb5ffd0fc0c755d6a006b62ef442532d6b70e0899931117125e39c8945dd"
    );
    assert_eq!(
        complete["activation_audit_sha256"],
        "acf0bad41d21c0abce86a958c3015cda09ed6f126fd1403e7abc67c4251b589e"
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
    let _guard = serial();
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
    let _guard = serial();
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
    let result = retry_native_observer_start(|| db.status(None));
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
    let result = retry_native_observer_start(|| db.status(None));
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
        let status = retry_native_observer_start(|| db.status(None))?;
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
fn authoritative_materialized_lane_can_preview_and_run_a_queued_merge_after_recording() {
    let _guard = serial();
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("tracked.txt"), b"base\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = match db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "merge-bot",
        Some("main"),
        LaneWorkdirMode::NativeCow,
        None,
        None,
        None,
        &[],
        false,
    ) {
        Ok(spawned) => spawned,
        Err(trail::Error::CloneUnsupported) => return,
        Err(error) => panic!("native COW lane setup failed: {error}"),
    };
    let workdir = spawned.workdir.unwrap();

    trail::test_support::set_changed_path_authority_override(true);
    let result = (|| {
        retry_native_observer_start(|| db.lane_status("merge-bot"))?;
        fs::write(Path::new(&workdir).join("tracked.txt"), b"lane edit\n")?;
        db.record_lane_workdir("merge-bot", Some("record lane edit".into()))?;
        db.agent_mark_reviewed("merge-bot", Some("review recorded edit".into()))?;

        let preview = db.merge_lane_user_with_options("merge-bot", "main", true, false)?;
        let queued = db.enqueue_lane_merge("merge-bot", "main", 0)?;
        let explain = db.explain_lane_merge_queue(&queued.entry.queue_id)?;
        let run = db.run_lane_merge_queue(None)?;
        let removed = db.remove_lane("merge-bot", false)?;
        let fsck = db.fsck()?;
        Ok::<_, trail::Error>((preview, explain, run, removed, fsck))
    })();
    trail::test_support::set_changed_path_authority_override(false);

    let (preview, explain, run, removed, fsck) = result.unwrap();
    assert_eq!(preview.changed_paths.len(), 1);
    assert!(explain.blockers.is_empty());
    assert_eq!(run.processed.len(), 1);
    assert_eq!(run.processed[0].status, "merged");
    assert_eq!(removed.lane_id, run.processed[0].lane_id);
    assert!(fsck.errors.is_empty());
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
    retry_native_observer_start(|| first.status(None)).unwrap();
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
    retry_native_observer_start(|| db.status(None)).unwrap();
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
