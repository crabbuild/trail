mod openapi;
mod request_types;
mod route;
mod transport;

pub use openapi::openapi_spec;
pub use transport::{
    handle_http_request, handle_http_request_with_auth, serve_listener, serve_listener_with_auth,
    serve_listener_with_auth_and_rate_limit, serve_listener_with_auth_rate_limit_and_timeout,
    HttpResponse, ServerAuth, ServerRateLimit,
};

#[cfg(unix)]
pub use transport::serve_unix_listener_with_auth_and_timeout;

#[derive(Clone, Debug, serde::Serialize)]
pub struct WorkspaceLedgerProof {
    pub scope_id: String,
    pub epoch: u64,
    pub sequence: u64,
    pub durable_offset: u64,
    pub folded_offset: u64,
}

fn public_workspace_proof(proof: crate::db::WorkspaceDaemonProof) -> WorkspaceLedgerProof {
    WorkspaceLedgerProof {
        scope_id: proof.scope_id,
        epoch: proof.epoch,
        sequence: proof.cut.sequence,
        durable_offset: proof.cut.durable_offset,
        folded_offset: proof.cut.folded_offset,
    }
}

#[doc(hidden)]
pub fn prepare_workspace_changed_path_daemon(
    db: &mut crate::Trail,
    replace_verified_stale_owner: bool,
) -> crate::Result<WorkspaceLedgerProof> {
    crate::db::prepare_workspace_daemon(db, replace_verified_stale_owner)
        .map(public_workspace_proof)
}

pub(crate) fn workspace_changed_path_fence(
    db: &mut crate::Trail,
    scope_id: Option<&str>,
    epoch: Option<u64>,
) -> crate::Result<WorkspaceLedgerProof> {
    crate::db::workspace_daemon_fence(db, scope_id, epoch).map(public_workspace_proof)
}

pub(crate) fn workspace_changed_path_reconcile(
    db: &mut crate::Trail,
    scope_id: Option<&str>,
    epoch: Option<u64>,
) -> crate::Result<WorkspaceLedgerProof> {
    crate::db::workspace_daemon_reconcile(db, scope_id, epoch).map(public_workspace_proof)
}

pub(crate) fn workspace_changed_path_ready_proof(
    db: &crate::Trail,
) -> crate::Result<WorkspaceLedgerProof> {
    crate::db::workspace_daemon_ready_proof(db).map(public_workspace_proof)
}
