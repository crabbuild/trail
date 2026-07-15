#![cfg(unix)]

use std::fs::{self, File};
use std::os::fd::{AsRawFd, FromRawFd};
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

fn spawn_status_waiting_after_daemon_authority_load(
    fixture: &Fixture,
    barrier: &Path,
) -> std::process::Child {
    fs::create_dir(barrier).unwrap();
    let canonical_root = fixture.root().canonicalize().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_trail"))
        .arg("--workspace")
        .arg(fixture.root())
        .arg("--json")
        .arg("status")
        .env("HOME", &canonical_root)
        .env("XDG_CONFIG_HOME", canonical_root.join(".config"))
        .env("GIT_CONFIG_GLOBAL", "")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("TRAIL_TEST_DAEMON_TRANSITION_AFTER_LOAD_BARRIER", barrier)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !barrier.join("loaded").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "daemon transition exited before authority load barrier: status={status}\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "daemon transition did not reach authority load barrier\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    child
}

#[derive(Debug, PartialEq)]
struct TransitionAuthoritySnapshot {
    scope: (
        i64,
        i64,
        String,
        String,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        i64,
    ),
    owner: (
        i64,
        String,
        String,
        String,
        String,
        Option<Vec<u8>>,
        i64,
        i64,
        i64,
        Option<String>,
        Option<i64>,
    ),
    limits: (i64, i64, i64, i64, i64, i64, i64),
}

fn transition_authority_snapshot(database: &Path) -> TransitionAuthoritySnapshot {
    let conn = rusqlite::Connection::open(database).unwrap();
    let scope = conn
        .query_row(
            "SELECT epoch,ref_generation,baseline_root_id,policy_fingerprint,
                    policy_dependency_generation,filesystem_identity,provider_id,
                    provider_identity,observer_owner_token,trust_state,continuity_generation
             FROM changed_path_scopes",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ))
            },
        )
        .unwrap();
    let owner = conn
        .query_row(
            "SELECT epoch,owner_token,provider_id,provider_identity,lease_state,
                    fence_nonce,acquired_at,heartbeat_at,expires_at,error_state,error_at
             FROM changed_path_observer_owners",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ))
            },
        )
        .unwrap();
    let limits = conn
        .query_row(
            "SELECT schema_version,max_candidate_rows,max_prefix_rows,
                    max_observer_log_bytes,max_segment_bytes,max_unfolded_tail_records,
                    case_sensitive
             FROM changed_path_scopes",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    TransitionAuthoritySnapshot {
        scope,
        owner,
        limits,
    }
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
fn auto_started_daemon_does_not_retain_unrelated_inherited_file_descriptors() {
    let fixture = Fixture::new();
    let mut pipe = [0_i32; 2];
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    let read = unsafe { File::from_raw_fd(pipe[0]) };
    let write = unsafe { File::from_raw_fd(pipe[1]) };
    let read_flags = unsafe { libc::fcntl(read.as_raw_fd(), libc::F_GETFD) };
    let write_flags = unsafe { libc::fcntl(write.as_raw_fd(), libc::F_GETFD) };
    assert!(read_flags >= 0 && write_flags >= 0);
    assert_eq!(
        unsafe {
            libc::fcntl(
                read.as_raw_fd(),
                libc::F_SETFD,
                read_flags | libc::FD_CLOEXEC,
            )
        },
        0
    );
    assert_eq!(
        unsafe {
            libc::fcntl(
                write.as_raw_fd(),
                libc::F_SETFD,
                write_flags & !libc::FD_CLOEXEC,
            )
        },
        0
    );

    let status = fixture.status();
    assert!(
        status.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    drop(write);

    let mut poll_fd = libc::pollfd {
        fd: read.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let polled = unsafe { libc::poll(&mut poll_fd, 1, 1_000) };
    assert_eq!(
        polled, 1,
        "daemon retained an unrelated inherited pipe writer"
    );
    assert_ne!(poll_fd.revents & libc::POLLHUP, 0);
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
fn stale_cleanup_does_not_unlink_socket_substituted_after_identity_verification() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    assert!(
        first.socket_path.exists(),
        "killed daemon removed the stale socket before cleanup could verify it"
    );

    let barrier = fixture.root().join("socket-unlink-race");
    fs::create_dir(&barrier).unwrap();
    let canonical_root = fixture.root().canonicalize().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_trail"))
        .arg("--workspace")
        .arg(fixture.root())
        .arg("--json")
        .arg("status")
        .env("HOME", &canonical_root)
        .env("XDG_CONFIG_HOME", canonical_root.join(".config"))
        .env("GIT_CONFIG_GLOBAL", "")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "TRAIL_TEST_WORKSPACE_DAEMON_SOCKET_UNLINK_BARRIER",
            &barrier,
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    while !barrier.join("verified").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "stale cleanup exited before the verified socket boundary: status={status}\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "stale cleanup did not reach the verified socket boundary before timeout\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }

    fs::remove_file(&first.socket_path).unwrap();
    let unrelated = UnixListener::bind(&first.socket_path).unwrap();
    fs::set_permissions(&first.socket_path, fs::Permissions::from_mode(0o600)).unwrap();
    let substituted_inode = fs::symlink_metadata(&first.socket_path).unwrap().ino();
    fs::write(barrier.join("continue"), b"go").unwrap();

    let output = child.wait_with_output().unwrap();
    assert_status_failed(&output);
    assert_eq!(
        fs::symlink_metadata(&first.socket_path).unwrap().ino(),
        substituted_inode,
        "stale cleanup unlinked the socket substituted after verification"
    );
    drop(unrelated);
}

#[test]
fn stale_cleanup_never_unlinks_socket_substituted_after_quarantine_verification() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    assert!(first.socket_path.exists());

    let barrier = fixture.root().join("socket-quarantine-race");
    fs::create_dir(&barrier).unwrap();
    let canonical_root = fixture.root().canonicalize().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_trail"))
        .arg("--workspace")
        .arg(fixture.root())
        .arg("--json")
        .arg("status")
        .env("HOME", &canonical_root)
        .env("XDG_CONFIG_HOME", canonical_root.join(".config"))
        .env("GIT_CONFIG_GLOBAL", "")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "TRAIL_TEST_WORKSPACE_DAEMON_SOCKET_QUARANTINE_BARRIER",
            &barrier,
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let verified = barrier.join("verified");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !verified.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "stale cleanup exited before quarantine verification: status={status}\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "stale cleanup did not reach quarantine verification\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }

    let quarantine_leaf = fs::read_to_string(&verified).unwrap();
    assert!(
        quarantine_leaf.starts_with(".changed-path-socket-tombstone."),
        "unexpected tombstone namespace: {quarantine_leaf}"
    );
    let quarantine_path = fixture.root().join(".trail").join(&quarantine_leaf);
    assert!(quarantine_path.exists());
    assert!(!first.socket_path.exists());
    let unrelated = UnixListener::bind(&first.socket_path).unwrap();
    fs::set_permissions(&first.socket_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(&quarantine_path).unwrap();
    fs::rename(&first.socket_path, &quarantine_path).unwrap();
    let substituted_inode = fs::symlink_metadata(&quarantine_path).unwrap().ino();
    fs::write(barrier.join("continue"), b"go").unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(
        quarantine_path.exists(),
        "cleanup pathname-unlinked a socket substituted after quarantine verification; status={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::symlink_metadata(&quarantine_path).unwrap().ino(),
        substituted_inode
    );
    drop(unrelated);
}

fn populate_socket_tombstones(fixture: &Fixture, count: usize) {
    for index in 0..count {
        fs::write(
            fixture.root().join(".trail").join(format!(
                ".changed-path-socket-tombstone.{index:024x}.removing"
            )),
            b"retained",
        )
        .unwrap();
    }
}

#[test]
fn socket_tombstone_cap_minus_one_permits_exactly_the_final_slot() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    populate_socket_tombstones(&fixture, 1023);
    fs::write(
        fixture
            .root()
            .join(".trail/.changed-path-socket-tombstone.not-hex.removing"),
        b"near-match",
    )
    .unwrap();

    let output = fixture.status();
    assert_status_failed(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("reinitialize this workspace"),
        "missing reinitialize guidance after consuming final slot: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exact = fs::read_dir(fixture.root().join(".trail"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(hex) = name
                .strip_prefix(".changed-path-socket-tombstone.")
                .and_then(|name| name.strip_suffix(".removing"))
            else {
                return false;
            };
            hex.len() == 24
                && hex
                    .as_bytes()
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
        .count();
    assert_eq!(exact, 1024);
    assert!(
        !first.socket_path.exists(),
        "cap-minus-one did not move the verified stale socket into the final tombstone slot"
    );
    let private_bind_leaves = fs::read_dir(fixture.root().join(".trail"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.len() == 14
                && name.starts_with(".s")
                && name[2..]
                    .as_bytes()
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
        .count();
    assert_eq!(private_bind_leaves, 0);
}

#[test]
fn socket_tombstone_cap_refuses_before_rename_with_reinitialize_guidance() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    let original_inode = fs::symlink_metadata(&first.socket_path).unwrap().ino();
    populate_socket_tombstones(&fixture, 1024);

    let output = fixture.status();
    assert_status_failed(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("reinitialize this workspace"),
        "missing reinitialize guidance: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::symlink_metadata(&first.socket_path).unwrap().ino(),
        original_inode,
        "cap refusal moved the original socket"
    );
}

#[test]
fn socket_tombstone_cap_refuses_before_creating_private_bind_leaf() {
    let fixture = Fixture::new();
    populate_socket_tombstones(&fixture, 1024);
    let output = fixture.status();
    assert_status_failed(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("reinitialize this workspace"),
        "missing reinitialize guidance: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let private_bind_leaves = fs::read_dir(fixture.root().join(".trail"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.len() == 14
                && name.starts_with(".s")
                && name[2..]
                    .as_bytes()
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
        .count();
    assert_eq!(private_bind_leaves, 0);
}

#[test]
fn private_bind_socket_is_created_with_owner_only_mode_atomically() {
    let fixture = Fixture::new();
    let barrier = fixture.root().join("socket-bound-mode");
    fs::create_dir(&barrier).unwrap();
    let canonical_root = fixture.root().canonicalize().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_trail"))
        .arg("--workspace")
        .arg(fixture.root())
        .arg("--json")
        .arg("status")
        .env("HOME", &canonical_root)
        .env("XDG_CONFIG_HOME", canonical_root.join(".config"))
        .env("GIT_CONFIG_GLOBAL", "")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("TRAIL_TEST_WORKSPACE_DAEMON_SOCKET_BOUND_BARRIER", &barrier)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let bound = barrier.join("bound");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !bound.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "daemon exited before socket bind boundary: status={status} stderr={}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert!(Instant::now() < deadline, "socket bind barrier timed out");
        thread::sleep(Duration::from_millis(10));
    }
    let leaf = fs::read_to_string(&bound).unwrap();
    let socket = fixture.root().join(".trail").join(leaf);
    let initial_mode = fs::symlink_metadata(&socket).unwrap().permissions().mode() & 0o777;
    fs::write(barrier.join("continue"), b"go").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "daemon startup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(initial_mode, 0o600);
}

#[test]
fn socket_tombstone_noreplace_collision_retains_original_socket() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    let original_inode = fs::symlink_metadata(&first.socket_path).unwrap().ino();
    let barrier = fixture.root().join("socket-quarantine-prerename-race");
    fs::create_dir(&barrier).unwrap();
    let canonical_root = fixture.root().canonicalize().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_trail"))
        .arg("--workspace")
        .arg(fixture.root())
        .arg("--json")
        .arg("status")
        .env("HOME", &canonical_root)
        .env("XDG_CONFIG_HOME", canonical_root.join(".config"))
        .env("GIT_CONFIG_GLOBAL", "")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "TRAIL_TEST_WORKSPACE_DAEMON_SOCKET_QUARANTINE_PRE_RENAME_BARRIER",
            &barrier,
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let prepared = barrier.join("prepared");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !prepared.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "cleanup exited before pre-rename boundary: status={status} stderr={}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert!(Instant::now() < deadline, "pre-rename barrier timed out");
        thread::sleep(Duration::from_millis(10));
    }
    let tombstone_leaf = fs::read_to_string(&prepared).unwrap();
    let collision = fixture.root().join(".trail").join(tombstone_leaf);
    fs::write(&collision, b"collision").unwrap();
    fs::write(barrier.join("continue"), b"go").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_status_failed(&output);
    assert_eq!(fs::read(&collision).unwrap(), b"collision");
    assert_eq!(
        fs::symlink_metadata(&first.socket_path).unwrap().ino(),
        original_inode
    );
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

#[test]
fn verified_stale_persisted_identity_drift_rotates_epoch_and_reconciles() {
    for column in ["filesystem_identity", "provider_identity"] {
        let fixture = Fixture::new();
        assert!(fixture.status().status.success());
        let first = fixture.endpoint();
        kill_and_wait(first.pid);

        let conn =
            rusqlite::Connection::open(fixture.root().join(".trail/index/trail.sqlite")).unwrap();
        let original_authority: (String, String, String, String, [i64; 7]) = conn
            .query_row(
                "SELECT filesystem_identity,scope_root_identity,
                        provider_identity,provider_id,
                        durable_cursor,linearizable_fence,rename_pairing,
                        overflow_scope,filesystem_supported,clean_proof_allowed,
                        power_loss_durability
                 FROM changed_path_scopes LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        [
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                            row.get(9)?,
                            row.get(10)?,
                        ],
                    ))
                },
            )
            .unwrap();
        assert_eq!(original_authority.0, original_authority.1);
        assert_eq!(original_authority.2, original_authority.3);
        conn.execute(&format!("UPDATE changed_path_scopes SET {column}='00'"), [])
            .unwrap();
        drop(conn);

        let recovered = fixture.status();
        assert!(
            recovered.status.success(),
            "{column} drift did not automatically recover: {}",
            String::from_utf8_lossy(&recovered.stderr)
        );
        let second = fixture.endpoint();
        assert!(second.epoch > first.epoch, "{column} drift kept old epoch");
        assert_ne!(second.owner_nonce, first.owner_nonce);

        let conn =
            rusqlite::Connection::open(fixture.root().join(".trail/index/trail.sqlite")).unwrap();
        let (stored_epoch, trust_state, recovered_authority): (
            i64,
            String,
            (String, String, String, String, [i64; 7]),
        ) = conn
            .query_row(
                "SELECT epoch,trust_state,
                        filesystem_identity,scope_root_identity,
                        provider_identity,provider_id,
                        durable_cursor,linearizable_fence,rename_pairing,
                        overflow_scope,filesystem_supported,clean_proof_allowed,
                        power_loss_durability
                 FROM changed_path_scopes LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        (
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            [
                                row.get(6)?,
                                row.get(7)?,
                                row.get(8)?,
                                row.get(9)?,
                                row.get(10)?,
                                row.get(11)?,
                                row.get(12)?,
                            ],
                        ),
                    ))
                },
            )
            .unwrap();
        assert_eq!(u64::try_from(stored_epoch).unwrap(), second.epoch);
        assert_eq!(trust_state, "trusted");
        assert_eq!(recovered_authority, original_authority);
        assert_eq!(recovered_authority.0, recovered_authority.1);
        assert_eq!(recovered_authority.2, recovered_authority.3);
        kill_and_wait(second.pid);
    }
}

fn assert_loaded_scope_authority_race_is_rejected(
    name: &str,
    mutation: &str,
    retained_column: &str,
    expected_value: &str,
) {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    let database = fixture.root().join(".trail/index/trail.sqlite");
    let conn = rusqlite::Connection::open(&database).unwrap();
    conn.execute(
        "UPDATE changed_path_scopes SET filesystem_identity='00'",
        [],
    )
    .unwrap();
    drop(conn);

    let barrier = fixture.root().join(format!("{name}-authority-race"));
    let child = spawn_status_waiting_after_daemon_authority_load(&fixture, &barrier);
    let conn = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(conn.execute(mutation, []).unwrap(), 1);
    drop(conn);
    let concurrent_authority = transition_authority_snapshot(&database);
    fs::write(barrier.join("continue"), b"go").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_status_failed(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("daemon authority transition lost exact loaded authority"),
        "{name} race did not fail at the full authority CAS boundary: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        transition_authority_snapshot(&database),
        concurrent_authority,
        "{name} race partially transitioned authority before failing closed"
    );
    let conn = rusqlite::Connection::open(&database).unwrap();
    let retained: String = conn
        .query_row(
            &format!("SELECT {retained_column} FROM changed_path_scopes"),
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, expected_value, "{name} authority was overwritten");
}

#[test]
fn verified_stale_transition_rejects_baseline_change_after_authority_load() {
    assert_loaded_scope_authority_race_is_rejected(
        "baseline",
        "UPDATE changed_path_scopes SET ref_generation=ref_generation+1, baseline_root_id='concurrent-baseline-root'",
        "baseline_root_id",
        "concurrent-baseline-root",
    );
}

#[test]
fn verified_stale_transition_rejects_policy_change_after_authority_load() {
    assert_loaded_scope_authority_race_is_rejected(
        "policy",
        "UPDATE changed_path_scopes SET policy_fingerprint='1111111111111111111111111111111111111111111111111111111111111111', policy_dependency_generation=policy_dependency_generation+1",
        "policy_fingerprint",
        "1111111111111111111111111111111111111111111111111111111111111111",
    );
}

#[test]
fn verified_stale_transition_rejects_limit_change_after_authority_load() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    let database = fixture.root().join(".trail/index/trail.sqlite");
    let conn = rusqlite::Connection::open(&database).unwrap();
    conn.execute(
        "UPDATE changed_path_scopes SET filesystem_identity='00'",
        [],
    )
    .unwrap();
    drop(conn);

    let barrier = fixture.root().join("limit-authority-race");
    let child = spawn_status_waiting_after_daemon_authority_load(&fixture, &barrier);
    let conn = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE changed_path_scopes SET max_candidate_rows=max_candidate_rows+1",
            [],
        )
        .unwrap(),
        1
    );
    drop(conn);
    let concurrent_authority = transition_authority_snapshot(&database);
    let concurrent_max_candidate_rows = concurrent_authority.limits.1;
    fs::write(barrier.join("continue"), b"go").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_status_failed(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("daemon authority transition lost exact loaded authority"),
        "limit race did not fail at the full authority CAS boundary: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        transition_authority_snapshot(&database),
        concurrent_authority,
        "limit race partially transitioned authority before failing closed"
    );
    let conn = rusqlite::Connection::open(&database).unwrap();
    let retained_limit: i64 = conn
        .query_row(
            "SELECT max_candidate_rows FROM changed_path_scopes",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained_limit, concurrent_max_candidate_rows);
}

#[test]
fn verified_stale_transition_does_not_revoke_owner_acquired_after_authority_load() {
    let fixture = Fixture::new();
    assert!(fixture.status().status.success());
    let first = fixture.endpoint();
    kill_and_wait(first.pid);
    let database = fixture.root().join(".trail/index/trail.sqlite");
    let conn = rusqlite::Connection::open(&database).unwrap();
    conn.execute(
        "UPDATE changed_path_scopes SET filesystem_identity='00'",
        [],
    )
    .unwrap();
    drop(conn);

    let barrier = fixture.root().join("owner-authority-race");
    let child = spawn_status_waiting_after_daemon_authority_load(&fixture, &barrier);
    let replacement_token = "ab".repeat(32);
    let conn = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE changed_path_observer_owners
             SET owner_token=?1, provider_id='concurrent-provider',
                 provider_identity='concurrent-provider-identity', lease_state='active',
                 fence_nonce=x'01020304050607080910111213141516',
                 error_state=NULL,error_at=NULL,updated_at=strftime('%s','now')
             WHERE scope_id=(SELECT scope_id FROM changed_path_scopes)",
            [&replacement_token],
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.execute(
            "UPDATE changed_path_scopes SET observer_owner_token=?1",
            [&replacement_token],
        )
        .unwrap(),
        1
    );
    drop(conn);
    fs::write(barrier.join("continue"), b"go").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_status_failed(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("daemon authority transition lost exact loaded authority"),
        "owner race did not fail at the full authority CAS boundary: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let conn = rusqlite::Connection::open(&database).unwrap();
    let owner: (String, String, String) = conn
        .query_row(
            "SELECT owner_token,lease_state,provider_id FROM changed_path_observer_owners",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        owner,
        (
            replacement_token.clone(),
            "active".into(),
            "concurrent-provider".into()
        )
    );
    let scope_owner: String = conn
        .query_row(
            "SELECT observer_owner_token FROM changed_path_scopes",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(scope_owner, replacement_token);
}
