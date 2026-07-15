#![recursion_limit = "512"]

//! Trail core library.
//!
//! Trail is a local-first operation database for code and text worktrees. It
//! records meaningful operations, preserves stable file and line identity, and
//! gives humans and coding lanes a safe branch/provenance layer above Git.

pub mod acp;
pub mod agent_hooks;
pub mod db;
pub mod error;
pub mod ids;
pub mod mcp;
pub mod model;
pub mod server;

pub use db::{InitImportMode, Trail};
pub use error::{Error, Result};
pub use ids::{AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId};
pub use model::*;

#[cfg(debug_assertions)]
#[doc(hidden)]
pub mod test_support {
    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_real_apfs_file_events() -> std::result::Result<(), String> {
        crate::db::run_macos_real_apfs_file_events()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_gap_flag_matrix() -> std::result::Result<(), String> {
        crate::db::run_macos_gap_flag_matrix()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_continuity_fault_matrix() -> std::result::Result<(), String> {
        crate::db::run_macos_continuity_fault_matrix()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_fence_ordering() -> std::result::Result<(), String> {
        crate::db::run_macos_fence_ordering()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_paused_callback_fence() -> std::result::Result<(), String> {
        crate::db::run_macos_paused_callback_fence()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_history_authority() -> std::result::Result<(), String> {
        crate::db::run_macos_history_authority()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_startup_cancellation() -> std::result::Result<(), String> {
        crate::db::run_macos_startup_cancellation()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_malformed_callbacks() -> std::result::Result<(), String> {
        crate::db::run_macos_malformed_callbacks()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_root_revalidation_failures() -> std::result::Result<(), String> {
        crate::db::run_macos_root_revalidation_failures()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_null_context_generation() -> std::result::Result<(), String> {
        crate::db::run_macos_null_context_generation()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_uuid_revalidation() -> std::result::Result<(), String> {
        crate::db::run_macos_uuid_revalidation()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_recursive_coverage() -> std::result::Result<(), String> {
        crate::db::run_recursive_coverage()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_reconciliation_interval_qualification(
    ) -> std::result::Result<(), String> {
        crate::db::run_reconciliation_interval_qualification()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_content_mode_create_delete() -> std::result::Result<(), String> {
        crate::db::run_content_mode_create_delete()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_rename_matrix() -> std::result::Result<(), String> {
        crate::db::run_rename_matrix()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_rename_storm_and_cookie_expiry() -> std::result::Result<(), String> {
        crate::db::run_rename_storm_and_cookie_expiry()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_delayed_backlog() -> std::result::Result<(), String> {
        crate::db::run_delayed_backlog()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_fence_ordering() -> std::result::Result<(), String> {
        crate::db::run_fence_ordering()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_fault_revocation_matrix() -> std::result::Result<(), String> {
        crate::db::run_fault_revocation_matrix()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_owner_death_and_root_replacement() -> std::result::Result<(), String>
    {
        crate::db::run_owner_death_and_root_replacement()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_process_owner_child(root: &str) -> std::result::Result<(), String> {
        crate::db::run_process_owner_child(root)
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_complete_prefix_publication_races() -> std::result::Result<(), String>
    {
        crate::db::run_complete_prefix_publication_races()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_authenticated_fence_rejections() -> std::result::Result<(), String> {
        crate::db::run_authenticated_fence_rejections()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_segment_writer_reconcile_publication(
    ) -> std::result::Result<(), String> {
        crate::db::run_segment_writer_reconcile_publication()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_raw_decoder_faults() -> std::result::Result<(), String> {
        crate::db::run_raw_decoder_faults()
    }

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_policy_dependency_observation() -> std::result::Result<(), String> {
        crate::db::run_policy_dependency_observation()
    }

    pub fn changed_path_reconciliation_oracle() -> std::result::Result<(), String> {
        crate::db::run_oracle()
    }

    pub fn changed_path_reconciliation_races() -> std::result::Result<(), String> {
        crate::db::run_races()
    }

    pub fn changed_path_reconciliation_callback_spool() -> std::result::Result<(), String> {
        crate::db::run_callback_spool()
    }

    pub fn changed_path_intent_acknowledgement_race() -> std::result::Result<(), String> {
        crate::db::run_acknowledgement_race()
    }

    pub fn changed_path_intent_gc_root_lifecycle() -> std::result::Result<(), String> {
        crate::db::run_gc_root_lifecycle()
    }

    pub fn changed_path_intent_crash_matrix() -> std::result::Result<(), String> {
        crate::db::run_crash_matrix()
    }

    pub fn changed_path_backup_restore_rotation() -> std::result::Result<(), String> {
        crate::db::run_backup_restore_rotation()
    }

    pub fn changed_path_qualified_proof_revalidation() -> std::result::Result<(), String> {
        crate::db::run_qualified_proof_revalidation()
    }

    pub fn changed_path_ambiguous_recovery_gate() -> std::result::Result<(), String> {
        crate::db::run_ambiguous_recovery_gate()
    }

    pub fn changed_path_backup_overwrite_rollback() -> std::result::Result<(), String> {
        crate::db::run_backup_overwrite_rollback()
    }

    pub fn changed_path_retirement_barrier() -> std::result::Result<(), String> {
        crate::db::run_retirement_barrier()
    }

    pub fn changed_path_lane_deletion_retirement() -> std::result::Result<(), String> {
        crate::db::run_lane_deletion_retirement()
    }

    pub fn changed_path_missing_sidecar_rejection() -> std::result::Result<(), String> {
        crate::db::run_missing_sidecar_rejection()
    }

    pub fn changed_path_advanced_prefix_recovery() -> std::result::Result<(), String> {
        crate::db::run_advanced_prefix_recovery()
    }

    pub fn changed_path_exact_interval_bridge_rejection() -> std::result::Result<(), String> {
        crate::db::run_exact_interval_bridge_rejection()
    }

    pub fn changed_path_prefix_interval_bridge_rejection() -> std::result::Result<(), String> {
        crate::db::run_prefix_interval_bridge_rejection()
    }

    pub fn changed_path_valid_prefix_interval_recovery() -> std::result::Result<(), String> {
        crate::db::run_valid_prefix_interval_recovery()
    }

    #[cfg(unix)]
    pub fn changed_path_mark_ancestor_substitution_rejection() -> std::result::Result<(), String> {
        crate::db::run_mark_ancestor_substitution_rejection()
    }

    #[cfg(unix)]
    pub fn changed_path_recovery_ancestor_substitution_rejection() -> std::result::Result<(), String>
    {
        crate::db::run_recovery_ancestor_substitution_rejection()
    }

    pub fn changed_path_deletion_parent_substitution_rejection() -> std::result::Result<(), String>
    {
        crate::db::run_deletion_parent_substitution_rejection()
    }

    pub fn changed_path_deletion_post_verification_substitution_rejection(
    ) -> std::result::Result<(), String> {
        crate::db::run_deletion_post_verification_substitution_rejection()
    }

    pub fn changed_path_deletion_post_quarantine_verification_substitution_rejection(
    ) -> std::result::Result<(), String> {
        crate::db::run_deletion_post_quarantine_verification_substitution_rejection()
    }

    pub fn changed_path_deletion_retry_hostile_quarantine_replacement_rejection(
    ) -> std::result::Result<(), String> {
        crate::db::run_deletion_retry_hostile_quarantine_replacement_rejection()
    }

    pub fn changed_path_deletion_normal_retry_idempotence() -> std::result::Result<(), String> {
        crate::db::run_deletion_normal_retry_idempotence()
    }

    pub fn changed_path_retained_writer_quiescence() -> std::result::Result<(), String> {
        crate::db::run_retained_writer_quiescence()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn changed_path_orphan_quarantine_substitution_rejection() -> std::result::Result<(), String>
    {
        crate::db::run_orphan_quarantine_substitution_rejection()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn changed_path_empty_orphan_quarantine_rejection() -> std::result::Result<(), String> {
        crate::db::run_empty_orphan_quarantine_rejection()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn changed_path_no_orphan_quarantine_allocation() -> std::result::Result<(), String> {
        crate::db::run_no_orphan_quarantine_allocation()
    }

    pub fn changed_path_deletion_quiesced_missing_quarantine_rejection(
    ) -> std::result::Result<(), String> {
        crate::db::run_deletion_quiesced_missing_quarantine_rejection()
    }

    pub fn changed_path_deletion_quiesced_reappeared_original_rejection(
    ) -> std::result::Result<(), String> {
        crate::db::run_deletion_quiesced_reappeared_original_rejection()
    }

    pub fn changed_path_restored_nullable_provider_lane_deletion() -> std::result::Result<(), String>
    {
        crate::db::run_restored_nullable_provider_lane_deletion()
    }

    #[cfg(unix)]
    pub fn changed_path_non_utf_database_path_mark_recover_and_retire(
    ) -> std::result::Result<(), String> {
        crate::db::run_non_utf_database_path_mark_recover_and_retire()
    }

    #[cfg(unix)]
    pub fn changed_path_deletion_leaf_substitution_rejection() -> std::result::Result<(), String> {
        crate::db::run_deletion_leaf_substitution_rejection()
    }
}

/// Re-export the prolly crate as a Trail module namespace.
pub use ::prolly;

/// Compatibility module for callers that prefer the explicit prolly-tree name.
pub mod prolly_tree {
    pub use ::prolly::*;
}

/// Common imports for Trail consumers.
pub mod prelude {
    pub use crate::{Actor, Error, InitImportMode, PatchDocument, Result, Trail};
    pub use ::prolly::{Config, MemStore, Prolly, Store, Tree};
}
