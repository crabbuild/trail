// This module is intentionally dormant until the final activation task wires
// authoritative readers and native producers to it.
#![allow(dead_code)]

mod intent;
mod log;
mod policy;
mod reconcile;
mod recovery;
mod store;
mod types;

#[allow(unused_imports)]
pub(crate) use intent::{
    mark_filesystem_applied, prepare_intent, publish_intent, IntentEvidence, IntentId,
    IntentProducer, IntentState, IntentTarget, QualifiedFilesystemProof,
};

#[allow(unused_imports)]
pub(crate) use log::{
    recover_segments, AuthenticatedSegment, DurableCut, ObserverRecord, PersistedLogLimits,
    RecoveredTail, RecoveryError, RecoveryScope, SegmentWriter,
};
#[allow(unused_imports)]
pub(crate) use policy::{
    compile_policy, raw_event_invalidates_policy, raw_path_may_invalidate_policy,
    validate_policy_manifest, AdapterEquivalence, CompiledPolicy, PolicyCompileContext,
    PolicyDependency, PolicyDependencyKind, PolicyDependencyMetrics, PolicyInvalidationIndex,
    PolicyManifest, PolicyManifestValidation, RecordingPolicySnapshot,
};
#[allow(unused_imports)]
pub(crate) use reconcile::{
    begin_reconciliation, persisted_proven_prefixes, reconcile_full, ObserverEvent, ObserverFence,
    ProvenPrefixSet, QualifiedObserver, ReconcileMode, ReconciliationAttempt,
};
#[cfg(debug_assertions)]
pub(crate) use reconcile::{run_callback_spool, run_oracle, run_races};
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
    run_gc_root_lifecycle, run_lane_deletion_retirement, run_missing_sidecar_rejection,
    run_qualified_proof_revalidation, run_retirement_barrier,
};
#[allow(unused_imports)]
pub(crate) use store::ChangedPathLedger;
#[allow(unused_imports)]
pub(crate) use types::{
    BaselineIdentity, CandidateSnapshot, DirtyPrefix, EvidenceCut, EvidenceFlags, EvidenceSource,
    ExpectedScope, FilesystemIdentity, LedgerPath, OwnedEvidence, PolicyIdentity,
    ProviderCapabilities, ProviderIdentity, ScopeId, ScopeIdentity, ScopeKind, TrustState,
};
