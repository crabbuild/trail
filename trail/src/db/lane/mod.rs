use super::*;
use crate::db::util::*;

mod control;
mod gates;
mod identity;
mod initialization;
pub(crate) use initialization::backfill_lane_initializations_v19;
#[cfg(any(test, debug_assertions))]
pub(crate) use initialization::{
    clear_schema_v19_backfill_times, install_schema_v18_authenticated_lane_evidence,
    install_schema_v19_backfill_times, schema_v19_backfill_times_remaining,
};
mod leases;
mod lifecycle;
mod patch_diff;
mod patch_edits;
mod patch_policy;
mod patching;
mod readiness;
mod rewind;
mod turns;
mod workdir;
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
mod workspace_view;

#[cfg(debug_assertions)]
pub(crate) use lifecycle::{
    lane_initialization_publication_lock_count, reset_lane_initialization_publication_lock_count,
    set_lane_association_failure_for_current_thread,
    set_lane_initialization_io_failure_for_current_thread,
    set_sparse_selection_write_failure_for_current_thread,
};
pub(crate) use workdir::ViewMutationBarrier;
#[cfg(debug_assertions)]
pub(crate) use workdir::{
    install_lane_record_after_c2_write_for_current_thread, run_changed_path_view_flow,
    set_lane_record_postcommit_failure_for_current_thread,
};
pub(crate) use workspace_layer::{
    EnvironmentLayerActivation, EnvironmentLayerOutputActivation, WorkspaceLayerBinding,
};
pub(crate) use workspace_view::WorkspaceMountLease;
