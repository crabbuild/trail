use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde_cbor::Value;
use sha2::{Digest, Sha256};

use super::*;

fn source_tag(source: EvidenceSource) -> u8 {
    match source {
        EvidenceSource::Observer => 1,
        EvidenceSource::Intent => 2,
        EvidenceSource::Reconciliation => 3,
        EvidenceSource::GitAdvisory => 4,
    }
}

fn parse_source(tag: u8) -> std::result::Result<EvidenceSource, RecoveryError> {
    match tag {
        1 => Ok(EvidenceSource::Observer),
        2 => Ok(EvidenceSource::Intent),
        3 => Ok(EvidenceSource::Reconciliation),
        4 => Ok(EvidenceSource::GitAdvisory),
        _ => Err(RecoveryError::new("unknown observer record source")),
    }
}

fn cbor_bytes(value: Value) -> std::result::Result<Vec<u8>, RecoveryError> {
    serde_cbor::to_vec(&value).map_err(|error| RecoveryError::new(error.to_string()))
}

pub(super) fn encode_header(
    identity: &SegmentIdentity,
) -> std::result::Result<Vec<u8>, RecoveryError> {
    if identity.epoch == 0 {
        return Err(RecoveryError::new(
            "observer segment epoch must be positive",
        ));
    }
    let payload = cbor_bytes(Value::Array(vec![
        Value::Bytes(identity.scope_id.0.to_vec()),
        Value::Integer(identity.epoch.into()),
        Value::Bytes(identity.owner_token.to_vec()),
        Value::Bytes(identity.provider_cursor.clone()),
        Value::Bytes(identity.previous_segment_hash.to_vec()),
    ]))?;
    if payload.len() > MAX_HEADER_BYTES {
        return Err(RecoveryError::new("observer segment header exceeds limit"));
    }
    let mut bytes = Vec::with_capacity(SEGMENT_MAGIC.len() + 2 + 4 + payload.len());
    bytes.extend_from_slice(SEGMENT_MAGIC);
    bytes.extend_from_slice(&LOG_FORMAT_VERSION.to_be_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

pub(super) fn decode_header(
    bytes: &[u8],
) -> std::result::Result<(SegmentIdentity, usize), RecoveryError> {
    const PREFIX: usize = 8 + 2 + 4;
    if bytes.len() < PREFIX {
        return Err(RecoveryError::new("partial observer segment header"));
    }
    if &bytes[..8] != SEGMENT_MAGIC {
        return Err(RecoveryError::new("invalid observer segment magic"));
    }
    let format = u16::from_be_bytes([bytes[8], bytes[9]]);
    if format != LOG_FORMAT_VERSION {
        return Err(RecoveryError::new("unsupported observer segment format"));
    }
    let length = u32::from_be_bytes(bytes[10..14].try_into().unwrap()) as usize;
    if length > MAX_HEADER_BYTES {
        return Err(RecoveryError::new("observer segment header exceeds limit"));
    }
    let end = PREFIX
        .checked_add(length)
        .ok_or_else(|| RecoveryError::new("observer segment header length overflow"))?;
    if end > bytes.len() {
        return Err(RecoveryError::new("partial observer segment header"));
    }
    let payload = &bytes[PREFIX..end];
    let value: Value = serde_cbor::from_slice(payload)
        .map_err(|error| RecoveryError::new(format!("invalid observer header CBOR: {error}")))?;
    if cbor_bytes(value.clone())? != payload {
        return Err(RecoveryError::new("non-canonical observer header CBOR"));
    }
    let Value::Array(fields) = value else {
        return Err(RecoveryError::new("invalid observer header shape"));
    };
    if fields.len() != 5 {
        return Err(RecoveryError::new("invalid observer header field count"));
    }
    let scope_id = fixed_bytes::<32>(&fields[0], "scope identity")?;
    let epoch = unsigned(&fields[1], "epoch")?;
    if epoch == 0 {
        return Err(RecoveryError::new(
            "observer segment epoch must be positive",
        ));
    }
    let owner_token = fixed_bytes::<32>(&fields[2], "owner token")?;
    let provider_cursor = byte_string(&fields[3], "provider cursor")?;
    let previous_segment_hash = fixed_bytes::<32>(&fields[4], "previous segment hash")?;
    Ok((
        SegmentIdentity {
            scope_id: ScopeId(scope_id),
            epoch,
            owner_token,
            provider_cursor,
            previous_segment_hash,
        },
        end,
    ))
}

fn fixed_bytes<const N: usize>(
    value: &Value,
    label: &str,
) -> std::result::Result<[u8; N], RecoveryError> {
    let bytes = byte_string(value, label)?;
    bytes
        .try_into()
        .map_err(|_| RecoveryError::new(format!("invalid {label} length")))
}

fn byte_string(value: &Value, label: &str) -> std::result::Result<Vec<u8>, RecoveryError> {
    match value {
        Value::Bytes(bytes) => Ok(bytes.clone()),
        _ => Err(RecoveryError::new(format!("invalid {label} encoding"))),
    }
}

fn unsigned(value: &Value, label: &str) -> std::result::Result<u64, RecoveryError> {
    match value {
        Value::Integer(value) => (*value)
            .try_into()
            .map_err(|_| RecoveryError::new(format!("invalid {label}"))),
        _ => Err(RecoveryError::new(format!("invalid {label} encoding"))),
    }
}

fn encode_payload(record: &ObserverRecord) -> std::result::Result<Vec<u8>, RecoveryError> {
    if record.flags.0 < 0 {
        return Err(RecoveryError::new("observer flags cannot be negative"));
    }
    let payload = cbor_bytes(Value::Array(vec![
        Value::Text(record.path.as_str().to_owned()),
        Value::Integer(record.flags.0.into()),
        Value::Bytes(record.provider_cursor.clone()),
    ]))?;
    if payload.len() > MAX_RECORD_PAYLOAD_BYTES {
        return Err(RecoveryError::new("observer record payload exceeds 1 MiB"));
    }
    Ok(payload)
}

pub(super) fn encode_record(
    record: &ObserverRecord,
    previous_hash: [u8; 32],
) -> std::result::Result<(Vec<u8>, [u8; 32]), RecoveryError> {
    if record.sequence == 0 {
        return Err(RecoveryError::new("observer sequence must be positive"));
    }
    let payload = encode_payload(record)?;
    let body_len = RECORD_FIXED_BYTES
        .checked_add(payload.len())
        .ok_or_else(|| RecoveryError::new("observer record length overflow"))?;
    let mut body = Vec::with_capacity(body_len);
    body.extend_from_slice(&record.sequence.to_be_bytes());
    body.push(source_tag(record.source));
    body.extend_from_slice(&payload);
    body.extend_from_slice(&previous_hash);
    let checksum: [u8; 32] = Sha256::digest(&body).into();
    body.extend_from_slice(&checksum);
    let mut framed = Vec::with_capacity(LENGTH_PREFIX_BYTES + body.len());
    framed.extend_from_slice(&(body.len() as u32).to_be_bytes());
    framed.extend_from_slice(&body);
    Ok((framed, checksum))
}

fn decode_record(
    body: &[u8],
    expected_previous_hash: [u8; 32],
) -> std::result::Result<(ObserverRecord, [u8; 32]), RecoveryError> {
    if body.len() < RECORD_FIXED_BYTES {
        return Err(RecoveryError::new(
            "observer record is shorter than fixed fields",
        ));
    }
    let sequence = u64::from_be_bytes(body[..8].try_into().unwrap());
    if sequence == 0 {
        return Err(RecoveryError::new("observer sequence must be positive"));
    }
    let source = parse_source(body[8])?;
    let payload_len = body.len() - RECORD_FIXED_BYTES;
    if payload_len > MAX_RECORD_PAYLOAD_BYTES {
        return Err(RecoveryError::new("observer record payload exceeds 1 MiB"));
    }
    let expected_len = RECORD_FIXED_BYTES
        .checked_add(payload_len)
        .ok_or_else(|| RecoveryError::new("observer record length overflow"))?;
    if body.len() != expected_len {
        return Err(RecoveryError::new(
            "observer record length does not match payload",
        ));
    }
    let payload_end = 9 + payload_len;
    let previous_hash: [u8; 32] = body[payload_end..payload_end + 32].try_into().unwrap();
    if previous_hash != expected_previous_hash {
        return Err(RecoveryError::new("broken observer record hash linkage"));
    }
    let checksum: [u8; 32] = body[payload_end + 32..].try_into().unwrap();
    let calculated: [u8; 32] = Sha256::digest(&body[..payload_end + 32]).into();
    if checksum != calculated {
        return Err(RecoveryError::new("observer record checksum mismatch"));
    }
    let payload = &body[9..payload_end];
    let value: Value = serde_cbor::from_slice(payload)
        .map_err(|error| RecoveryError::new(format!("invalid observer record CBOR: {error}")))?;
    if cbor_bytes(value.clone())? != payload {
        return Err(RecoveryError::new("non-canonical observer record CBOR"));
    }
    let Value::Array(fields) = value else {
        return Err(RecoveryError::new("invalid observer payload shape"));
    };
    if fields.len() != 3 {
        return Err(RecoveryError::new("invalid observer payload field count"));
    }
    let Value::Text(path) = &fields[0] else {
        return Err(RecoveryError::new("invalid observer path encoding"));
    };
    let path = LedgerPath::parse(path)
        .map_err(|error| RecoveryError::new(format!("invalid observer path: {error}")))?;
    let flags = unsigned(&fields[1], "observer flags")?;
    let flags = i64::try_from(flags)
        .map(EvidenceFlags)
        .map_err(|_| RecoveryError::new("observer flags exceed supported range"))?;
    let provider_cursor = byte_string(&fields[2], "provider cursor")?;
    Ok((
        ObserverRecord {
            sequence,
            source,
            path,
            flags,
            provider_cursor,
        },
        checksum,
    ))
}

pub(super) fn recover_bytes(
    bytes: &[u8],
    expected: &SegmentIdentity,
    limits: PersistedLogLimits,
) -> std::result::Result<RecoveredTail, RecoveryError> {
    let limits = limits.validate()?;
    if bytes.len() as u64 > limits.max_segment_bytes || bytes.len() as u64 > limits.max_log_bytes {
        return Err(RecoveryError::new(
            "observer segment exceeds persisted byte limit",
        ));
    }
    let (actual, mut offset) = decode_header(bytes)?;
    if actual != *expected {
        return Err(RecoveryError::new(
            "observer segment identity does not match expected lease",
        ));
    }
    let durable_start = offset as u64;
    let mut records = Vec::new();
    let mut previous_hash = [0; 32];
    let mut last_sequence = 0;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        if remaining < LENGTH_PREFIX_BYTES {
            return Ok(RecoveredTail {
                records,
                durable_end: offset as u64,
                last_sequence,
                last_hash: previous_hash,
                requires_reconciliation: true,
            });
        }
        let body_len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if !(RECORD_FIXED_BYTES..=RECORD_FIXED_BYTES + MAX_RECORD_PAYLOAD_BYTES).contains(&body_len)
        {
            return Err(RecoveryError::new(
                "observer record length exceeds protocol bound",
            ));
        }
        let end = offset
            .checked_add(LENGTH_PREFIX_BYTES)
            .and_then(|value| value.checked_add(body_len))
            .ok_or_else(|| RecoveryError::new("observer record length overflow"))?;
        if end > bytes.len() {
            return Ok(RecoveredTail {
                records,
                durable_end: offset as u64,
                last_sequence,
                last_hash: previous_hash,
                requires_reconciliation: true,
            });
        }
        if records.len() == limits.max_unfolded_tail_records {
            return Err(RecoveryError::new(
                "observer tail record count exceeds persisted limit",
            ));
        }
        let (record, hash) = decode_record(&bytes[offset + 4..end], previous_hash)?;
        if record.sequence <= last_sequence {
            return Err(RecoveryError::new(
                "observer sequence is not strictly monotonic",
            ));
        }
        last_sequence = record.sequence;
        previous_hash = hash;
        records.push(record);
        offset = end;
    }
    Ok(RecoveredTail {
        records,
        durable_end: if offset as u64 == durable_start {
            durable_start
        } else {
            offset as u64
        },
        last_sequence,
        last_hash: previous_hash,
        requires_reconciliation: false,
    })
}

#[cfg(test)]
pub(super) fn encoded_segment(
    identity: &SegmentIdentity,
    records: &[ObserverRecord],
) -> std::result::Result<Vec<u8>, RecoveryError> {
    let mut bytes = encode_header(identity)?;
    let mut previous_hash = [0; 32];
    for record in records {
        let (encoded, hash) = encode_record(record, previous_hash)?;
        bytes.extend_from_slice(&encoded);
        previous_hash = hash;
    }
    Ok(bytes)
}

#[cfg(test)]
pub(super) fn header_end(bytes: &[u8]) -> std::result::Result<usize, RecoveryError> {
    decode_header(bytes).map(|(_, end)| end)
}

pub(crate) fn recover_segments(
    database_path: &Path,
    segment_directory: &Path,
    expected: &RecoveryScope,
    limits: PersistedLogLimits,
) -> std::result::Result<RecoveredTail, RecoveryError> {
    let limits = limits.validate()?;
    let connection = Connection::open(database_path)
        .map_err(|error| RecoveryError::new(format!("open observer control database: {error}")))?;
    let mut statement = connection
        .prepare(
            "SELECT segment_id, owner_token, first_sequence, last_sequence,
                    durable_end_offset, previous_segment_id, previous_segment_hash,
                    segment_hash, segment_path, state
             FROM changed_path_observer_segments
             WHERE scope_id = ?1 AND epoch = ?2
             ORDER BY first_sequence, segment_id",
        )
        .map_err(|error| RecoveryError::new(format!("read observer metadata: {error}")))?;
    let epoch = sql_i64(expected.epoch, "observer epoch")
        .map_err(|error| RecoveryError::new(error.to_string()))?;
    let rows = statement
        .query_map(params![expected.scope_id.to_text(), epoch], |row| {
            Ok(SegmentMetadata {
                segment_id: row.get(0)?,
                owner_token: row.get(1)?,
                first_sequence: row.get(2)?,
                last_sequence: row.get(3)?,
                durable_end_offset: row.get(4)?,
                previous_segment_id: row.get(5)?,
                previous_segment_hash: row.get(6)?,
                segment_hash: row.get(7)?,
                segment_path: PathBuf::from(row.get::<_, String>(8)?),
                state: row.get(9)?,
            })
        })
        .map_err(|error| RecoveryError::new(format!("query observer metadata: {error}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| RecoveryError::new(format!("decode observer metadata: {error}")))?;
    if rows.is_empty() {
        return Ok(RecoveredTail {
            records: Vec::new(),
            durable_end: 0,
            last_sequence: 0,
            last_hash: [0; 32],
            requires_reconciliation: true,
        });
    }

    let expected_owner = hex::encode(expected.owner_token);
    let mut published_paths = HashSet::new();
    let mut total_bytes = 0_u64;
    for row in &rows {
        if row.owner_token != expected_owner {
            return Err(RecoveryError::new(
                "observer segment metadata has wrong owner",
            ));
        }
        let durable = db_u64(row.durable_end_offset, "durable observer offset")?;
        if durable > limits.max_segment_bytes {
            return Err(RecoveryError::new(
                "observer segment exceeds persisted byte limit",
            ));
        }
        total_bytes = total_bytes
            .checked_add(durable)
            .ok_or_else(|| RecoveryError::new("observer log byte count overflow"))?;
        if total_bytes > limits.max_log_bytes {
            return Err(RecoveryError::new(
                "observer log exceeds persisted byte limit",
            ));
        }
        published_paths.insert(row.segment_path.clone());
    }

    let mut records = Vec::new();
    let mut durable_end = 0_u64;
    let mut last_sequence = 0_u64;
    let mut last_hash = [0; 32];
    let mut previous_segment_id: Option<String> = None;
    let mut previous_segment_hash = [0; 32];
    let mut requires_reconciliation = false;
    for (index, row) in rows.iter().enumerate() {
        if row.state == "open" && index + 1 != rows.len() {
            return Err(RecoveryError::new(
                "open observer segment appears before the recovered tail",
            ));
        }
        let durable = db_u64(row.durable_end_offset, "durable observer offset")?;
        let metadata = fs::metadata(&row.segment_path).map_err(|error| {
            RecoveryError::new(format!("read observer segment metadata: {error}"))
        })?;
        if metadata.len() > limits.max_segment_bytes || metadata.len() > limits.max_log_bytes {
            return Err(RecoveryError::new(
                "observer segment file exceeds persisted byte limit",
            ));
        }
        if durable > metadata.len() {
            return Err(RecoveryError::new(
                "durable observer offset exceeds segment length",
            ));
        }
        if metadata.len() > durable && row.state == "sealed" {
            return Err(RecoveryError::new(
                "sealed observer segment contains unpublished trailing bytes",
            ));
        }
        if metadata.len() > durable {
            requires_reconciliation = true;
        }
        let durable_usize = usize::try_from(durable)
            .map_err(|_| RecoveryError::new("durable observer offset cannot fit memory"))?;
        let mut bytes = Vec::with_capacity(durable_usize);
        File::open(&row.segment_path)
            .and_then(|file| file.take(durable).read_to_end(&mut bytes))
            .map_err(|error| RecoveryError::new(format!("read observer segment: {error}")))?;
        if bytes.len() != durable_usize {
            return Err(RecoveryError::new(
                "observer segment shortened during recovery",
            ));
        }
        let (identity, _) = decode_header(&bytes)?;
        if identity.recovery_scope() != *expected {
            return Err(RecoveryError::new(
                "observer segment identity does not match expected lease",
            ));
        }
        if identity.previous_segment_hash != previous_segment_hash {
            return Err(RecoveryError::new("broken observer segment header lineage"));
        }
        if row.previous_segment_id != previous_segment_id {
            return Err(RecoveryError::new(
                "broken observer segment metadata lineage",
            ));
        }
        let metadata_previous_hash = decode_optional_hash(row.previous_segment_hash.as_deref())?;
        if metadata_previous_hash != previous_segment_hash {
            return Err(RecoveryError::new("broken observer segment metadata hash"));
        }
        let recovered = recover_bytes(&bytes, &identity, limits)?;
        if recovered.requires_reconciliation && index + 1 != rows.len() {
            return Err(RecoveryError::new(
                "partial observer record before final segment",
            ));
        }
        if recovered
            .records
            .first()
            .is_some_and(|record| record.sequence <= last_sequence)
        {
            return Err(RecoveryError::new(
                "observer sequence is not monotonic across segments",
            ));
        }
        if records.len().saturating_add(recovered.records.len()) > limits.max_unfolded_tail_records
        {
            return Err(RecoveryError::new(
                "observer tail record count exceeds persisted limit",
            ));
        }
        if let Some(first) = recovered.records.first() {
            if db_u64(row.first_sequence, "first observer sequence")? != first.sequence {
                return Err(RecoveryError::new(
                    "observer segment first sequence metadata mismatch",
                ));
            }
        }
        if let Some(metadata_last) = row.last_sequence {
            if db_u64(metadata_last, "last observer sequence")? != recovered.last_sequence {
                return Err(RecoveryError::new(
                    "observer segment last sequence metadata mismatch",
                ));
            }
        }
        let segment_hash: [u8; 32] = Sha256::digest(&bytes).into();
        if row.state == "sealed" {
            let stored = decode_required_hash(row.segment_hash.as_deref(), "sealed segment hash")?;
            if stored != segment_hash {
                return Err(RecoveryError::new("sealed observer segment hash mismatch"));
            }
        } else if row.state != "open" {
            return Err(RecoveryError::new(
                "observer segment metadata is not recoverable",
            ));
        }
        records.extend(recovered.records);
        durable_end = durable_end
            .checked_add(recovered.durable_end)
            .ok_or_else(|| RecoveryError::new("observer durable offset overflow"))?;
        last_sequence = recovered.last_sequence.max(last_sequence);
        last_hash = recovered.last_hash;
        requires_reconciliation |= recovered.requires_reconciliation;
        previous_segment_id = Some(row.segment_id.clone());
        previous_segment_hash = segment_hash;
    }

    if segment_directory.exists() {
        for entry in fs::read_dir(segment_directory).map_err(|error| {
            RecoveryError::new(format!("list observer segment directory: {error}"))
        })? {
            let path = entry
                .map_err(|error| RecoveryError::new(format!("list observer segment: {error}")))?
                .path();
            if path.extension().is_some_and(|extension| extension == "cpl")
                && !published_paths.contains(&path)
            {
                requires_reconciliation = true;
            }
        }
    }
    Ok(RecoveredTail {
        records,
        durable_end,
        last_sequence,
        last_hash,
        requires_reconciliation,
    })
}

#[derive(Debug)]
struct SegmentMetadata {
    segment_id: String,
    owner_token: String,
    first_sequence: i64,
    last_sequence: Option<i64>,
    durable_end_offset: i64,
    previous_segment_id: Option<String>,
    previous_segment_hash: Option<String>,
    segment_hash: Option<String>,
    segment_path: PathBuf,
    state: String,
}

fn decode_optional_hash(value: Option<&str>) -> std::result::Result<[u8; 32], RecoveryError> {
    match value {
        Some(value) => decode_required_hash(Some(value), "segment hash"),
        None => Ok([0; 32]),
    }
}

fn decode_required_hash(
    value: Option<&str>,
    label: &str,
) -> std::result::Result<[u8; 32], RecoveryError> {
    let value = value.ok_or_else(|| RecoveryError::new(format!("missing {label}")))?;
    if value != value.to_ascii_lowercase() {
        return Err(RecoveryError::new(format!("non-canonical {label}")));
    }
    let bytes = hex::decode(value).map_err(|_| RecoveryError::new(format!("invalid {label}")))?;
    bytes
        .try_into()
        .map_err(|_| RecoveryError::new(format!("invalid {label} length")))
}

fn db_u64(value: i64, label: &str) -> std::result::Result<u64, RecoveryError> {
    value
        .try_into()
        .map_err(|_| RecoveryError::new(format!("negative {label}")))
}
