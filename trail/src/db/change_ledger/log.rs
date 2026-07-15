use std::fmt;

use super::types::{EvidenceFlags, EvidenceSource, LedgerPath, ScopeId};
use crate::error::{Error, Result};

mod codec;
mod writer;

pub(crate) use codec::authenticate_segment_for_deletion;
#[cfg(test)]
pub(crate) use codec::recover_segments;
pub(crate) use codec::recover_segments_from_directory;
pub(crate) use writer::SegmentWriter;

#[cfg(all(test, target_os = "linux"))]
use codec::open_segment_no_follow;
#[cfg(test)]
use codec::{
    decode_header, encode_header, encode_record, encoded_segment, header_end, recover_bytes,
};
#[cfg(test)]
use writer::{segment_filename, segment_id, sync_directory, FaultPoint, FaultScript};

#[cfg(test)]
mod tests;

const SEGMENT_MAGIC: &[u8; 8] = b"TRAILCPL";
const LOG_FORMAT_VERSION: u16 = 1;
const MAX_HEADER_BYTES: usize = 1024 * 1024;
const MAX_RECORD_PAYLOAD_BYTES: usize = 1024 * 1024;
const RECORD_FIXED_BYTES: usize = 8 + 1 + 32 + 32;
const LENGTH_PREFIX_BYTES: usize = 4;
const MAX_SEGMENT_FILENAME_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObserverRecord {
    pub(crate) sequence: u64,
    pub(crate) source: EvidenceSource,
    pub(crate) path: LedgerPath,
    pub(crate) flags: EvidenceFlags,
    pub(crate) provider_cursor: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DurableCut {
    pub(crate) segment_id: String,
    pub(crate) durable_end_offset: u64,
    pub(crate) last_sequence: u64,
    pub(crate) last_hash: [u8; 32],
    pub(crate) provider_cursor: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObserverWriterBinding {
    pub(crate) owner_token: String,
    pub(crate) provider_id: String,
    pub(crate) provider_identity: Vec<u8>,
    pub(crate) fence_nonce: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PersistedLogLimits {
    pub(crate) max_log_bytes: u64,
    pub(crate) max_segment_bytes: u64,
    pub(crate) max_unfolded_tail_records: usize,
}

impl Default for PersistedLogLimits {
    fn default() -> Self {
        Self {
            max_log_bytes: 268_435_456,
            max_segment_bytes: 16_777_216,
            max_unfolded_tail_records: 65_536,
        }
    }
}

impl PersistedLogLimits {
    fn validate(self) -> std::result::Result<Self, RecoveryError> {
        if self.max_log_bytes == 0
            || self.max_segment_bytes == 0
            || self.max_segment_bytes > self.max_log_bytes
            || self.max_unfolded_tail_records == 0
        {
            return Err(RecoveryError::new("invalid persisted observer log limits"));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveredTail {
    pub(crate) records: Vec<ObserverRecord>,
    pub(crate) record_boundaries: Vec<AuthenticatedRecordBoundary>,
    pub(crate) durable_end: u64,
    pub(crate) last_sequence: u64,
    pub(crate) last_hash: [u8; 32],
    pub(crate) requires_reconciliation: bool,
    pub(crate) segments: Vec<AuthenticatedSegment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedRecordBoundary {
    pub(crate) segment_id: String,
    pub(crate) sequence: u64,
    pub(crate) durable_end_offset: u64,
    pub(crate) provider_cursor: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedSegment {
    pub(crate) segment_id: String,
    pub(crate) segment_path: String,
    pub(crate) state: String,
    pub(crate) start_cursor: Vec<u8>,
    pub(crate) end_cursor: Vec<u8>,
    pub(crate) first_sequence: u64,
    pub(crate) last_sequence: u64,
    pub(crate) durable_end_offset: u64,
    pub(crate) folded_end_offset: u64,
    pub(crate) segment_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeletionSegmentExpectation {
    pub(crate) scope_id: ScopeId,
    pub(crate) epoch: u64,
    pub(crate) segment_id: String,
    pub(crate) owner_token: [u8; 32],
    pub(crate) first_sequence: u64,
    pub(crate) last_sequence: Option<u64>,
    pub(crate) durable_end_offset: u64,
    pub(crate) previous_segment_hash: [u8; 32],
    pub(crate) stored_segment_hash: Option<[u8; 32]>,
    pub(crate) state: String,
    pub(crate) limits: PersistedLogLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedDeletionSegment {
    pub(crate) file_length: u64,
    pub(crate) file_hash: [u8; 32],
    pub(crate) durable_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryScope {
    pub(crate) scope_id: ScopeId,
    pub(crate) epoch: u64,
    pub(crate) owner_token: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryError {
    pub(crate) message: String,
    pub(crate) requires_reconciliation: bool,
}

impl RecoveryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            requires_reconciliation: true,
        }
    }
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RecoveryError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SegmentIdentity {
    scope_id: ScopeId,
    epoch: u64,
    owner_token: [u8; 32],
    provider_cursor: Vec<u8>,
    previous_segment_hash: [u8; 32],
}

impl SegmentIdentity {
    #[cfg(test)]
    fn test(scope_id: ScopeId, epoch: u64, owner_token: [u8; 32]) -> Self {
        Self {
            scope_id,
            epoch,
            owner_token,
            provider_cursor: Vec::new(),
            previous_segment_hash: [0; 32],
        }
    }

    fn recovery_scope(&self) -> RecoveryScope {
        RecoveryScope {
            scope_id: self.scope_id,
            epoch: self.epoch,
            owner_token: self.owner_token,
        }
    }
}

fn sql_i64(value: u64, label: &str) -> Result<i64> {
    value
        .try_into()
        .map_err(|_| Error::InvalidInput(format!("{label} exceeds SQLite range")))
}
