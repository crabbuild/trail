use std::ops::{BitOr, BitOrAssign};

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, Result};
use crate::{ChangeId, ObjectId};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TrustState {
    Trusted,
    Reconciling,
    Overflow,
    UntrustedGap,
    StaleBaseline,
    Corrupt,
}

impl TrustState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Reconciling => "reconciling",
            Self::Overflow => "overflow",
            Self::UntrustedGap => "untrusted_gap",
            Self::StaleBaseline => "stale_baseline",
            Self::Corrupt => "corrupt",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "trusted" => Ok(Self::Trusted),
            "reconciling" => Ok(Self::Reconciling),
            "overflow" => Ok(Self::Overflow),
            "untrusted_gap" => Ok(Self::UntrustedGap),
            "stale_baseline" => Ok(Self::StaleBaseline),
            "corrupt" => Ok(Self::Corrupt),
            other => Err(Error::Corrupt(format!(
                "unknown changed-path trust state `{other}`"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProviderCapabilities {
    pub(crate) durable_cursor: bool,
    pub(crate) linearizable_fence: bool,
    pub(crate) rename_pairing: bool,
    pub(crate) overflow_scope: bool,
    pub(crate) filesystem_supported: bool,
    pub(crate) clean_proof_allowed: bool,
    pub(crate) power_loss_durability: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub(crate) struct ScopeId(pub(crate) [u8; 32]);

impl ScopeId {
    pub(crate) fn to_text(self) -> String {
        hex::encode(self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScopeKind {
    Workspace,
    MaterializedLane,
    WorkspaceView,
}

impl ScopeKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::MaterializedLane => "materialized_lane",
            Self::WorkspaceView => "workspace_view",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub(crate) struct LedgerPath(pub(crate) String);

impl LedgerPath {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        let invalid = |reason: &str| Error::InvalidPath {
            path: value.to_string(),
            reason: reason.to_string(),
        };
        if value.is_empty() {
            return Err(invalid("ledger paths cannot be empty"));
        }
        if value.starts_with('/')
            || (value.len() >= 3
                && value.as_bytes()[0].is_ascii_alphabetic()
                && value.as_bytes()[1] == b':'
                && value.as_bytes()[2] == b'/')
        {
            return Err(invalid("ledger paths must be relative"));
        }
        if value.contains('\\') {
            return Err(invalid("ledger paths use `/` separators"));
        }
        if value.contains('\0') {
            return Err(invalid("ledger paths cannot contain NUL"));
        }
        if !value.nfc().eq(value.chars()) {
            return Err(invalid("ledger paths must be Unicode NFC normalized"));
        }
        if value
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(invalid("ledger paths must contain normalized components"));
        }
        Ok(Self(value.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScopeIdentity {
    pub(crate) scope_id: ScopeId,
    pub(crate) kind: ScopeKind,
    pub(crate) owner_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BaselineIdentity {
    pub(crate) ref_name: String,
    pub(crate) ref_generation: u64,
    pub(crate) change_id: ChangeId,
    pub(crate) root_id: ObjectId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyIdentity {
    pub(crate) fingerprint: [u8; 32],
    pub(crate) generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FilesystemIdentity(pub(crate) Vec<u8>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderIdentity {
    pub(crate) identity: Vec<u8>,
    pub(crate) capabilities: ProviderCapabilities,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceSource {
    Observer,
    Intent,
    Reconciliation,
    GitAdvisory,
}

impl EvidenceSource {
    pub(crate) const fn mask(self) -> i64 {
        match self {
            Self::Observer => 1,
            Self::Intent => 2,
            Self::Reconciliation => 4,
            Self::GitAdvisory => 8,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Observer => "observer",
            Self::Intent => "intent",
            Self::Reconciliation => "reconciliation",
            Self::GitAdvisory => "git_advisory",
        }
    }

    #[cfg(test)]
    pub(crate) const fn from_index(index: u8) -> Self {
        match index % 4 {
            0 => Self::Observer,
            1 => Self::Intent,
            2 => Self::Reconciliation,
            _ => Self::GitAdvisory,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct EvidenceFlags(pub(crate) i64);

impl EvidenceFlags {
    pub(crate) const CREATE: Self = Self(1 << 0);
    pub(crate) const CONTENT: Self = Self(1 << 1);
    pub(crate) const MODE: Self = Self(1 << 2);
    pub(crate) const DELETE: Self = Self(1 << 3);
    pub(crate) const RENAME_FROM: Self = Self(1 << 4);
    pub(crate) const RENAME_TO: Self = Self(1 << 5);
    pub(crate) const PROVIDER_COMPLETE_PREFIX: Self = Self(1 << 6);
    /// A controlled projection path is covered by any authenticated native
    /// mutation for that exact path. The pinned comparison after c1 proves the
    /// final bytes/type/mode; requiring CONTENT would reject create, delete,
    /// rename, and chmod-only producers.
    pub(crate) const ANY_MUTATION: Self = Self(
        Self::CREATE.0
            | Self::CONTENT.0
            | Self::MODE.0
            | Self::DELETE.0
            | Self::RENAME_FROM.0
            | Self::RENAME_TO.0,
    );

    #[cfg(test)]
    pub(crate) const fn from_index(index: u8) -> Self {
        match index % 6 {
            0 => Self::CREATE,
            1 => Self::CONTENT,
            2 => Self::MODE,
            3 => Self::DELETE,
            4 => Self::RENAME_FROM,
            _ => Self::RENAME_TO,
        }
    }
}

impl BitOr for EvidenceFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for EvidenceFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct EvidenceCut {
    pub(crate) source: EvidenceSource,
    pub(crate) sequence: u64,
    pub(crate) durable_offset: u64,
    pub(crate) folded_offset: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirtyPrefix {
    pub(crate) path: LedgerPath,
    pub(crate) complete: bool,
    pub(crate) reason: String,
    pub(crate) first_sequence: u64,
    pub(crate) last_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OwnedEvidence {
    pub(crate) source: EvidenceSource,
    pub(crate) through_sequence: u64,
    pub(crate) exact_paths: Vec<LedgerPath>,
    pub(crate) prefixes: Vec<DirtyPrefix>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateSnapshot {
    pub(crate) expected: ExpectedScope,
    pub(crate) cut: EvidenceCut,
    pub(crate) exact_paths: Vec<LedgerPath>,
    pub(crate) prefixes: Vec<DirtyPrefix>,
    /// Immutable row identities captured in the same read transaction as the
    /// candidate set.  A checkpoint may acknowledge a row only when every
    /// field still matches this token; merging newer or differently-owned
    /// evidence therefore always leaves the row pending.
    pub(crate) acknowledgement_tokens: Vec<EvidenceAcknowledgementToken>,
    pub(crate) trust: TrustState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvidenceAcknowledgementToken {
    pub(crate) kind: EvidenceRowKind,
    pub(crate) path: LedgerPath,
    pub(crate) flags: EvidenceFlags,
    pub(crate) source_mask: i64,
    pub(crate) first_sequence: u64,
    pub(crate) last_sequence: u64,
    pub(crate) provider_id: Option<String>,
    pub(crate) provider_sequence: Option<u64>,
    pub(crate) intent_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceRowKind {
    Exact,
    CompletePrefix,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExpectedScope {
    pub(crate) scope_id: ScopeId,
    pub(crate) epoch: u64,
    pub(crate) ref_name: String,
    pub(crate) ref_generation: u64,
    pub(crate) baseline_root: ObjectId,
    pub(crate) policy_fingerprint: [u8; 32],
    pub(crate) policy_generation: u64,
    pub(crate) filesystem_identity: Vec<u8>,
    pub(crate) provider_identity: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExactEvidence {
    pub(crate) path: LedgerPath,
    pub(crate) flags: EvidenceFlags,
    pub(crate) source_mask: i64,
    pub(crate) first_sequence: u64,
    pub(crate) last_sequence: u64,
}
