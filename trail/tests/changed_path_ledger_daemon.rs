#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tempfile::TempDir;
use trail::{InitImportMode, Trail};

const DAEMON_PROTOCOL_VERSION: u16 = 2;

#[derive(Clone, Debug, Deserialize)]
struct Endpoint {
    protocol_version: u16,
    pid: u32,
    process_start_identity: String,
    executable_identity: String,
    workspace_identity: String,
    owner_nonce: String,
    auth_token: String,
    socket_path: PathBuf,
    observer_ready: bool,
    recovery_complete: bool,
    reconciliation_complete: bool,
    live_fence_sequence: u64,
    epoch: u64,
}

struct Fixture {
    temp: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("tracked.txt"), b"baseline\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        Self { temp }
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn endpoint_path(&self) -> PathBuf {
        self.root().join(".trail/index/change-ledger/daemon.json")
    }

    fn authority(&self) -> PathBuf {
        self.root().join(".trail/index/change-ledger")
    }

    fn token_path(&self) -> PathBuf {
        self.authority().join("daemon.token")
    }

    fn socket_path(&self) -> PathBuf {
        self.root()
            .canonicalize()
            .unwrap()
            .join(".trail/changed-path.sock")
    }

    fn create_authority(&self) {
        fs::create_dir_all(self.authority()).unwrap();
        fs::set_permissions(
            self.root().join(".trail"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::set_permissions(
            self.root().join(".trail/index"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::set_permissions(self.authority(), fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn status(&self) -> std::process::Output {
        self.status_with_env(&[])
    }

    fn status_with_env(&self, env: &[(&str, &str)]) -> std::process::Output {
        self.run_with_env(&["status"], env)
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.run_with_env(args, &[])
    }

    fn run_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
        let canonical_root = self.root().canonicalize().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_trail"));
        command
            .arg("--workspace")
            .arg(self.root())
            .arg("--json")
            .args(args)
            .env("HOME", &canonical_root)
            .env("XDG_CONFIG_HOME", canonical_root.join(".config"))
            .env("GIT_CONFIG_GLOBAL", "")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        for (name, value) in env {
            command.env(name, value);
        }
        command.output().unwrap()
    }

    fn endpoint(&self) -> Endpoint {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(bytes) = fs::read(self.endpoint_path()) {
                if let Ok(endpoint) = serde_json::from_slice(&bytes) {
                    return endpoint;
                }
            }
            assert!(
                Instant::now() < deadline,
                "daemon endpoint was not published"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn assert_status_failed(output: &std::process::Output) {
    assert!(
        !output.status.success(),
        "status unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("\"error\""),
        "unexpected diagnostic: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_owner_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn kill_and_wait(pid: u32) {
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    let deadline = Instant::now() + Duration::from_secs(5);
    while unsafe { libc::kill(pid as i32, 0) } == 0 {
        assert!(Instant::now() < deadline, "daemon did not exit");
        thread::sleep(Duration::from_millis(10));
    }
}

fn process_command_line(pid: u32) -> String {
    #[cfg(target_os = "linux")]
    {
        return String::from_utf8_lossy(
            &fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default(),
        )
        .replace('\0', " ");
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("ps")
            .args(["-o", "command=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        return String::from_utf8_lossy(&output.stdout).into_owned();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    String::new()
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(bytes) = fs::read(self.endpoint_path()) {
            if let Ok(endpoint) = serde_json::from_slice::<Endpoint>(&bytes) {
                unsafe {
                    libc::kill(endpoint.pid as i32, libc::SIGTERM);
                }
            }
        }
    }
}

#[test]
fn first_status_publishes_a_ready_workspace_daemon() {
    let fixture = Fixture::new();
    let output = fixture.status();
    assert!(
        output.status.success(),
        "status failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let endpoint = fixture.endpoint();
    assert_eq!(endpoint.protocol_version, DAEMON_PROTOCOL_VERSION);
    assert!(endpoint.observer_ready);
    assert!(endpoint.recovery_complete);
    assert!(endpoint.reconciliation_complete);
    assert!(endpoint.live_fence_sequence > 0);
    let command_line = process_command_line(endpoint.pid);
    assert!(!command_line.contains(&endpoint.auth_token));
    assert!(!command_line.contains(&endpoint.owner_nonce));
}

#[test]
fn concurrent_first_status_calls_converge_on_one_ready_owner() {
    let fixture = Arc::new(Fixture::new());
    assert!(!fixture.authority().exists());
    let barrier = Arc::new(Barrier::new(16));
    let callers = (0..16)
        .map(|_| {
            let fixture = Arc::clone(&fixture);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                fixture.status()
            })
        })
        .collect::<Vec<_>>();

    for output in callers.into_iter().map(|caller| caller.join().unwrap()) {
        assert!(
            output.status.success(),
            "status failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let endpoint = fixture.endpoint();
    let connection =
        rusqlite::Connection::open(fixture.root().join(".trail/index/trail.sqlite")).unwrap();
    let active_owners: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM changed_path_observer_owners WHERE lease_state='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_owners, 1);
    assert_eq!(endpoint.protocol_version, DAEMON_PROTOCOL_VERSION);
    assert!(endpoint.pid > 0);
    assert!(!endpoint.process_start_identity.is_empty());
    assert_eq!(endpoint.executable_identity.len(), 64);
    assert_eq!(endpoint.workspace_identity.len(), 64);
    assert_eq!(endpoint.owner_nonce.len(), 64);
    assert_eq!(endpoint.auth_token.len(), 64);
    assert_eq!(
        endpoint.socket_path,
        fixture
            .root()
            .canonicalize()
            .unwrap()
            .join(".trail/changed-path.sock")
    );
    assert!(endpoint.observer_ready);
    assert!(endpoint.recovery_complete);
    assert!(endpoint.reconciliation_complete);
    assert!(endpoint.live_fence_sequence > 0);
}

#[test]
fn first_diff_and_record_share_the_automatically_started_workspace_daemon() {
    let fixture = Fixture::new();
    let diff = fixture.run(&["diff", "--dirty", "--name-only"]);
    assert!(
        diff.status.success(),
        "{}",
        String::from_utf8_lossy(&diff.stderr)
    );
    let first = fixture.endpoint();
    fs::write(fixture.root().join("tracked.txt"), b"record me\n").unwrap();
    let record = fixture.run(&["record", "-m", "daemon record"]);
    assert!(
        record.status.success(),
        "record failed: {}",
        String::from_utf8_lossy(&record.stderr)
    );
    let second = fixture.endpoint();
    assert_eq!(second.pid, first.pid);
    assert_eq!(second.owner_nonce, first.owner_nonce);
    kill_and_wait(second.pid);
    let after_record_restart = fixture.status();
    assert!(
        after_record_restart.status.success(),
        "post-record restart failed: {}",
        String::from_utf8_lossy(&after_record_restart.stderr)
    );
    assert!(fixture.endpoint().epoch > second.epoch);
}

#[test]
fn endpoint_and_socket_symlinks_are_rejected_without_touching_targets() {
    let fixture = Fixture::new();
    fixture.create_authority();
    let endpoint_target = fixture.root().join("endpoint-target");
    fs::write(&endpoint_target, b"sentinel").unwrap();
    symlink(&endpoint_target, fixture.endpoint_path()).unwrap();
    assert_status_failed(&fixture.status());
    assert_eq!(fs::read(&endpoint_target).unwrap(), b"sentinel");
    fs::remove_file(fixture.endpoint_path()).unwrap();

    let socket_target = fixture.root().join("socket-target");
    fs::write(&socket_target, b"socket-sentinel").unwrap();
    symlink(&socket_target, fixture.socket_path()).unwrap();
    assert_status_failed(&fixture.status());
    assert_eq!(fs::read(&socket_target).unwrap(), b"socket-sentinel");
}

#[test]
fn unsafe_authority_and_endpoint_modes_fail_closed() {
    let fixture = Fixture::new();
    fixture.create_authority();
    fs::set_permissions(fixture.authority(), fs::Permissions::from_mode(0o755)).unwrap();
    assert_status_failed(&fixture.status());

    fs::set_permissions(fixture.authority(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(fixture.endpoint_path(), b"{}").unwrap();
    fs::set_permissions(fixture.endpoint_path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert_status_failed(&fixture.status());
}

#[test]
fn live_daemon_rejects_tampered_endpoint_and_token_identity() {
    let fixture = Fixture::new();
    let output = fixture.status();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let endpoint_bytes = fs::read(fixture.endpoint_path()).unwrap();
    let endpoint: serde_json::Value = serde_json::from_slice(&endpoint_bytes).unwrap();

    for (field, value) in [
        ("protocol_version", serde_json::json!(99)),
        ("owner_nonce", serde_json::json!("0".repeat(64))),
        ("workspace_identity", serde_json::json!("1".repeat(64))),
        ("executable_identity", serde_json::json!("2".repeat(64))),
        ("scope_id", serde_json::json!("3".repeat(64))),
        ("epoch", serde_json::json!(u64::MAX)),
    ] {
        let mut tampered = endpoint.clone();
        tampered[field] = value;
        write_owner_file(
            &fixture.endpoint_path(),
            &serde_json::to_vec_pretty(&tampered).unwrap(),
        );
        assert_status_failed(&fixture.status());
        write_owner_file(&fixture.endpoint_path(), &endpoint_bytes);
    }

    let mut unrelated_pid = endpoint.clone();
    unrelated_pid["pid"] = serde_json::json!(std::process::id());
    unrelated_pid["process_start_identity"] = serde_json::json!("synthetic-reused-pid-token");
    write_owner_file(
        &fixture.endpoint_path(),
        &serde_json::to_vec_pretty(&unrelated_pid).unwrap(),
    );
    assert_status_failed(&fixture.status());
    write_owner_file(&fixture.endpoint_path(), &endpoint_bytes);

    assert_status_failed(&fixture.status_with_env(&[(
        "TRAIL_TEST_WORKSPACE_DAEMON_POST_CHALLENGE_START_IDENTITY",
        "synthetic-reuse",
    )]));

    let token_bytes = fs::read(fixture.token_path()).unwrap();
    let token_target = fixture.root().join("token-target");
    fs::write(&token_target, b"sentinel").unwrap();
    fs::remove_file(fixture.token_path()).unwrap();
    symlink(&token_target, fixture.token_path()).unwrap();
    assert_status_failed(&fixture.status());
    assert_eq!(fs::read(&token_target).unwrap(), b"sentinel");
    fs::remove_file(fixture.token_path()).unwrap();
    write_owner_file(&fixture.token_path(), &token_bytes);

    fs::set_permissions(&fixture.socket_path(), fs::Permissions::from_mode(0o666)).unwrap();
    assert_status_failed(&fixture.status());
    fs::set_permissions(&fixture.socket_path(), fs::Permissions::from_mode(0o600)).unwrap();

    let starting = serde_json::json!({
        "protocol_version": endpoint["protocol_version"],
        "pid": endpoint["pid"],
        "process_start_identity": endpoint["process_start_identity"],
        "executable_identity": endpoint["executable_identity"],
        "workspace_identity": endpoint["workspace_identity"],
        "owner_nonce": endpoint["owner_nonce"],
        "socket_path": endpoint["socket_path"],
        "socket_device": endpoint["socket_device"],
        "socket_inode": endpoint["socket_inode"],
    });
    fs::remove_file(fixture.endpoint_path()).unwrap();
    let starting_path = fixture.authority().join("daemon.starting.json");
    write_owner_file(
        &starting_path,
        &serde_json::to_vec_pretty(&starting).unwrap(),
    );
    assert_status_failed(&fixture.status_with_env(&[(
        "TRAIL_TEST_WORKSPACE_DAEMON_UNVERIFIABLE_PID",
        &endpoint["pid"].as_u64().unwrap().to_string(),
    )]));
    assert!(starting_path.exists());
    assert!(fixture.socket_path().exists());
    fs::remove_file(starting_path).unwrap();
    write_owner_file(&fixture.endpoint_path(), &endpoint_bytes);
}

#[test]
fn killed_daemon_is_replaced_and_full_reconciliation_captures_offline_change() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    fs::write(fixture.root().join("tracked.txt"), b"changed while down\n").unwrap();
    fs::write(fixture.root().join(".trailignore"), b"new-ignore-rule\n").unwrap();

    let restarted = fixture.status();
    assert!(
        restarted.status.success(),
        "restart failed: {}",
        String::from_utf8_lossy(&restarted.stderr)
    );
    let second = fixture.endpoint();
    assert_ne!(second.pid, first.pid);
    assert_ne!(second.owner_nonce, first.owner_nonce);
    assert!(second.epoch > first.epoch);

    let connection =
        rusqlite::Connection::open(fixture.root().join(".trail/index/trail.sqlite")).unwrap();
    let captured: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM changed_path_entries WHERE normalized_path='tracked.txt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(captured, 1);
}

#[test]
fn dead_endpoint_with_missing_socket_restarts_safely() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    fs::remove_file(&first.socket_path).unwrap();
    let mut stale: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.endpoint_path()).unwrap()).unwrap();
    stale["protocol_version"] = serde_json::json!(1);
    stale["executable_identity"] = serde_json::json!("old-executable-identity");
    write_owner_file(
        &fixture.endpoint_path(),
        &serde_json::to_vec_pretty(&stale).unwrap(),
    );
    let restarted = fixture.status();
    assert!(
        restarted.status.success(),
        "restart failed: {}",
        String::from_utf8_lossy(&restarted.stderr)
    );
}

#[test]
fn stale_cleanup_refuses_to_unlink_a_substituted_same_user_socket() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    fs::remove_file(&first.socket_path).unwrap();
    let unrelated = UnixListener::bind(&first.socket_path).unwrap();
    fs::set_permissions(&first.socket_path, fs::Permissions::from_mode(0o600)).unwrap();
    let substituted_inode = fs::symlink_metadata(&first.socket_path).unwrap().ino();

    assert_status_failed(&fixture.status());
    assert_eq!(
        fs::symlink_metadata(&first.socket_path).unwrap().ino(),
        substituted_inode
    );
    drop(unrelated);
}

#[test]
fn crash_after_persisting_ledger_owner_is_automatically_recovered() {
    let fixture = Fixture::new();
    let crashed =
        fixture.status_with_env(&[("TRAIL_TEST_WORKSPACE_DAEMON_EXIT_AFTER_PREPARE", "1")]);
    assert_status_failed(&crashed);
    let recovered = fixture.status();
    assert!(
        recovered.status.success(),
        "recovery failed: {}",
        String::from_utf8_lossy(&recovered.stderr)
    );
}

#[test]
fn ordinary_error_after_persisting_owner_preserves_recovery_identity() {
    let fixture = Fixture::new();
    let failed =
        fixture.status_with_env(&[("TRAIL_TEST_WORKSPACE_DAEMON_ERROR_AFTER_PREPARE", "1")]);
    assert_status_failed(&failed);
    assert!(fixture.authority().join("daemon.starting.json").exists());
    let recovered = fixture.status();
    assert!(
        recovered.status.success(),
        "ordinary-error recovery failed: {}",
        String::from_utf8_lossy(&recovered.stderr)
    );
}

#[test]
fn readiness_timeout_kills_startup_owner_and_next_status_recovers() {
    let fixture = Fixture::new();
    let timed_out = fixture.status_with_env(&[
        ("TRAIL_TEST_WORKSPACE_DAEMON_READY_TIMEOUT_MS", "50"),
        ("TRAIL_TEST_WORKSPACE_DAEMON_DELAY_AFTER_INTENT_MS", "500"),
    ]);
    assert_status_failed(&timed_out);
    assert!(
        String::from_utf8_lossy(&timed_out.stderr).contains("readiness timed out"),
        "unexpected timeout diagnostic: {}",
        String::from_utf8_lossy(&timed_out.stderr)
    );
    let recovered = fixture.status();
    assert!(
        recovered.status.success(),
        "recovery failed: {}",
        String::from_utf8_lossy(&recovered.stderr)
    );
}

#[test]
fn live_policy_invalidation_self_restarts_and_reconciles() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    fs::write(fixture.root().join(".trailignore"), b"generated/**\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let second = loop {
        thread::sleep(Duration::from_millis(100));
        let recovered = fixture.status();
        assert!(
            recovered.status.success(),
            "live invalidation recovery failed: {}",
            String::from_utf8_lossy(&recovered.stderr)
        );
        let endpoint = fixture.endpoint();
        if endpoint.epoch > first.epoch {
            break endpoint;
        }
        assert!(
            Instant::now() < deadline,
            "policy invalidation was not observed"
        );
    };
    assert_ne!(second.owner_nonce, first.owner_nonce);
    assert!(second.epoch > first.epoch);
}

#[test]
fn real_git_external_global_config_is_observed_and_live_creation_recovers() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let home = temp.path().join("home");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::write(workspace.join("tracked.txt"), b"baseline\n").unwrap();
    Trail::init(&workspace, "main", InitImportMode::WorkingTree, false).unwrap();
    let git = Command::new("git")
        .args(["-C", workspace.to_str().unwrap(), "init", "--quiet"])
        .output()
        .unwrap();
    assert!(git.status.success(), "git init failed");
    let global_config = home.join("missing-global.gitconfig");
    let run_status = || {
        Command::new(env!("CARGO_BIN_EXE_trail"))
            .args([
                "--workspace",
                workspace.to_str().unwrap(),
                "--json",
                "status",
            ])
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", &global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
    };
    let first_status = run_status();
    assert!(
        first_status.status.success(),
        "real-Git first status failed: {}",
        String::from_utf8_lossy(&first_status.stderr)
    );
    let endpoint_path = workspace.join(".trail/index/change-ledger/daemon.json");
    let first: Endpoint = serde_json::from_slice(&fs::read(&endpoint_path).unwrap()).unwrap();

    fs::write(&global_config, b"[core]\n\tignorecase = false\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let second = loop {
        thread::sleep(Duration::from_millis(100));
        let status = run_status();
        assert!(
            status.status.success(),
            "external-policy recovery failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        let endpoint: Endpoint =
            serde_json::from_slice(&fs::read(&endpoint_path).unwrap()).unwrap();
        if endpoint.epoch > first.epoch {
            break endpoint;
        }
        assert!(
            Instant::now() < deadline,
            "global Git config creation was not observed"
        );
    };
    assert_ne!(second.owner_nonce, first.owner_nonce);
    unsafe { libc::kill(second.pid as i32, libc::SIGTERM) };
}
