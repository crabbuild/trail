#![cfg(debug_assertions)]

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard, OnceLock};
use trail::{Actor, InitImportMode, Trail};

static GIT_QUALIFICATION_TESTS: OnceLock<Mutex<()>> = OnceLock::new();

struct Fixture {
    db: Trail,
    temp: tempfile::TempDir,
}

struct EnvironmentGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvironmentGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        // Every test in this binary holds `GIT_QUALIFICATION_TESTS`, so no
        // other thread in the process reads or writes this selector.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvironmentGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "--quiet"]);
        git(temp.path(), &["config", "user.name", "Trail Test"]);
        git(
            temp.path(),
            &["config", "user.email", "trail@example.invalid"],
        );
        git(temp.path(), &["config", "core.filemode", "true"]);
        git(temp.path(), &["config", "core.symlinks", "true"]);
        git(
            temp.path(),
            &[
                "config",
                "core.ignorecase",
                if filesystem_is_case_insensitive(temp.path()) {
                    "true"
                } else {
                    "false"
                },
            ],
        );
        fs::write(temp.path().join("tracked.txt"), b"base\n").unwrap();
        fs::write(temp.path().join("rename-me.txt"), b"rename\n").unwrap();
        fs::write(temp.path().join(".trailignore"), b"").unwrap();
        fs::write(temp.path().join(".gitignore"), b".trail/\n").unwrap();
        git(temp.path(), &["add", "."]);
        git(temp.path(), &["commit", "--quiet", "-m", "base"]);
        Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        trail::test_support::set_changed_path_authority_override(false);
        Self { db, temp }
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn qualify(&self) -> Value {
        self.qualify_with_policy_mismatch(false)
    }

    fn qualify_with_policy_mismatch(&self, mismatch: bool) -> Value {
        trail::test_support::changed_path_git_qualification(&self.db, mismatch).unwrap()
    }
}

fn serial() -> MutexGuard<'static, ()> {
    GIT_QUALIFICATION_TESTS
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

fn git(root: &Path, args: &[&str]) -> Output {
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
    output
}

fn git_text(root: &Path, args: &[&str]) -> String {
    String::from_utf8(git(root, args).stdout)
        .unwrap()
        .trim()
        .to_string()
}

fn qualification(value: &Value) -> &Value {
    &value["qualification"]
}

fn exact_paths(value: &Value) -> BTreeSet<String> {
    value["exact_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap().to_string())
        .collect()
}

fn reasons(value: &Value) -> BTreeSet<String> {
    qualification(value)["advisory_reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reason| reason.as_str().unwrap().to_string())
        .collect()
}

fn absolute_git_path(root: &Path, path: String) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn replace_regular_file(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let replacement = path.with_extension("trail-qualification-replacement");
    fs::write(&replacement, bytes).map_err(|error| error.to_string())?;
    fs::rename(replacement, path).map_err(|error| error.to_string())
}

fn filesystem_is_case_insensitive(root: &Path) -> bool {
    let lower = root.join(".trail-git-case-probe-a");
    let upper = root.join(".TRAIL-GIT-CASE-PROBE-A");
    fs::write(&lower, b"probe").unwrap();
    let insensitive = upper.exists();
    fs::remove_file(lower).unwrap();
    insensitive
}

#[test]
fn exact_equivalence_allows_clean_proof_and_reports_hidden_git_work() {
    let _guard = serial();
    let fixture = Fixture::new();
    let qualified = fixture.qualify();
    let qualification = qualification(&qualified);

    assert_eq!(qualification["clean_proof_allowed"], true);
    assert_eq!(
        qualification["mapped_trail_root"],
        qualification["ledger_baseline_root"]
    );
    assert!(exact_paths(&qualified).is_empty());
    assert!(qualified["metrics"]["subprocess_count"].as_u64().unwrap() > 0);
    assert!(qualified["metrics"]["trace2_bytes"].as_u64().unwrap() > 0);
    assert!(
        qualified["metrics"]["external_adapter_global_work"]
            .as_u64()
            .unwrap()
            > 0
    );
    // Includes before/after qualification, post-c2 path capture, and the
    // descriptor-held verification read returned to the caller.
    assert!(qualified["metrics"]["index_read_count"].as_u64().unwrap() >= 5);
    assert!(qualified["metrics"]["index_bytes"].as_u64().unwrap() > 0);
    assert_eq!(qualification["worktree_equivalent"], true);
    assert!(!qualification["worktree_top_level"]
        .as_str()
        .unwrap()
        .is_empty());

    // Qualification is testable before activation but must not flip command
    // authority on as a side effect.
    trail::test_support::set_changed_path_authority_override(false);
}

#[test]
fn trail_full_scan_policy_oracle_matches_nested_untracked_and_ignored_candidates() {
    let _guard = serial();
    let fixture = Fixture::new();
    fs::write(fixture.root().join("tracked.txt"), b"changed\n").unwrap();
    fs::create_dir(fixture.root().join("nested")).unwrap();
    fs::write(fixture.root().join("nested/untracked.txt"), b"new\n").unwrap();
    fs::write(fixture.root().join(".trailignore"), b"nested/ignored.log\n").unwrap();
    fs::write(fixture.root().join("nested/ignored.log"), b"ignored\n").unwrap();

    let qualified = fixture.qualify();
    let oracle = trail::test_support::changed_path_git_full_scan_oracle(&fixture.db)
        .unwrap()
        .into_iter()
        .collect::<BTreeSet<_>>();

    assert_eq!(exact_paths(&qualified), oracle);
    assert!(oracle.contains("nested/untracked.txt"));
    assert!(!oracle.contains("nested/ignored.log"));
    assert_eq!(qualification(&qualified)["clean_proof_allowed"], true);
}

#[test]
fn trail_ahead_reversion_and_policy_mismatch_are_advisory_only() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    fs::write(fixture.root().join("tracked.txt"), b"trail ahead\n").unwrap();
    fixture
        .db
        .record(
            Some("main"),
            Some("advance Trail baseline".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    git(fixture.root(), &["checkout", "--", "tracked.txt"]);

    let ahead = fixture.qualify();
    assert_eq!(qualification(&ahead)["clean_proof_allowed"], false);
    assert!(reasons(&ahead).contains("head_or_mapping"));

    let mismatch = fixture.qualify_with_policy_mismatch(true);
    assert_eq!(qualification(&mismatch)["clean_proof_allowed"], false);
    assert!(reasons(&mismatch).contains("policy_fingerprint"));
}

#[test]
fn porcelain_v2_retains_both_rename_endpoints() {
    let _guard = serial();
    let fixture = Fixture::new();
    git(fixture.root(), &["mv", "rename-me.txt", "renamed.txt"]);

    let qualified = fixture.qualify();
    assert!(exact_paths(&qualified).is_superset(&BTreeSet::from([
        "rename-me.txt".to_string(),
        "renamed.txt".to_string(),
    ])));
    assert!(qualified["rename_pairs"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!(["rename-me.txt", "renamed.txt"])));
}

#[cfg(unix)]
#[test]
fn mode_and_symlink_semantics_are_qualified_conservatively() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = serial();
    let fixture = Fixture::new();
    let tracked = fixture.root().join("tracked.txt");
    let mut permissions = fs::metadata(&tracked).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tracked, permissions).unwrap();
    let mode = fixture.qualify();
    assert!(exact_paths(&mode).contains("tracked.txt"));
    assert_eq!(qualification(&mode)["mode_equivalent"], true);

    let fixture = Fixture::new();
    let oid = git_text(fixture.root(), &["rev-parse", "HEAD:tracked.txt"]);
    git(
        fixture.root(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{oid},tracked-link"),
        ],
    );
    let symlink = fixture.qualify();
    assert_eq!(qualification(&symlink)["clean_proof_allowed"], false);
    assert!(reasons(&symlink).contains("symlink"));
}
#[test]
fn index_flags_sparse_submodules_and_caches_are_advisory_only() {
    let _guard = serial();

    let fixture = Fixture::new();
    git(
        fixture.root(),
        &["update-index", "--assume-unchanged", "tracked.txt"],
    );
    let assume = fixture.qualify();
    assert_eq!(qualification(&assume)["clean_proof_allowed"], false);
    assert!(reasons(&assume).contains("index_identity_or_flags"));

    let fixture = Fixture::new();
    git(
        fixture.root(),
        &["update-index", "--skip-worktree", "tracked.txt"],
    );
    let skip = fixture.qualify();
    assert_eq!(qualification(&skip)["clean_proof_allowed"], false);
    assert!(reasons(&skip).contains("sparse_or_skip_worktree"));

    let fixture = Fixture::new();
    git(fixture.root(), &["config", "core.sparseCheckout", "true"]);
    let sparse = fixture.qualify();
    assert_eq!(qualification(&sparse)["clean_proof_allowed"], false);
    assert!(reasons(&sparse).contains("sparse_or_skip_worktree"));

    let fixture = Fixture::new();
    let commit = git_text(fixture.root(), &["rev-parse", "HEAD"]);
    git(
        fixture.root(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{commit},nested-repository"),
        ],
    );
    let submodule = fixture.qualify();
    assert_eq!(qualification(&submodule)["clean_proof_allowed"], false);
    assert!(reasons(&submodule).contains("submodule"));

    let fixture = Fixture::new();
    git(fixture.root(), &["config", "core.fsmonitor", "/bin/true"]);
    let fsmonitor = fixture.qualify();
    assert_eq!(qualification(&fsmonitor)["clean_proof_allowed"], false);
    assert!(reasons(&fsmonitor).contains("fsmonitor"));

    let fixture = Fixture::new();
    git(fixture.root(), &["config", "core.untrackedCache", "true"]);
    let cache = fixture.qualify();
    assert_eq!(qualification(&cache)["clean_proof_allowed"], false);
    assert!(reasons(&cache).contains("untracked_cache"));
}

#[test]
fn main_and_shared_index_replacement_races_revoke_clean_proof() {
    let _guard = serial();

    let fixture = Fixture::new();
    let index = absolute_git_path(
        fixture.root(),
        git_text(fixture.root(), &["rev-parse", "--git-path", "index"]),
    );
    trail::test_support::install_git_qualification_after_porcelain_hook(move || {
        replace_regular_file(&index)
    });
    let replaced = fixture.qualify();
    assert_eq!(qualification(&replaced)["clean_proof_allowed"], false);
    assert!(reasons(&replaced).contains("index_identity_or_flags"));

    let fixture = Fixture::new();
    git(fixture.root(), &["update-index", "--split-index"]);
    let stable_split = fixture.qualify();
    assert!(
        stable_split["metrics"]["shared_index_read_count"]
            .as_u64()
            .unwrap()
            >= 2
    );
    assert!(
        stable_split["metrics"]["shared_index_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    let shared = absolute_git_path(
        fixture.root(),
        git_text(fixture.root(), &["rev-parse", "--shared-index-path"]),
    );
    assert!(shared.is_file());
    trail::test_support::install_git_qualification_after_porcelain_hook(move || {
        replace_regular_file(&shared)
    });
    match trail::test_support::changed_path_git_qualification(&fixture.db, false) {
        Ok(replaced) => {
            assert_eq!(qualification(&replaced)["clean_proof_allowed"], false);
            assert!(reasons(&replaced).contains("index_identity_or_flags"));
        }
        Err(error) => assert!(error.contains("shared_index"), "{error}"),
    }

    let fixture = Fixture::new();
    git(fixture.root(), &["update-index", "--split-index"]);
    let shared = absolute_git_path(
        fixture.root(),
        git_text(fixture.root(), &["rev-parse", "--shared-index-path"]),
    );
    trail::test_support::install_git_qualification_after_porcelain_hook(move || {
        fs::remove_file(shared).map_err(|error| error.to_string())
    });
    match trail::test_support::changed_path_git_qualification(&fixture.db, false) {
        Ok(missing) => {
            assert_eq!(qualification(&missing)["clean_proof_allowed"], false);
            assert!(reasons(&missing).contains("index_identity_or_flags"));
        }
        Err(error) => assert!(error.contains("shared_index"), "{error}"),
    }
}

#[test]
fn post_c2_index_replacement_fails_closed_before_consumption() {
    let _guard = serial();
    let fixture = Fixture::new();
    let index = absolute_git_path(
        fixture.root(),
        git_text(fixture.root(), &["rev-parse", "--git-path", "index"]),
    );
    trail::test_support::install_git_qualification_after_c2_hook(move || {
        replace_regular_file(&index)
    });
    let error =
        trail::test_support::changed_path_git_qualification(&fixture.db, false).unwrap_err();
    assert!(error.contains("changed across ledger c2"), "{error}");
}

#[test]
fn public_status_does_not_fall_back_when_git_index_disappears() {
    let _guard = serial();
    let fixture = Fixture::new();
    fixture.qualify();
    let index = absolute_git_path(
        fixture.root(),
        git_text(fixture.root(), &["rev-parse", "--git-path", "index"]),
    );
    trail::test_support::install_git_qualification_after_c2_hook(move || {
        fs::remove_file(index).map_err(|error| error.to_string())
    });
    trail::test_support::set_changed_path_authority_override(true);
    let result = fixture.db.status(None);
    trail::test_support::set_changed_path_authority_override(false);
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("index") || error.contains("changed across ledger c2"),
        "{error}"
    );
}

#[test]
fn public_status_fails_closed_on_git_index_replacement_aba() {
    let _guard = serial();
    let fixture = Fixture::new();
    fixture.qualify();
    let index = absolute_git_path(
        fixture.root(),
        git_text(fixture.root(), &["rev-parse", "--git-path", "index"]),
    );
    trail::test_support::install_git_qualification_after_c2_hook(move || {
        replace_regular_file(&index)
    });
    trail::test_support::set_changed_path_authority_override(true);
    let result = fixture.db.status(None);
    trail::test_support::set_changed_path_authority_override(false);
    let error = result.unwrap_err().to_string();
    assert!(error.contains("changed across ledger c2"), "{error}");
}

#[test]
fn post_c2_head_symbolic_ref_and_packed_refs_replacements_fail_closed() {
    let _guard = serial();

    for structural_path in ["HEAD", "symbolic-ref"] {
        let fixture = Fixture::new();
        let path = if structural_path == "HEAD" {
            absolute_git_path(
                fixture.root(),
                git_text(fixture.root(), &["rev-parse", "--git-path", "HEAD"]),
            )
        } else {
            let reference = git_text(fixture.root(), &["symbolic-ref", "HEAD"]);
            absolute_git_path(
                fixture.root(),
                git_text(
                    fixture.root(),
                    &["rev-parse", "--git-path", reference.as_str()],
                ),
            )
        };
        trail::test_support::install_git_qualification_after_c2_hook(move || {
            replace_regular_file(&path)
        });
        let error =
            trail::test_support::changed_path_git_qualification(&fixture.db, false).unwrap_err();
        assert!(error.contains("changed across ledger c2"), "{error}");
    }

    let fixture = Fixture::new();
    git(fixture.root(), &["pack-refs", "--all"]);
    let packed_refs = absolute_git_path(
        fixture.root(),
        git_text(fixture.root(), &["rev-parse", "--git-path", "packed-refs"]),
    );
    trail::test_support::install_git_qualification_after_c2_hook(move || {
        replace_regular_file(&packed_refs)
    });
    let error =
        trail::test_support::changed_path_git_qualification(&fixture.db, false).unwrap_err();
    assert!(error.contains("changed across ledger c2"), "{error}");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn post_c2_worktree_root_replacement_fails_closed() {
    let _guard = serial();
    let fixture = Fixture::new();
    let root = fixture.root().to_path_buf();
    let backup = root.with_extension("trail-original-worktree");
    let hook_root = root.clone();
    let hook_backup = backup.clone();
    trail::test_support::install_git_qualification_after_c2_hook(move || {
        fs::rename(&hook_root, &hook_backup).map_err(|error| error.to_string())?;
        let output = Command::new("cp")
            .args(["-R"])
            .arg(&hook_backup)
            .arg(&hook_root)
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    });
    let result = trail::test_support::changed_path_git_qualification(&fixture.db, false);
    fs::remove_dir_all(&root).unwrap();
    fs::rename(&backup, &root).unwrap();
    let error = result.unwrap_err();
    assert!(error.contains("changed across ledger c2"), "{error}");
}

#[test]
fn ambient_git_worktree_selector_cannot_redirect_qualification() {
    let _guard = serial();
    let fixture = Fixture::new();
    let hostile = tempfile::tempdir().unwrap();
    let _environment = EnvironmentGuard::set("GIT_WORK_TREE", hostile.path());

    let qualified = fixture.qualify();
    assert_eq!(qualification(&qualified)["worktree_equivalent"], true);
    assert_eq!(qualification(&qualified)["clean_proof_allowed"], true);
}

#[test]
fn status_diff_and_record_consume_ledger_paths_under_bounded_git_fence() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    fs::create_dir(fixture.root().join("nested-command")).unwrap();
    fs::write(
        fixture.root().join("nested-command/untracked.txt"),
        b"command flow\n",
    )
    .unwrap();

    let flow = trail::test_support::changed_path_git_command_flow(&mut fixture.db).unwrap();
    for surface in ["status", "diff", "record"] {
        assert!(flow[surface]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "nested-command/untracked.txt"));
    }
    trail::test_support::set_changed_path_authority_override(false);
}

#[test]
fn git_backed_public_commands_emit_zero_global_adapter_work() {
    let _guard = serial();
    let metrics_temp = tempfile::tempdir().unwrap();
    let metrics_path = metrics_temp.path().join("git-command-metrics.jsonl");
    let _metrics_environment =
        EnvironmentGuard::set("TRAIL_PERFORMANCE_METRICS_FILE", &metrics_path);
    let mut fixture = Fixture::new();
    fs::create_dir(fixture.root().join("metrics-command")).unwrap();
    fs::write(
        fixture.root().join("metrics-command/untracked.txt"),
        b"bounded command flow\n",
    )
    .unwrap();

    trail::test_support::changed_path_git_command_flow(&mut fixture.db).unwrap();

    let reports = fs::read_to_string(&metrics_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|report| {
            matches!(
                report["operation"].as_str(),
                Some("status" | "diff" | "record")
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reports.len(),
        3,
        "expected one public report for status, diff, and record: {reports:?}"
    );
    for report in reports {
        assert_eq!(report["outcome"], "success", "{report}");
        assert_eq!(report["external_adapter_global_work"], 0, "{report}");
        assert_eq!(report["git_global_work_count"], 0, "{report}");
        assert_eq!(report["git_index_read_count"], 0, "{report}");
        assert_eq!(report["git_index_bytes"], 0, "{report}");
        assert_eq!(report["git_shared_index_read_count"], 0, "{report}");
        assert_eq!(report["git_shared_index_bytes"], 0, "{report}");
    }
    trail::test_support::set_changed_path_authority_override(false);
}

#[test]
fn authoritative_dirty_diff_materializes_patches_and_line_changes() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    fs::write(
        fixture.root().join("tracked.txt"),
        b"base changed\nsecond line\n",
    )
    .unwrap();
    // Establish observer ownership and reconcile the pre-existing mutation.
    fixture.qualify();
    trail::test_support::set_changed_path_authority_override(true);
    let result = fixture.db.diff_dirty(true, true);
    trail::test_support::set_changed_path_authority_override(false);
    let diff = result.unwrap();
    let tracked = diff
        .files
        .iter()
        .find(|file| file.path == "tracked.txt")
        .expect("authoritative dirty diff omitted tracked.txt");
    let patch = tracked
        .patch
        .as_deref()
        .expect("authoritative dirty diff omitted its patch");
    assert!(patch.contains("-base"), "{patch}");
    assert!(patch.contains("+base changed"), "{patch}");
    assert!(
        !tracked.line_changes.is_empty(),
        "authoritative dirty diff omitted line changes"
    );
}

#[test]
fn mcp_status_and_dirty_diff_use_authoritative_public_dispatch() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    fs::create_dir(fixture.root().join("mcp-nested")).unwrap();
    fs::write(fixture.root().join("mcp-nested/new.txt"), b"mcp\n").unwrap();
    // Starts the workspace daemon and establishes the trusted ledger baseline.
    fixture.qualify();
    trail::test_support::set_changed_path_authority_override(true);

    let status = trail::mcp::handle_json_rpc(
        &mut fixture.db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "trail.status", "arguments": {"branch": "main"}}
        }),
    )
    .unwrap();
    let diff = trail::mcp::handle_json_rpc(
        &mut fixture.db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "trail.diff", "arguments": {"dirty": true}}
        }),
    )
    .unwrap();
    trail::test_support::set_changed_path_authority_override(false);

    assert_eq!(status["result"]["isError"], false, "{status}");
    assert!(status["result"]["structuredContent"]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|change| change["path"] == "mcp-nested/new.txt"));
    assert_eq!(diff["result"]["isError"], false, "{diff}");
    assert!(diff["result"]["structuredContent"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|change| change["path"] == "mcp-nested/new.txt"));
}
