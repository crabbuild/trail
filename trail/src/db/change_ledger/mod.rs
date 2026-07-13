// This module is intentionally dormant until the final activation task wires
// authoritative readers and native producers to it.
#![allow(dead_code)]

mod store;
mod types;

#[allow(unused_imports)]
pub(crate) use store::ChangedPathLedger;
#[allow(unused_imports)]
pub(crate) use types::{
    BaselineIdentity, CandidateSnapshot, DirtyPrefix, EvidenceCut, EvidenceFlags, EvidenceSource,
    ExpectedScope, FilesystemIdentity, LedgerPath, OwnedEvidence, PolicyIdentity,
    ProviderCapabilities, ProviderIdentity, ScopeId, ScopeIdentity, ScopeKind, TrustState,
};
