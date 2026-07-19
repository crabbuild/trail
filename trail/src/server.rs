mod openapi;
mod request_types;
mod route;
mod transport;

pub use openapi::openapi_spec;
pub use transport::{
    handle_http_request, handle_http_request_with_auth, serve_listener, serve_listener_with_auth,
    serve_listener_with_auth_and_rate_limit, serve_listener_with_auth_rate_limit_and_timeout,
    DaemonServerIdentity, HttpResponse, ServerAuth, ServerRateLimit,
};

#[cfg(unix)]
pub use transport::serve_unix_listener_with_auth_and_timeout;

#[derive(Clone, Debug, serde::Serialize)]
pub struct WorkspaceLedgerProof {
    pub scope_id: String,
    pub epoch: u64,
    pub daemon_launch_nonce: String,
    pub sequence: u64,
    pub durable_offset: u64,
    pub folded_offset: u64,
}

fn public_workspace_proof(
    proof: crate::db::WorkspaceDaemonProof,
) -> crate::Result<WorkspaceLedgerProof> {
    Ok(WorkspaceLedgerProof {
        scope_id: proof.scope_id,
        epoch: proof.epoch,
        daemon_launch_nonce: proof.daemon_launch_nonce.ok_or_else(|| {
            crate::Error::DaemonUnavailable(
                "workspace daemon proof is missing its persisted launch binding".into(),
            )
        })?,
        sequence: proof.cut.sequence,
        durable_offset: proof.cut.durable_offset,
        folded_offset: proof.cut.folded_offset,
    })
}

#[derive(serde::Deserialize)]
struct StaleDaemonPublicationEvidence {
    stale_pid: u32,
    process_start_identity: String,
    daemon_launch_nonce: String,
}

#[doc(hidden)]
pub fn prepare_workspace_changed_path_daemon(
    db: &mut crate::Trail,
) -> crate::Result<WorkspaceLedgerProof> {
    let daemon_launch_nonce =
        std::env::var("TRAIL_WORKSPACE_DAEMON_LAUNCH_NONCE").map_err(|_| {
            crate::Error::DaemonUnavailable("workspace daemon launch nonce is absent".into())
        })?;
    let pid = std::process::id();
    let process_start_identity = workspace_daemon_process_start_identity(pid).ok_or_else(|| {
        crate::Error::DaemonUnavailable(
            "workspace daemon process start identity is unavailable".into(),
        )
    })?;
    if daemon_launch_nonce.len() != 64 {
        return Err(crate::Error::DaemonUnavailable(
            "workspace daemon launch nonce is malformed".into(),
        ));
    }
    let published_stale = std::env::var("TRAIL_WORKSPACE_DAEMON_VERIFIED_STALE_OWNER")
        .ok()
        .map(|value| serde_json::from_str::<StaleDaemonPublicationEvidence>(&value))
        .transpose()
        .map_err(|_| {
            crate::Error::DaemonUnavailable(
                "workspace daemon stale publication evidence is malformed".into(),
            )
        })?;
    let verified_stale_owner = match published_stale {
        Some(stale) => verify_stale_workspace_owner_publication(db, stale)?,
        None => verify_persisted_workspace_owner(db)?,
    };
    crate::db::prepare_workspace_daemon_launch(
        db,
        crate::db::WorkspaceDaemonLaunchIdentity {
            nonce: daemon_launch_nonce,
            pid,
            process_start_identity,
        },
        verified_stale_owner,
    )
    .and_then(public_workspace_proof)
}

fn verify_persisted_workspace_owner(
    db: &crate::Trail,
) -> crate::Result<Option<crate::db::VerifiedStaleWorkspaceOwner>> {
    let owner: Option<crate::db::PersistedWorkspaceDaemonOwner> =
        crate::db::persisted_workspace_daemon_owner(db)?;
    let Some(owner) = owner else {
        return Ok(None);
    };
    if workspace_daemon_process_is_alive(owner.stale_pid) {
        match workspace_daemon_process_start_identity(owner.stale_pid) {
            Some(actual) if actual == owner.process_start_identity => {
                return Err(crate::Error::DaemonUnavailable(
                    "persisted workspace daemon owner process is still live; refusing owner replacement"
                        .into(),
                ));
            }
            Some(_) => {}
            None => {
                return Err(crate::Error::DaemonUnavailable(
                    "persisted workspace daemon owner PID is live but its start identity cannot be verified; refusing owner replacement"
                        .into(),
                ));
            }
        }
    }
    Ok(Some(crate::db::VerifiedStaleWorkspaceOwner {
        stale_pid: owner.stale_pid,
        process_start_identity: owner.process_start_identity,
        scope_id: owner.scope_id,
        epoch: owner.epoch,
        observer_owner_token: owner.observer_owner_token,
        daemon_launch_nonce: owner.daemon_launch_nonce,
    }))
}

fn verify_stale_workspace_owner_publication(
    db: &crate::Trail,
    stale: StaleDaemonPublicationEvidence,
) -> crate::Result<Option<crate::db::VerifiedStaleWorkspaceOwner>> {
    let stale_pid = stale.stale_pid;
    let process_start_identity = stale.process_start_identity;
    if stale_pid == 0
        || stale_pid > i32::MAX as u32
        || process_start_identity.is_empty()
        || stale.daemon_launch_nonce.len() != 64
    {
        return Err(crate::Error::DaemonUnavailable(
            "verified stale workspace owner capability is malformed".into(),
        ));
    }
    if workspace_daemon_process_is_alive(stale_pid) {
        match workspace_daemon_process_start_identity(stale_pid) {
            Some(actual) if actual == process_start_identity => {
                return Err(crate::Error::DaemonUnavailable(
                    "verified stale workspace daemon process is still live; refusing owner replacement"
                        .into(),
                ));
            }
            Some(_) => {}
            None => {
                return Err(crate::Error::DaemonUnavailable(
                    "workspace daemon PID is live but its start identity cannot be verified; refusing owner replacement"
                        .into(),
                ));
            }
        }
    }
    crate::db::verified_stale_workspace_owner_for_launch(
        db,
        stale_pid,
        &process_start_identity,
        &stale.daemon_launch_nonce,
    )
}

fn workspace_daemon_process_is_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn workspace_daemon_process_start_identity(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
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
        Some(format!(
            "macos:{}:{}:{}",
            info.pbi_pid, info.pbi_start_tvsec, info.pbi_start_tvusec
        ))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

pub(crate) fn workspace_changed_path_fence(
    db: &mut crate::Trail,
    scope_id: Option<&str>,
    epoch: Option<u64>,
) -> crate::Result<WorkspaceLedgerProof> {
    crate::db::workspace_daemon_fence(db, scope_id, epoch).and_then(public_workspace_proof)
}

pub(crate) fn workspace_changed_path_reconcile(
    db: &mut crate::Trail,
    scope_id: Option<&str>,
    epoch: Option<u64>,
) -> crate::Result<WorkspaceLedgerProof> {
    crate::db::workspace_daemon_reconcile(db, scope_id, epoch).and_then(public_workspace_proof)
}

pub(crate) fn workspace_changed_path_ready_proof(
    db: &crate::Trail,
) -> crate::Result<WorkspaceLedgerProof> {
    crate::db::workspace_daemon_ready_proof(db).and_then(public_workspace_proof)
}

/// Start the scope observer when necessary and perform one full filesystem
/// reconciliation for either the workspace or a materialized lane.
pub fn reconcile_changed_path_ledger(
    db: &mut crate::Trail,
    lane: Option<&str>,
) -> crate::Result<crate::model::ChangeLedgerReconcileReport> {
    let result = match lane {
        Some(lane) => crate::db::materialized_lane_daemon_full_reconcile(db, lane),
        None => crate::db::workspace_daemon_full_reconcile(db),
    };
    result.map_err(|error| match error {
        crate::Error::ChangeLedgerReconcileRequired {
            scope,
            state,
            reason,
            ..
        } => crate::Error::ChangeLedgerReconcileRequired {
            scope,
            state,
            reason,
            command: lane.map_or_else(
                || "trail index reconcile".to_string(),
                |lane| format!("trail index reconcile --lane {}", shell_quote(lane)),
            ),
        },
        error => error,
    })
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}
