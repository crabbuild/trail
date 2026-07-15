use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::{Duration, Instant};

use getrandom::getrandom;
use rustix::fs::{flock, FlockOperation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trail::{Error, Result, Trail};

use super::{daemon_rpc, RuntimeContext};

const PROTOCOL_VERSION: u16 = 2;
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);
const ENDPOINT_FILE: &str = "daemon.json";
const TOKEN_FILE: &str = "daemon.token";
const LOCK_FILE: &str = "daemon.lock";
const STARTING_FILE: &str = "daemon.starting.json";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct WorkspaceDaemonEndpoint {
    pub(super) protocol_version: u16,
    pub(super) pid: u32,
    pub(super) process_start_identity: String,
    pub(super) executable_identity: String,
    pub(super) workspace_identity: String,
    pub(super) owner_nonce: String,
    pub(super) auth_token: String,
    pub(super) socket_path: PathBuf,
    pub(super) socket_device: u64,
    pub(super) socket_inode: u64,
    pub(super) url: String,
    pub(super) observer_ready: bool,
    pub(super) recovery_complete: bool,
    pub(super) reconciliation_complete: bool,
    pub(super) live_fence_sequence: u64,
    pub(super) scope_id: String,
    pub(super) epoch: u64,
    pub(super) durable_offset: u64,
    pub(super) folded_offset: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct WorkspaceDaemonStarting {
    protocol_version: u16,
    pid: u32,
    process_start_identity: String,
    executable_identity: String,
    workspace_identity: String,
    owner_nonce: String,
    socket_path: PathBuf,
    socket_device: u64,
    socket_inode: u64,
}

#[derive(Clone, Debug)]
pub(super) struct DaemonReady {
    pub(super) url: String,
    pub(super) auth_token: String,
}

pub(super) fn ensure_workspace_daemon_ready(
    workspace: &Path,
    requested_token: Option<&str>,
) -> Result<DaemonReady> {
    let workspace = workspace.canonicalize()?;
    let db_dir = workspace.join(".trail");
    let authority = secure_authority_directory(&db_dir)?;
    authority.verify_trail_identity(&db_dir)?;
    let lock = secure_open_lock(&authority)?;
    if let Some(ready) =
        acquire_or_observe_published_daemon(&lock, &authority, &workspace, requested_token)?
    {
        return Ok(ready);
    }
    let mut replace_verified_stale_owner = false;
    if let Some(endpoint) = read_secure_endpoint(&authority)? {
        match classify_endpoint(&workspace, &authority, &endpoint, requested_token)? {
            EndpointState::Ready(ready) => return Ok(ready),
            EndpointState::Stale => {
                remove_stale_publication(&authority, &endpoint)?;
                replace_verified_stale_owner = true;
            }
        }
    }
    if recover_stale_starting_publication(&workspace, &authority)? {
        replace_verified_stale_owner = true;
    }

    let token = match requested_token {
        Some(token) if !token.trim().is_empty() => token.to_string(),
        Some(_) => {
            return Err(Error::InvalidInput(
                "workspace daemon authentication token cannot be empty".into(),
            ))
        }
        None => random_hex(32)?,
    };
    if let Err(error) = spawn_workspace_daemon(&workspace, &token, replace_verified_stale_owner) {
        if error
            .to_string()
            .contains("recording policy changed during observer startup")
        {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match recover_stale_starting_publication(&workspace, &authority) {
                    Ok(true) => break,
                    Ok(false) => return Err(error),
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return Err(error),
                }
            }
            spawn_workspace_daemon(&workspace, &token, true)?;
        } else {
            return Err(error);
        }
    }
    let endpoint = read_secure_endpoint(&authority)?.ok_or_else(|| {
        Error::DaemonUnavailable("workspace daemon became ready without an endpoint".into())
    })?;
    match classify_endpoint(&workspace, &authority, &endpoint, Some(&token))? {
        EndpointState::Ready(ready) => Ok(ready),
        EndpointState::Stale => Err(Error::DaemonUnavailable(
            "workspace daemon exited before readiness could be authenticated".into(),
        )),
    }
}

enum EndpointState {
    Ready(DaemonReady),
    Stale,
}

fn classify_endpoint(
    workspace: &Path,
    authority: &SecureAuthority,
    endpoint: &WorkspaceDaemonEndpoint,
    requested_token: Option<&str>,
) -> Result<EndpointState> {
    let expected_workspace = workspace_identity(workspace)?;
    let expected_executable = executable_identity(&std::env::current_exe()?)?;
    let expected_socket = workspace.join(".trail").join("changed-path.sock");
    authority.verify_trail_identity(&workspace.join(".trail"))?;
    if endpoint.socket_path != expected_socket
        || endpoint.pid == 0
        || endpoint.process_start_identity.is_empty()
    {
        return Err(Error::DaemonUnavailable(
            "workspace daemon endpoint lacks exact stale-cleanup identity; refusing replacement"
                .into(),
        ));
    }
    let alive = process_is_alive(endpoint.pid);
    let actual_start = process_start_identity(endpoint.pid);
    if !alive {
        return Ok(EndpointState::Stale);
    }
    let Some(actual_start) = actual_start else {
        return Err(Error::DaemonUnavailable(format!(
            "live workspace daemon PID {} cannot be identity-verified; refusing replacement",
            endpoint.pid
        )));
    };
    let mut invalid = Vec::new();
    if endpoint.protocol_version != PROTOCOL_VERSION {
        invalid.push("protocol");
    }
    if endpoint.workspace_identity != expected_workspace {
        invalid.push("workspace");
    }
    if endpoint.executable_identity != expected_executable {
        invalid.push("executable");
    }
    if endpoint.url != format!("unix://{}", expected_socket.display()) {
        invalid.push("url");
    }
    if endpoint.owner_nonce.len() != 64 {
        invalid.push("owner_nonce");
    }
    if endpoint.auth_token.len() != 64 {
        invalid.push("auth_token");
    }
    if requested_token.is_some_and(|token| token != endpoint.auth_token) {
        invalid.push("requested_token");
    }
    if !endpoint.observer_ready {
        invalid.push("observer_ready");
    }
    if !endpoint.recovery_complete {
        invalid.push("recovery_complete");
    }
    if !endpoint.reconciliation_complete {
        invalid.push("reconciliation_complete");
    }
    if endpoint.live_fence_sequence == 0 {
        invalid.push("live_fence");
    }
    if endpoint.scope_id.len() != 64 {
        invalid.push("scope");
    }
    if endpoint.epoch == 0 {
        invalid.push("epoch");
    }
    if endpoint.folded_offset > endpoint.durable_offset {
        invalid.push("offsets");
    }
    if !invalid.is_empty() {
        if actual_start != endpoint.process_start_identity {
            return Ok(EndpointState::Stale);
        }
        return Err(Error::DaemonUnavailable(format!(
            "workspace daemon endpoint identity is unverifiable ({}) ; refusing replacement",
            invalid.join(",")
        )));
    }
    let published_token = read_secure_owner_text(authority, TOKEN_FILE, 4096)?;
    if published_token.trim_end() != endpoint.auth_token {
        return Err(Error::DaemonUnavailable(
            "workspace daemon token publication does not match the endpoint".into(),
        ));
    }
    verify_secure_socket_identity(
        &endpoint.socket_path,
        endpoint.socket_device,
        endpoint.socket_inode,
    )?;
    let proof = match daemon_rpc::authenticated_ledger_fence(endpoint) {
        Ok(proof) => proof,
        Err(_) if actual_start != endpoint.process_start_identity => {
            return Ok(EndpointState::Stale)
        }
        Err(error)
            if error
                .to_string()
                .contains("changed-path observer health no longer authorizes") =>
        {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if !process_is_alive(endpoint.pid)
                    || process_start_identity(endpoint.pid).as_deref()
                        != Some(endpoint.process_start_identity.as_str())
                {
                    return Ok(EndpointState::Stale);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    let post_challenge_start = post_challenge_process_start_identity(endpoint.pid).ok_or_else(|| {
        Error::DaemonUnavailable(
            "workspace daemon process identity disappeared during authentication; refusing replacement"
                .into(),
        )
    })?;
    if actual_start != endpoint.process_start_identity
        || post_challenge_start != endpoint.process_start_identity
        || post_challenge_start != actual_start
    {
        return Err(Error::DaemonUnavailable(
            "an authenticated workspace daemon is live but its published process identity is unverifiable; refusing replacement".into(),
        ));
    }
    if proof.protocol_version != endpoint.protocol_version
        || proof.pid != endpoint.pid
        || proof.process_start_identity != endpoint.process_start_identity
        || proof.executable_identity != endpoint.executable_identity
        || proof.owner_nonce != endpoint.owner_nonce
        || proof.workspace_identity != endpoint.workspace_identity
        || proof.scope_id != endpoint.scope_id
        || proof.epoch != endpoint.epoch
        || proof.live_fence_sequence < endpoint.live_fence_sequence
        || proof.durable_offset < endpoint.durable_offset
        || proof.folded_offset > proof.durable_offset
    {
        return Err(Error::DaemonUnavailable(
            "workspace daemon challenge-response identity mismatch".into(),
        ));
    }
    Ok(EndpointState::Ready(DaemonReady {
        url: endpoint.url.clone(),
        auth_token: endpoint.auth_token.clone(),
    }))
}

fn spawn_workspace_daemon(
    workspace: &Path,
    token: &str,
    replace_verified_stale_owner: bool,
) -> Result<()> {
    let mut pipe = [0_i32; 2];
    if unsafe { libc::pipe(pipe.as_mut_ptr()) } != 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let read_fd = pipe[0];
    let write_fd = pipe[1];
    let mut token_pipe = [0_i32; 2];
    if unsafe { libc::pipe(token_pipe.as_mut_ptr()) } != 0 {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let token_read_fd = token_pipe[0];
    let token_write_fd = token_pipe[1];
    let flags = unsafe { libc::fcntl(write_fd, libc::F_GETFD) };
    let token_read_flags = unsafe { libc::fcntl(token_read_fd, libc::F_GETFD) };
    let token_write_flags = unsafe { libc::fcntl(token_write_fd, libc::F_GETFD) };
    if flags < 0
        || token_read_flags < 0
        || token_write_flags < 0
        || unsafe { libc::fcntl(write_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0
        || unsafe {
            libc::fcntl(
                token_read_fd,
                libc::F_SETFD,
                token_read_flags & !libc::FD_CLOEXEC,
            )
        } < 0
        || unsafe {
            libc::fcntl(
                token_write_fd,
                libc::F_SETFD,
                token_write_flags | libc::FD_CLOEXEC,
            )
        } < 0
    {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
            libc::close(token_read_fd);
            libc::close(token_write_fd);
        }
        return Err(Error::Io(std::io::Error::last_os_error()));
    }

    let owner_nonce = random_hex(32)?;
    let child = ProcessCommand::new(std::env::current_exe()?)
        .arg("--workspace")
        .arg(workspace)
        .arg("--quiet")
        .arg("daemon")
        .arg("--port")
        .arg("0")
        .env("TRAIL_WORKSPACE_DAEMON", "1")
        .env("TRAIL_WORKSPACE_DAEMON_READY_FD", write_fd.to_string())
        .env("TRAIL_WORKSPACE_DAEMON_TOKEN_FD", token_read_fd.to_string())
        .env("TRAIL_WORKSPACE_DAEMON_OWNER_NONCE", owner_nonce)
        .env(
            "TRAIL_WORKSPACE_DAEMON_REPLACE_STALE",
            if replace_verified_stale_owner {
                "1"
            } else {
                "0"
            },
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
                libc::close(token_read_fd);
                libc::close(token_write_fd);
            }
            return Err(Error::Io(error));
        }
    };
    unsafe {
        libc::close(write_fd);
        libc::close(token_read_fd);
    }
    let mut token_writer = unsafe { File::from_raw_fd(token_write_fd) };
    token_writer.write_all(token.as_bytes())?;
    drop(token_writer);
    let mut read = unsafe { File::from_raw_fd(read_fd) };
    let deadline = Instant::now() + ready_timeout();
    let mut byte = [0_u8; 1];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::DaemonUnavailable(
                "workspace daemon readiness timed out".into(),
            ));
        }
        let mut poll_fd = libc::pollfd {
            fd: read.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms =
            i32::try_from(remaining.as_millis().min(i32::MAX as u128)).unwrap_or(i32::MAX);
        let polled = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
        if polled < 0 {
            return Err(Error::Io(std::io::Error::last_os_error()));
        }
        if polled == 0 {
            continue;
        }
        match read.read(&mut byte) {
            Ok(1) if byte[0] == 1 => return Ok(()),
            Ok(_) => {
                let status = child.try_wait()?.map(|status| status.to_string());
                let mut diagnostic = String::new();
                if let Some(stderr) = child.stderr.take() {
                    let _ = stderr.take(64 * 1024).read_to_string(&mut diagnostic);
                }
                return Err(Error::DaemonUnavailable(format!(
                    "workspace daemon exited before readiness{}{}",
                    status
                        .map(|value| format!(" ({value})"))
                        .unwrap_or_default(),
                    if diagnostic.trim().is_empty() {
                        String::new()
                    } else {
                        format!(": {}", diagnostic.trim())
                    }
                )));
            }
            Err(error) => return Err(Error::Io(error)),
        }
    }
}

pub(super) fn is_auto_workspace_daemon() -> bool {
    std::env::var_os("TRAIL_WORKSPACE_DAEMON").is_some()
}

pub(super) fn run_auto_workspace_daemon(mut db: Trail) -> Result<()> {
    let workspace = db.workspace_root().canonicalize()?;
    let authority = secure_authority_directory(db.db_dir())?;
    let token_fd = required_env("TRAIL_WORKSPACE_DAEMON_TOKEN_FD")?
        .parse::<RawFd>()
        .map_err(|_| Error::InvalidInput("workspace daemon token fd is invalid".into()))?;
    std::env::remove_var("TRAIL_WORKSPACE_DAEMON_TOKEN_FD");
    let mut token_reader = unsafe { File::from_raw_fd(token_fd) };
    let mut token = String::new();
    (&mut token_reader).take(65).read_to_string(&mut token)?;
    drop(token_reader);
    let owner_nonce = required_env("TRAIL_WORKSPACE_DAEMON_OWNER_NONCE")?;
    if token.len() != 64 || owner_nonce.len() != 64 {
        return Err(Error::InvalidInput(
            "workspace daemon received malformed authentication identity".into(),
        ));
    }
    let ready_fd = required_env("TRAIL_WORKSPACE_DAEMON_READY_FD")?
        .parse::<RawFd>()
        .map_err(|_| Error::InvalidInput("workspace daemon readiness fd is invalid".into()))?;
    std::env::remove_var("TRAIL_WORKSPACE_DAEMON_READY_FD");

    let socket_path = db.db_dir().join("changed-path.sock");
    authority.verify_trail_identity(db.db_dir())?;
    if fs::symlink_metadata(&socket_path).is_ok() {
        return Err(Error::DaemonUnavailable(
            "workspace daemon socket pathname is already occupied".into(),
        ));
    }
    let socket = std::os::unix::net::UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    verify_secure_socket(&socket_path)?;
    let socket_metadata = fs::symlink_metadata(&socket_path)?;
    authority.verify_trail_identity(db.db_dir())?;
    let mut unpublished_socket = BoundSocketGuard {
        path: socket_path.clone(),
        device: socket_metadata.dev(),
        inode: socket_metadata.ino(),
        armed: true,
    };
    let starting = WorkspaceDaemonStarting {
        protocol_version: PROTOCOL_VERSION,
        pid: std::process::id(),
        process_start_identity: process_start_identity(std::process::id()).ok_or_else(|| {
            Error::DaemonUnavailable("workspace daemon process identity is unavailable".into())
        })?,
        executable_identity: executable_identity(&std::env::current_exe()?)?,
        workspace_identity: workspace_identity(&workspace)?,
        owner_nonce: owner_nonce.clone(),
        socket_path: socket_path.clone(),
        socket_device: socket_metadata.dev(),
        socket_inode: socket_metadata.ino(),
    };
    publish_owner_file(
        &authority,
        STARTING_FILE,
        &serde_json::to_vec_pretty(&starting)?,
    )?;
    unpublished_socket.armed = false;
    #[cfg(debug_assertions)]
    if let Ok(delay) = std::env::var("TRAIL_TEST_WORKSPACE_DAEMON_DELAY_AFTER_INTENT_MS") {
        if let Ok(delay) = delay.parse::<u64>() {
            std::thread::sleep(Duration::from_millis(delay));
        }
    }

    let replace_verified_stale_owner =
        std::env::var("TRAIL_WORKSPACE_DAEMON_REPLACE_STALE").as_deref() == Ok("1");
    let ledger_ready = trail::server::prepare_workspace_changed_path_daemon(
        &mut db,
        replace_verified_stale_owner,
    )?;
    #[cfg(debug_assertions)]
    if std::env::var_os("TRAIL_TEST_WORKSPACE_DAEMON_EXIT_AFTER_PREPARE").is_some() {
        std::process::exit(86);
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("TRAIL_TEST_WORKSPACE_DAEMON_ERROR_AFTER_PREPARE").is_some() {
        return Err(Error::DaemonUnavailable(
            "injected ordinary readiness failure after observer ownership".into(),
        ));
    }

    let endpoint = WorkspaceDaemonEndpoint {
        protocol_version: PROTOCOL_VERSION,
        pid: std::process::id(),
        process_start_identity: starting.process_start_identity.clone(),
        executable_identity: starting.executable_identity.clone(),
        workspace_identity: starting.workspace_identity.clone(),
        owner_nonce,
        auth_token: token.clone(),
        socket_path: socket_path.clone(),
        socket_device: socket_metadata.dev(),
        socket_inode: socket_metadata.ino(),
        url: format!("unix://{}", socket_path.display()),
        observer_ready: true,
        recovery_complete: true,
        reconciliation_complete: true,
        live_fence_sequence: ledger_ready.sequence,
        scope_id: ledger_ready.scope_id,
        epoch: ledger_ready.epoch,
        durable_offset: ledger_ready.durable_offset,
        folded_offset: ledger_ready.folded_offset,
    };
    std::env::set_var(
        "TRAIL_WORKSPACE_DAEMON_WORKSPACE_IDENTITY",
        &endpoint.workspace_identity,
    );
    std::env::set_var(
        "TRAIL_WORKSPACE_DAEMON_EXECUTABLE_IDENTITY",
        &endpoint.executable_identity,
    );
    std::env::set_var(
        "TRAIL_WORKSPACE_DAEMON_PROCESS_START_IDENTITY",
        &endpoint.process_start_identity,
    );
    publish_owner_file(
        &authority,
        TOKEN_FILE,
        format!("{}\n", endpoint.auth_token).as_bytes(),
    )?;
    authority.verify_trail_identity(db.db_dir())?;
    publish_owner_file(
        &authority,
        ENDPOINT_FILE,
        &serde_json::to_vec_pretty(&endpoint)?,
    )?;
    unlink_authority_file(&authority, STARTING_FILE)?;
    let mut publication = PublicationGuard {
        authority: authority.try_clone()?,
        socket_path,
        socket_device: endpoint.socket_device,
        socket_inode: endpoint.socket_inode,
        endpoint: endpoint.clone(),
        preserve_stale_identity: false,
    };
    let mut ready = unsafe { File::from_raw_fd(ready_fd) };
    ready.write_all(&[1])?;
    ready.flush()?;
    drop(ready);

    let result = trail::server::serve_unix_listener_with_auth_and_timeout(
        &mut db,
        socket,
        trail::server::ServerAuth::bearer(token)?,
        Duration::from_secs(30),
    );
    if result.as_ref().is_err_and(|error| {
        error
            .to_string()
            .contains("workspace daemon observer health retirement requested")
    }) {
        publication.preserve_stale_identity = true;
    }
    result
}

struct PublicationGuard {
    authority: SecureAuthority,
    socket_path: PathBuf,
    socket_device: u64,
    socket_inode: u64,
    endpoint: WorkspaceDaemonEndpoint,
    preserve_stale_identity: bool,
}

struct BoundSocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
    armed: bool,
}

impl Drop for BoundSocketGuard {
    fn drop(&mut self) {
        if self.armed && verify_secure_socket_identity(&self.path, self.device, self.inode).is_ok()
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Drop for PublicationGuard {
    fn drop(&mut self) {
        if self.preserve_stale_identity {
            return;
        }
        if read_secure_endpoint(&self.authority)
            .ok()
            .flatten()
            .as_ref()
            == Some(&self.endpoint)
            && verify_secure_socket_identity(
                &self.socket_path,
                self.socket_device,
                self.socket_inode,
            )
            .is_ok()
        {
            let _ = unlink_authority_file(&self.authority, ENDPOINT_FILE);
            let _ = unlink_authority_file(&self.authority, TOKEN_FILE);
            let _ = fs::remove_file(&self.socket_path);
        }
    }
}

#[derive(Debug)]
struct SecureAuthority {
    path: PathBuf,
    directory: File,
    trail_directory: File,
    trail_identity: (u64, u64),
}

impl SecureAuthority {
    fn try_clone(&self) -> Result<Self> {
        Ok(Self {
            path: self.path.clone(),
            directory: self.directory.try_clone()?,
            trail_directory: self.trail_directory.try_clone()?,
            trail_identity: self.trail_identity,
        })
    }

    fn verify_trail_identity(&self, db_dir: &Path) -> Result<()> {
        let pinned = self.trail_directory.metadata()?;
        if (pinned.dev(), pinned.ino()) != self.trail_identity {
            return Err(Error::DaemonUnavailable(
                "workspace daemon pinned .trail authority changed identity".into(),
            ));
        }
        let named = open_private_directory(db_dir)?;
        let named = named.metadata()?;
        if (named.dev(), named.ino()) != self.trail_identity {
            return Err(Error::DaemonUnavailable(
                "workspace .trail directory was replaced; refusing daemon pathname authority"
                    .into(),
            ));
        }
        Ok(())
    }
}

fn secure_authority_directory(db_dir: &Path) -> Result<SecureAuthority> {
    let trail_directory = open_private_directory(db_dir)?;
    let trail_metadata = trail_directory.metadata()?;
    let trail_identity = (trail_metadata.dev(), trail_metadata.ino());
    let mut directory = trail_directory.try_clone()?;
    let mut current = db_dir.to_path_buf();
    for component in ["index", "change-ledger"] {
        current.push(component);
        directory = open_or_create_private_child(&directory, component, &current)?;
    }
    Ok(SecureAuthority {
        path: current,
        directory,
        trail_directory,
        trail_identity,
    })
}

fn open_private_directory(path: &Path) -> Result<File> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| {
            Error::DaemonUnavailable(format!(
                "could not open pinned workspace authority directory {}: {error}",
                path.display()
            ))
        })?;
    verify_private_directory(&directory, path)?;
    Ok(directory)
}

fn verify_private_directory(directory: &File, path: &Path) -> Result<()> {
    let metadata = directory.metadata()?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(Error::DaemonUnavailable(format!(
            "workspace daemon authority directory {} has unsafe owner or mode; reinitialize this workspace before using changed-path ledger commands",
            path.display()
        )));
    }
    Ok(())
}

fn open_or_create_private_child(parent: &File, name: &str, path: &Path) -> Result<File> {
    match openat_file(
        parent,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0,
    ) {
        Ok(directory) => {
            verify_private_directory(&directory, path)?;
            Ok(directory)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let leaf = CString::new(name)
                .map_err(|_| Error::InvalidInput("invalid authority leaf".into()))?;
            let created = unsafe { libc::mkdirat(parent.as_raw_fd(), leaf.as_ptr(), 0o700) };
            if created != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(Error::Io(error));
                }
            }
            let directory = openat_file(
                parent,
                name,
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0,
            )
            .map_err(|error| {
                Error::DaemonUnavailable(format!(
                    "could not reopen concurrently created authority directory {}: {error}",
                    path.display()
                ))
            })?;
            verify_private_directory(&directory, path)?;
            Ok(directory)
        }
        Err(error) => Err(Error::DaemonUnavailable(format!(
            "could not open authority directory {}: {error}",
            path.display()
        ))),
    }
}

fn openat_file(parent: &File, name: &str, flags: i32, mode: libc::mode_t) -> std::io::Result<File> {
    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid file leaf"))?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags,
            libc::c_uint::from(mode),
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn secure_open_lock(authority: &SecureAuthority) -> Result<File> {
    let deadline = Instant::now() + Duration::from_millis(250);
    let file = loop {
        match openat_file(
            &authority.directory,
            LOCK_FILE,
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        ) {
            Ok(file) => break file,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => {
                return Err(Error::DaemonUnavailable(format!(
                    "could not securely open workspace daemon lock: {error}"
                )))
            }
        }
    };
    verify_owner_file(&file, &authority.path.join(LOCK_FILE))?;
    Ok(file)
}

fn acquire_or_observe_published_daemon(
    lock: &File,
    authority: &SecureAuthority,
    workspace: &Path,
    requested_token: Option<&str>,
) -> Result<Option<DaemonReady>> {
    let deadline = Instant::now() + LOCK_TIMEOUT;
    loop {
        match flock(lock, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => return Ok(None),
            Err(error) if error == rustix::io::Errno::WOULDBLOCK => {
                if let Some(endpoint) = read_secure_endpoint(authority)? {
                    if let EndpointState::Ready(ready) =
                        classify_endpoint(workspace, authority, &endpoint, requested_token)?
                    {
                        return Ok(Some(ready));
                    }
                }
                if Instant::now() >= deadline {
                    return Err(Error::DaemonUnavailable(
                        "workspace daemon startup lock timed out".into(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(Error::Io(error.into())),
        }
    }
}

fn read_secure_endpoint(authority: &SecureAuthority) -> Result<Option<WorkspaceDaemonEndpoint>> {
    let path = authority.path.join(ENDPOINT_FILE);
    let file = match openat_file(
        &authority.directory,
        ENDPOINT_FILE,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0,
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::Io(error)),
    };
    verify_owner_file(&file, &path)?;
    let mut bytes = Vec::new();
    file.take(64 * 1024).read_to_end(&mut bytes)?;
    let endpoint = serde_json::from_slice(&bytes)
        .map_err(|_| Error::DaemonUnavailable("workspace daemon endpoint is malformed".into()))?;
    Ok(Some(endpoint))
}

fn read_secure_starting(authority: &SecureAuthority) -> Result<Option<WorkspaceDaemonStarting>> {
    let path = authority.path.join(STARTING_FILE);
    let file = match openat_file(
        &authority.directory,
        STARTING_FILE,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0,
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::Io(error)),
    };
    verify_owner_file(&file, &path)?;
    let mut bytes = Vec::new();
    file.take(64 * 1024).read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).map(Some).map_err(|_| {
        Error::DaemonUnavailable("workspace daemon startup intent is malformed".into())
    })
}

fn recover_stale_starting_publication(
    workspace: &Path,
    authority: &SecureAuthority,
) -> Result<bool> {
    let Some(starting) = read_secure_starting(authority)? else {
        return Ok(false);
    };
    let expected_socket = workspace.join(".trail/changed-path.sock");
    if starting.protocol_version != PROTOCOL_VERSION
        || starting.pid == 0
        || starting.process_start_identity.is_empty()
        || starting.executable_identity != executable_identity(&std::env::current_exe()?)?
        || starting.workspace_identity != workspace_identity(workspace)?
        || starting.owner_nonce.len() != 64
        || starting.socket_path != expected_socket
    {
        return Err(Error::DaemonUnavailable(
            "workspace daemon startup identity is unverifiable; refusing replacement".into(),
        ));
    }
    if process_is_alive(starting.pid) {
        match process_start_identity(starting.pid) {
            Some(identity) if identity == starting.process_start_identity => {
                return Err(Error::DaemonUnavailable(
                    "workspace daemon startup owner is still live; refusing replacement".into(),
                ));
            }
            Some(_) => {}
            None => {
                return Err(Error::DaemonUnavailable(
                    "live workspace daemon startup owner cannot be identity-verified; refusing replacement"
                        .into(),
                ));
            }
        }
    }
    match fs::symlink_metadata(&starting.socket_path) {
        Ok(_) => {
            verify_secure_socket_identity(
                &starting.socket_path,
                starting.socket_device,
                starting.socket_inode,
            )?;
            fs::remove_file(&starting.socket_path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::Io(error)),
    }
    unlink_authority_file(authority, STARTING_FILE)?;
    authority.directory.sync_all()?;
    Ok(true)
}

fn read_secure_owner_text(authority: &SecureAuthority, name: &str, limit: u64) -> Result<String> {
    let path = authority.path.join(name);
    let file = openat_file(
        &authority.directory,
        name,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0,
    )
    .map_err(|error| {
        Error::DaemonUnavailable(format!(
            "could not securely open workspace daemon {name}: {error}"
        ))
    })?;
    verify_owner_file(&file, &path)?;
    let mut value = String::new();
    file.take(limit).read_to_string(&mut value)?;
    Ok(value)
}

fn verify_owner_file(file: &File, path: &Path) -> Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(Error::DaemonUnavailable(format!(
            "workspace daemon file {} has unsafe owner or mode",
            path.display()
        )));
    }
    Ok(())
}

fn verify_secure_socket(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        Error::DaemonUnavailable(format!(
            "could not inspect workspace daemon socket {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(Error::DaemonUnavailable(
            "workspace daemon socket has unsafe type, owner, or mode".into(),
        ));
    }
    Ok(())
}

fn verify_secure_socket_identity(
    path: &Path,
    expected_device: u64,
    expected_inode: u64,
) -> Result<()> {
    verify_secure_socket(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        Error::DaemonUnavailable(format!(
            "workspace daemon socket disappeared during identity verification: {error}"
        ))
    })?;
    if metadata.dev() != expected_device || metadata.ino() != expected_inode {
        return Err(Error::DaemonUnavailable(
            "workspace daemon socket identity changed; refusing pathname authority".into(),
        ));
    }
    Ok(())
}

fn publish_owner_file(authority: &SecureAuthority, name: &str, bytes: &[u8]) -> Result<()> {
    let tmp = format!(".{name}.{}.tmp", random_hex(12)?);
    let mut file = openat_file(
        &authority.directory,
        &tmp,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0o600,
    )?;
    file.write_all(bytes)?;
    file.sync_all()?;
    renameat_leaf(&authority.directory, &tmp, name)?;
    authority.directory.sync_all()?;
    Ok(())
}

fn remove_stale_publication(
    authority: &SecureAuthority,
    endpoint: &WorkspaceDaemonEndpoint,
) -> Result<()> {
    match fs::symlink_metadata(&endpoint.socket_path) {
        Ok(_) => verify_secure_socket_identity(
            &endpoint.socket_path,
            endpoint.socket_device,
            endpoint.socket_inode,
        )?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::Io(error)),
    }
    for name in [ENDPOINT_FILE, TOKEN_FILE] {
        unlink_authority_file(authority, name)?;
    }
    remove_if_exists(&endpoint.socket_path)?;
    authority.directory.sync_all()?;
    Ok(())
}

fn renameat_leaf(parent: &File, old: &str, new: &str) -> Result<()> {
    let old =
        CString::new(old).map_err(|_| Error::InvalidInput("invalid publication leaf".into()))?;
    let new =
        CString::new(new).map_err(|_| Error::InvalidInput("invalid publication leaf".into()))?;
    if unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            old.as_ptr(),
            parent.as_raw_fd(),
            new.as_ptr(),
        )
    } != 0
    {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

fn unlink_authority_file(authority: &SecureAuthority, name: &str) -> Result<()> {
    let name =
        CString::new(name).map_err(|_| Error::InvalidInput("invalid authority leaf".into()))?;
    if unsafe { libc::unlinkat(authority.directory.as_raw_fd(), name.as_ptr(), 0) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(Error::Io(error))
    }
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::Io(error)),
    }
}

fn random_hex(bytes: usize) -> Result<String> {
    let mut value = vec![0_u8; bytes];
    getrandom(&mut value).map_err(|error| {
        Error::InvalidInput(format!("workspace daemon entropy failed: {error}"))
    })?;
    Ok(hex::encode(value))
}

fn workspace_identity(workspace: &Path) -> Result<String> {
    let canonical = workspace.canonicalize()?;
    let metadata = fs::metadata(&canonical)?;
    let mut digest = Sha256::new();
    digest.update(canonical.as_os_str().as_encoded_bytes());
    digest.update(metadata.dev().to_be_bytes());
    digest.update(metadata.ino().to_be_bytes());
    Ok(hex::encode(digest.finalize()))
}

fn executable_identity(path: &Path) -> Result<String> {
    let canonical = path.canonicalize()?;
    let mut file = File::open(&canonical)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn process_is_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    let result = unsafe { libc::kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn process_start_identity(pid: u32) -> Option<String> {
    #[cfg(debug_assertions)]
    if std::env::var("TRAIL_TEST_WORKSPACE_DAEMON_UNVERIFIABLE_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        == Some(pid)
    {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let end = stat.rfind(')')?;
        return stat
            .get(end + 2..)?
            .split_whitespace()
            .nth(19)
            .map(|value| format!("linux:{value}"));
    }
    #[cfg(target_os = "macos")]
    {
        let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
        let expected = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        let read = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                expected,
            )
        };
        if read != expected || info.pbi_pid != pid {
            return None;
        }
        return Some(format!(
            "macos:{}:{}:{}",
            info.pbi_pid, info.pbi_start_tvsec, info.pbi_start_tvusec
        ));
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

fn post_challenge_process_start_identity(pid: u32) -> Option<String> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("TRAIL_TEST_WORKSPACE_DAEMON_POST_CHALLENGE_START_IDENTITY") {
        return Some(value);
    }
    process_start_identity(pid)
}

fn required_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| Error::InvalidInput(format!("workspace daemon missing {name}")))
}

fn ready_timeout() -> Duration {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("TRAIL_TEST_WORKSPACE_DAEMON_READY_TIMEOUT_MS") {
        if let Ok(milliseconds) = value.parse::<u64>() {
            return Duration::from_millis(milliseconds.max(1));
        }
    }
    Duration::from_secs(60)
}

pub(super) fn workspace_from_context(ctx: &RuntimeContext) -> Result<PathBuf> {
    ctx.workspace
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| Error::InvalidInput("workspace path is unavailable".into()))
}
