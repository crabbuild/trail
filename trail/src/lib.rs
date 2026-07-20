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

pub use db::{InitImportMode, LaneSpawnMaterializationAdmission, Trail};
pub use error::{Error, Result};
pub use ids::{AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId};
pub use model::*;

#[cfg(debug_assertions)]
#[doc(hidden)]
pub mod test_support {
    #[allow(dead_code)]
    pub(crate) mod scoped_state {
        include!("test_support/scoped_state.rs");
    }

    pub fn run_workspace_lock_holder(workspace: &std::path::Path) -> Result<(), String> {
        let db = crate::Trail::open(workspace).map_err(|error| error.to_string())?;
        let _lock = crate::Trail::with_write_lock_wait(std::time::Duration::from_secs(10), || {
            db.acquire_write_lock()
        })
        .map_err(|error| error.to_string())?;
        let mut stdout = std::io::stdout().lock();
        std::io::Write::write_all(&mut stdout, b"READY\n").map_err(|error| error.to_string())?;
        std::io::Write::flush(&mut stdout).map_err(|error| error.to_string())?;

        let (released_tx, released_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let result = std::io::copy(&mut stdin, &mut std::io::sink())
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = released_tx.send(result);
        });
        match released_rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(result) => result?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err("workspace lock holder exceeded its maximum hold time".to_string());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("workspace lock holder release monitor disconnected".to_string());
            }
        }
        Ok(())
    }

    pub fn create_schema_v18_fixture(workspace: &std::path::Path) -> Result<(), String> {
        crate::db::create_schema_v18_fixture_for_test(workspace).map_err(|error| error.to_string())
    }

    pub fn fail_schema_v19_migration_after_ddl(db_path: &std::path::Path) {
        crate::db::install_schema_v19_migration_failure(
            db_path,
            crate::db::SchemaV19MigrationBoundary::AfterDdlBeforeUserVersion,
        );
    }

    pub fn clear_schema_v19_migration_failure(db_path: &std::path::Path) {
        crate::db::clear_schema_v19_migration_failure(db_path);
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SchemaV20MigrationBoundary {
        AfterDdlBeforeUserVersion,
    }

    pub fn install_schema_v20_migration_failure(
        db_path: &std::path::Path,
        boundary: SchemaV20MigrationBoundary,
    ) {
        let boundary = match boundary {
            SchemaV20MigrationBoundary::AfterDdlBeforeUserVersion => {
                crate::db::SchemaV20MigrationBoundary::AfterDdlBeforeUserVersion
            }
        };
        crate::db::install_schema_v20_migration_failure(db_path, boundary);
    }

    pub fn clear_schema_v20_migration_failure(db_path: &std::path::Path) {
        crate::db::clear_schema_v20_migration_failure(db_path);
    }

    pub fn install_schema_v19_backfill_times(times: Vec<i64>) {
        crate::db::install_schema_v19_backfill_times(times);
    }

    pub fn clear_schema_v19_backfill_times() {
        crate::db::clear_schema_v19_backfill_times();
    }

    pub fn schema_v19_backfill_times_remaining() -> usize {
        crate::db::schema_v19_backfill_times_remaining()
    }

    pub fn install_schema_v18_authenticated_lane_evidence(
        workspace: &std::path::Path,
        lane_id: &str,
    ) -> Result<(), String> {
        crate::db::install_schema_v18_authenticated_lane_evidence(workspace, lane_id)
            .map_err(|error| error.to_string())
    }

    pub fn changed_path_command_flow() -> std::result::Result<(), String> {
        crate::db::run_command_flow()
    }

    pub fn changed_path_command_long_lock_flow() -> std::result::Result<(), String> {
        crate::db::run_command_long_lock_flow()
    }

    pub fn changed_path_tracked_ignored_candidate_flow() -> std::result::Result<(), String> {
        crate::db::run_tracked_ignored_candidate_flow()
    }

    pub fn changed_path_materialized_lane_snapshot_flow() -> std::result::Result<(), String> {
        crate::db::run_materialized_lane_snapshot_flow()
    }

    pub fn changed_path_materialized_candidate_lifecycle_flow() -> std::result::Result<(), String> {
        crate::db::run_materialized_candidate_lifecycle_flow()
    }

    #[cfg(unix)]
    pub fn changed_path_view_flow() -> std::result::Result<(), String> {
        crate::db::run_changed_path_view_flow()
    }

    pub fn set_sparse_selection_write_failure_for_current_thread(enabled: bool) {
        crate::db::set_sparse_selection_write_failure_for_current_thread(enabled);
    }

    pub fn set_lane_initialization_io_failure_for_current_thread(
        boundary: Option<&'static str>,
        disk_full: bool,
    ) {
        crate::db::set_lane_initialization_io_failure_for_current_thread(boundary, disk_full);
    }

    pub type LaneInitializationMaterializationRelease =
        std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>;
    pub type LaneInitializationMaterializationBarrier = (
        std::sync::mpsc::Sender<()>,
        LaneInitializationMaterializationRelease,
    );

    pub fn set_lane_initialization_materialization_barrier_for_current_thread(
        barrier: Option<LaneInitializationMaterializationBarrier>,
    ) {
        crate::db::set_lane_initialization_materialization_barrier_for_current_thread(barrier);
    }

    pub fn set_lane_association_failure_for_current_thread(boundary: Option<&'static str>) {
        crate::db::set_lane_association_failure_for_current_thread(boundary);
    }

    pub fn set_lane_initialization_wait_timeout_for_current_thread(
        timeout: Option<std::time::Duration>,
    ) {
        crate::db::set_lane_initialization_wait_timeout_for_current_thread(timeout);
    }

    pub fn current_process_start_token() -> String {
        crate::db::current_process_start_token_for_test()
    }

    pub fn steal_lane_initialization_owner_on_next_heartbeat_for_current_thread() {
        crate::db::steal_lane_initialization_owner_on_next_heartbeat_for_current_thread();
    }

    pub fn set_lane_initialization_owner_liveness_unknown_for_current_thread(
        pid: u32,
        start_identity: &str,
    ) {
        crate::db::set_lane_initialization_owner_liveness_unknown_for_current_thread(
            pid,
            start_identity,
        );
    }

    pub fn clear_lane_initialization_owner_liveness_overrides_for_current_thread() {
        crate::db::clear_lane_initialization_owner_liveness_overrides_for_current_thread();
    }

    pub fn install_lane_record_after_c2_write_for_current_thread(
        path: std::path::PathBuf,
        bytes: Vec<u8>,
    ) {
        crate::db::install_lane_record_after_c2_write_for_current_thread(path, bytes);
    }

    pub fn set_lane_record_postcommit_failure_for_current_thread(boundary: Option<&'static str>) {
        crate::db::set_lane_record_postcommit_failure_for_current_thread(boundary);
    }

    pub fn set_changed_path_authority_override(enabled: bool) {
        crate::db::set_command_authority_override(enabled);
    }

    pub fn changed_path_activation_evidence() -> std::result::Result<serde_json::Value, String> {
        let evidence = crate::db::ActivationEvidence::from_checked_build()?;
        serde_json::to_value(evidence).map_err(|error| error.to_string())
    }

    pub fn changed_path_authority_enabled_for(platform: &str) -> std::result::Result<bool, String> {
        let evidence = crate::db::ActivationEvidence::from_checked_build()?;
        Ok(crate::db::ledger_authority_enabled_for(platform, &evidence))
    }

    pub fn changed_path_production_authority_default() -> bool {
        crate::db::LEDGER_AUTHORITY_ENABLED
    }

    pub fn changed_path_git_qualification(
        db: &crate::Trail,
        force_policy_mismatch: bool,
    ) -> std::result::Result<serde_json::Value, String> {
        crate::db::prepare_workspace_daemon(db, true).map_err(|error| error.to_string())?;
        let qualified = db
            .qualified_git_candidates_for_test(force_policy_mismatch)
            .map_err(|error| error.to_string())?;
        serde_json::to_value(qualified).map_err(|error| error.to_string())
    }

    pub fn changed_path_git_full_scan_oracle(
        db: &crate::Trail,
    ) -> std::result::Result<Vec<String>, String> {
        db.git_qualification_full_scan_oracle_for_test()
            .map_err(|error| error.to_string())
    }

    pub fn changed_path_git_command_flow(
        db: &mut crate::Trail,
    ) -> std::result::Result<serde_json::Value, String> {
        crate::db::prepare_workspace_daemon(db, true).map_err(|error| error.to_string())?;
        crate::db::set_command_authority_override(true);
        let result = (|| {
            let status = db.status(None).map_err(|error| error.to_string())?;
            let diff = db
                .diff_dirty(false, false)
                .map_err(|error| error.to_string())?;
            let record = db
                .record(
                    Some("main"),
                    Some("qualified Git evidence command flow".to_string()),
                    crate::Actor::human(),
                    false,
                )
                .map_err(|error| error.to_string())?;
            Ok(serde_json::json!({
                "status": status.changed_paths.into_iter().map(|change| change.path).collect::<Vec<_>>(),
                "diff": diff.files.into_iter().map(|change| change.path).collect::<Vec<_>>(),
                "record": record.changed_paths.into_iter().map(|change| change.path).collect::<Vec<_>>()
            }))
        })();
        crate::db::set_command_authority_override(false);
        result
    }

    pub fn install_git_qualification_after_porcelain_hook(
        hook: impl FnOnce() -> std::result::Result<(), String> + Send + 'static,
    ) {
        crate::db::install_git_qualification_after_porcelain_hook(move || {
            hook().map_err(crate::Error::InvalidInput)
        });
    }

    pub fn install_git_qualification_after_c2_hook(
        hook: impl FnOnce() -> std::result::Result<(), String> + Send + 'static,
    ) {
        crate::db::install_git_qualification_after_c2_hook(move || {
            hook().map_err(crate::Error::InvalidInput)
        });
    }
    #[cfg(target_os = "macos")]
    fn run_macos_integration(
        test: fn() -> std::result::Result<(), String>,
    ) -> std::result::Result<(), String> {
        use std::sync::{Mutex, OnceLock};

        static MACOS_INTEGRATION: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = MACOS_INTEGRATION
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        test()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_real_apfs_file_events() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_real_apfs_file_events)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_gap_flag_matrix() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_gap_flag_matrix)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_continuity_fault_matrix() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_continuity_fault_matrix)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_fence_ordering() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_fence_ordering)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_paused_callback_fence() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_paused_callback_fence)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_history_authority() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_history_authority)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_startup_cancellation() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_startup_cancellation)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_malformed_callbacks() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_malformed_callbacks)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_root_revalidation_failures() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_root_revalidation_failures)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_null_context_generation() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_null_context_generation)
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_uuid_revalidation() -> std::result::Result<(), String> {
        run_macos_integration(crate::db::run_macos_uuid_revalidation)
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
    pub fn changed_path_linux_controlled_fence_queue_ordering() -> std::result::Result<(), String> {
        crate::db::run_controlled_fence_queue_ordering()
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

    #[cfg(target_os = "linux")]
    pub fn changed_path_linux_unsupported_filesystem_rejection() -> std::result::Result<(), String>
    {
        crate::db::run_unsupported_filesystem_rejection()
    }

    #[cfg(target_os = "macos")]
    pub fn changed_path_macos_unsupported_filesystem_rejection() -> std::result::Result<(), String>
    {
        run_macos_integration(crate::db::run_macos_unsupported_filesystem_rejection)
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
