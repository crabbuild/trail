use super::*;
use crate::db::util::*;

mod control;
mod gates;
mod identity;
mod initialization;
mod initialization_owner;
mod leases;
mod lifecycle;
pub(crate) mod managed_execution;
#[cfg(debug_assertions)]
pub(crate) use initialization_owner::steal_owner_on_next_heartbeat_for_current_thread;
#[cfg(debug_assertions)]
pub(crate) use initialization_owner::{
    clear_process_liveness_overrides, install_process_liveness_unknown_override,
};
#[cfg(debug_assertions)]
pub(crate) use lifecycle::set_lane_initialization_wait_timeout_for_current_thread;
pub(crate) use managed_execution::HotAccessCapture;
mod patch_diff;
mod patch_edits;
mod patch_policy;
mod patching;
mod readiness;
mod retirement;
mod rewind;
mod source_export;
mod turns;
mod workdir;
// Phase-one artifact contracts are intentionally reachable only from their
// qualification tests until explicit resolve and CAS publication operations
// activate them in later OpenSpec tasks.
#[allow(dead_code)]
mod workspace_artifact;
pub(crate) use workspace_artifact::validate_artifact_validation_receipt;
mod workspace_cargo;
mod workspace_cmake;
mod workspace_environment;
mod workspace_git;
mod workspace_go;
mod workspace_layer;
mod workspace_node;
mod workspace_oci;
mod workspace_plugin;
mod workspace_python;
mod workspace_recipe;
mod workspace_runtime;
mod workspace_runtime_toolchain;
mod workspace_view;

#[cfg(debug_assertions)]
pub(crate) use lifecycle::{
    set_lane_association_failure_for_current_thread,
    set_lane_initialization_io_failure_for_current_thread,
    set_lane_initialization_materialization_barrier_for_current_thread,
    set_sparse_selection_write_failure_for_current_thread,
};
#[cfg(all(debug_assertions, unix))]
pub(crate) use workdir::run_changed_path_view_flow;
pub(crate) use workdir::ViewMutationBarrier;
#[cfg(debug_assertions)]
pub(crate) use workdir::{
    install_lane_record_after_c2_write_for_current_thread,
    set_lane_record_postcommit_failure_for_current_thread,
};
pub(crate) use workspace_layer::{
    EnvironmentLayerActivation, EnvironmentLayerOutputActivation, WorkspaceLayerBinding,
};
pub(crate) use workspace_view::WorkspaceMountLease;
