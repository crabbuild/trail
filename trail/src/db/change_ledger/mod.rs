// This module is intentionally dormant until the final activation task wires
// authoritative readers and native producers to it.
#![allow(dead_code)]

mod log;
mod policy;
mod reconcile;
mod store;
mod types;

#[allow(unused_imports)]
pub(crate) use log::{
    recover_segments, DurableCut, ObserverRecord, PersistedLogLimits, RecoveredTail, RecoveryError,
    RecoveryScope, SegmentWriter,
};
#[allow(unused_imports)]
pub(crate) use policy::{
    compile_policy, raw_event_invalidates_policy, raw_path_may_invalidate_policy,
    validate_policy_manifest, AdapterEquivalence, CompiledPolicy, PolicyCompileContext,
    PolicyDependency, PolicyDependencyKind, PolicyDependencyMetrics, PolicyInvalidationIndex,
    PolicyManifest, PolicyManifestValidation, QualifiedPolicyObserverCut, RecordingPolicySnapshot,
};
#[allow(unused_imports)]
pub(crate) use reconcile::{
    begin_reconciliation, persisted_proven_prefixes, reconcile_full, ObserverEvent, ObserverFence,
    ObserverQualification, ProvenPrefixSet, QualifiedObserver, ReconcileMode,
    ReconciliationAttempt,
};
#[allow(unused_imports)]
pub(crate) use store::ChangedPathLedger;
#[allow(unused_imports)]
pub(crate) use types::{
    BaselineIdentity, CandidateSnapshot, DirtyPrefix, EvidenceCut, EvidenceFlags, EvidenceSource,
    ExpectedScope, FilesystemIdentity, LedgerPath, OwnedEvidence, PolicyIdentity,
    ProviderCapabilities, ProviderIdentity, ScopeId, ScopeIdentity, ScopeKind, TrustState,
};
