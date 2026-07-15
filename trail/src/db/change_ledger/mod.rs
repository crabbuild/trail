// This module is intentionally dormant until the final activation task wires
// authoritative readers and native producers to it.
#![allow(dead_code)]

mod intent;
mod log;
mod observer;
mod policy;
mod reconcile;
mod recovery;
mod secure_fs;
mod store;
mod types;

#[allow(unused_imports)]
pub(crate) use intent::{
    mark_filesystem_applied, prepare_intent, publish_intent, IntentEvidence, IntentId,
    IntentProducer, IntentState, IntentTarget, QualifiedFilesystemProof,
};

#[allow(unused_imports)]
pub(crate) use log::{
    recover_segments_from_directory, AuthenticatedSegment, DurableCut, ObserverRecord,
    ObserverWriterBinding, PersistedLogLimits, RecoveredTail, RecoveryError, RecoveryScope,
    SegmentWriter,
};
#[cfg(all(debug_assertions, target_os = "linux"))]
pub(crate) use observer::linux::{
    run_authenticated_fence_rejections, run_complete_prefix_publication_races,
    run_content_mode_create_delete, run_delayed_backlog, run_fault_revocation_matrix,
    run_fence_ordering, run_owner_death_and_root_replacement, run_policy_dependency_observation,
    run_process_owner_child, run_raw_decoder_faults, run_reconciliation_interval_qualification,
    run_recursive_coverage, run_rename_matrix, run_rename_storm_and_cookie_expiry,
    run_segment_writer_reconcile_publication,
};
#[cfg(all(debug_assertions, target_os = "macos"))]
pub(crate) use observer::macos::{
    run_continuity_fault_matrix as run_macos_continuity_fault_matrix,
    run_fence_ordering as run_macos_fence_ordering,
    run_gap_flag_matrix as run_macos_gap_flag_matrix,
    run_history_authority as run_macos_history_authority,
    run_malformed_callbacks as run_macos_malformed_callbacks,
    run_paused_callback_fence as run_macos_paused_callback_fence,
    run_real_apfs_file_events as run_macos_real_apfs_file_events,
    run_root_revalidation_failures as run_macos_root_revalidation_failures,
    run_startup_cancellation as run_macos_startup_cancellation,
};
#[allow(unused_imports)]
pub(crate) use observer::{select_observer, ObserverFence, ObserverLease, QualifiedObserver};
#[allow(unused_imports)]
pub(crate) use policy::{
    compile_policy, raw_event_invalidates_policy, raw_path_may_invalidate_policy,
    validate_policy_manifest, AdapterEquivalence, CompiledPolicy, PolicyCompileContext,
    PolicyDependency, PolicyDependencyKind, PolicyDependencyMetrics, PolicyInvalidationIndex,
    PolicyManifest, PolicyManifestValidation, RecordingPolicySnapshot,
};
#[cfg(all(debug_assertions, target_os = "linux"))]
pub(crate) use reconcile::install_initial_scan_hook;
#[allow(unused_imports)]
pub(crate) use reconcile::{
    begin_reconciliation, persisted_proven_prefixes, reconcile_full, ObserverEvent,
    ProvenPrefixSet, ReconcileMode, ReconciliationAttempt,
};
#[cfg(debug_assertions)]
pub(crate) use reconcile::{run_callback_spool, run_oracle, run_races};
#[cfg(all(debug_assertions, unix))]
pub(crate) use recovery::run_non_utf_database_path_mark_recover_and_retire;
#[allow(unused_imports)]
pub(crate) use recovery::{
    ledger_gc_roots, mark_backup_scopes_untrusted, recover_scope, remove_retired_segments,
    retire_deletion_scopes, retire_scope, rotate_restored_scopes, IntentGcRoot, RecoveryDecision,
    SegmentDeletionToken,
};
#[cfg(debug_assertions)]
pub(crate) use recovery::{
    run_acknowledgement_race, run_advanced_prefix_recovery, run_ambiguous_recovery_gate,
    run_backup_overwrite_rollback, run_backup_restore_rotation, run_crash_matrix,
    run_deletion_normal_retry_idempotence, run_deletion_parent_substitution_rejection,
    run_deletion_post_quarantine_verification_substitution_rejection,
    run_deletion_post_verification_substitution_rejection,
    run_deletion_quiesced_missing_quarantine_rejection,
    run_deletion_quiesced_reappeared_original_rejection,
    run_deletion_retry_hostile_quarantine_replacement_rejection,
    run_exact_interval_bridge_rejection, run_gc_root_lifecycle, run_lane_deletion_retirement,
    run_missing_sidecar_rejection, run_prefix_interval_bridge_rejection,
    run_qualified_proof_revalidation, run_restored_nullable_provider_lane_deletion,
    run_retained_writer_quiescence, run_retirement_barrier, run_valid_prefix_interval_recovery,
};
#[cfg(all(debug_assertions, unix))]
pub(crate) use recovery::{
    run_deletion_leaf_substitution_rejection, run_mark_ancestor_substitution_rejection,
    run_recovery_ancestor_substitution_rejection,
};
#[cfg(all(debug_assertions, any(target_os = "linux", target_os = "macos")))]
pub(crate) use recovery::{
    run_empty_orphan_quarantine_rejection, run_no_orphan_quarantine_allocation,
    run_orphan_quarantine_substitution_rejection,
};
#[allow(unused_imports)]
pub(crate) use store::ChangedPathLedger;
#[allow(unused_imports)]
pub(crate) use types::{
    BaselineIdentity, CandidateSnapshot, DirtyPrefix, EvidenceCut, EvidenceFlags, EvidenceSource,
    ExpectedScope, FilesystemIdentity, LedgerPath, OwnedEvidence, PolicyIdentity,
    ProviderCapabilities, ProviderIdentity, ScopeId, ScopeIdentity, ScopeKind, TrustState,
};
