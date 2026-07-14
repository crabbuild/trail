use super::*;
use crate::db::change_ledger::{
    BaselineIdentity, ChangedPathLedger, EvidenceFlags, EvidenceSource, FilesystemIdentity,
    LedgerPath, PolicyIdentity, ProviderCapabilities, ProviderIdentity, ScopeId, ScopeIdentity,
    ScopeKind,
};
use crate::{ChangeId, InitImportMode, ObjectId, Trail};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const TEST_SCHEMA: &str = "
        PRAGMA foreign_keys = ON;
        CREATE TABLE changed_path_scopes (
            scope_id TEXT PRIMARY KEY,
            epoch INTEGER NOT NULL,
            max_observer_log_bytes INTEGER NOT NULL,
            max_segment_bytes INTEGER NOT NULL,
            max_unfolded_tail_records INTEGER NOT NULL
        );
        CREATE TABLE changed_path_observer_owners (
            scope_id TEXT PRIMARY KEY REFERENCES changed_path_scopes(scope_id),
            epoch INTEGER NOT NULL,
            owner_token TEXT NOT NULL UNIQUE,
            provider_id TEXT NOT NULL,
            provider_identity TEXT NOT NULL,
            lease_state TEXT NOT NULL,
            fence_nonce BLOB,
            acquired_at INTEGER NOT NULL,
            heartbeat_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            error_state TEXT,
            error_at INTEGER,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE changed_path_observer_segments (
            scope_id TEXT NOT NULL,
            epoch INTEGER NOT NULL,
            segment_id TEXT NOT NULL,
            log_format_version INTEGER NOT NULL,
            owner_token TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            first_sequence INTEGER NOT NULL,
            last_sequence INTEGER,
            durable_end_offset INTEGER NOT NULL,
            folded_end_offset INTEGER NOT NULL,
            previous_segment_id TEXT,
            previous_segment_hash TEXT,
            segment_hash TEXT,
            segment_path TEXT NOT NULL,
            state TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            sealed_at INTEGER,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(scope_id, epoch, segment_id)
        );";

struct Fixture {
    _temp: tempfile::TempDir,
    database: std::path::PathBuf,
    segments: std::path::PathBuf,
    scope: ScopeId,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("trail.db");
        let segments = temp.path().join("observer");
        let scope = ScopeId([0x44; 32]);
        let connection = Connection::open(&database).unwrap();
        connection.execute_batch(TEST_SCHEMA).unwrap();
        connection
            .execute(
                "INSERT INTO changed_path_scopes VALUES(?1, 3, ?2, ?3, ?4)",
                params![scope.to_text(), 268_435_456_i64, 16_777_216_i64, 65_536_i64],
            )
            .unwrap();
        Self {
            _temp: temp,
            database,
            segments,
            scope,
        }
    }

    fn acquire(&self, token: [u8; 32]) -> SegmentWriter {
        self.acquire_epoch(3, token).unwrap()
    }

    fn acquire_epoch(&self, epoch: u64, token: [u8; 32]) -> Result<SegmentWriter> {
        SegmentWriter::acquire(
            &self.database,
            &self.segments,
            self.scope,
            epoch,
            token,
            "test-provider",
            b"cursor-0".to_vec(),
            Duration::from_secs(60),
        )
    }

    fn full_v18() -> Self {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let scope = ScopeId([0x44; 32]);
        ChangedPathLedger::new(&db.conn)
            .begin_scope(
                &ScopeIdentity {
                    scope_id: scope,
                    kind: ScopeKind::Workspace,
                    owner_id: "observer-review-fixture".into(),
                },
                &BaselineIdentity {
                    ref_name: "refs/branches/main".into(),
                    ref_generation: 1,
                    change_id: ChangeId("observer-review-change".into()),
                    root_id: ObjectId("observer-review-root".into()),
                },
                &PolicyIdentity {
                    fingerprint: [0x45; 32],
                    generation: 1,
                },
                &FilesystemIdentity(vec![0x46]),
                &ProviderIdentity {
                    identity: vec![0x47],
                    capabilities: ProviderCapabilities {
                        durable_cursor: false,
                        linearizable_fence: false,
                        rename_pairing: true,
                        overflow_scope: true,
                        filesystem_supported: true,
                        clean_proof_allowed: false,
                        power_loss_durability: false,
                    },
                },
            )
            .unwrap();
        Self {
            database: db.db_dir.join(crate::db::DB_RELATIVE_PATH),
            segments: db.db_dir.join("observer-review"),
            scope,
            _temp: temp,
        }
    }

    fn durable_offset(&self) -> u64 {
        Connection::open(&self.database)
            .unwrap()
            .query_row(
                "SELECT durable_end_offset FROM changed_path_observer_segments
                     ORDER BY first_sequence DESC LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap() as u64
    }
}

fn database_snapshot(fixture: &Fixture) -> (Vec<String>, Vec<String>) {
    let connection = Connection::open(&fixture.database).unwrap();
    let mut owners = connection
        .prepare(
            "SELECT printf('%s|%d|%s|%s|%d|%d', scope_id, epoch, owner_token,
                    lease_state, expires_at, updated_at)
             FROM changed_path_observer_owners ORDER BY scope_id",
        )
        .unwrap();
    let owners = owners
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<Vec<String>, _>>()
        .unwrap();
    let mut segments = connection
        .prepare(
            "SELECT printf('%s|%d|%s|%d|%s|%s|%s', scope_id, epoch, segment_id,
                    first_sequence, owner_token, segment_path, state)
             FROM changed_path_observer_segments
             ORDER BY epoch, first_sequence, segment_id",
        )
        .unwrap();
    let segments = segments
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<Vec<String>, _>>()
        .unwrap();
    (owners, segments)
}

fn file_snapshot(fixture: &Fixture) -> Vec<(String, Vec<u8>)> {
    let mut files = fs::read_dir(&fixture.segments)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn event(sequence: u64) -> ObserverRecord {
    ObserverRecord {
        sequence,
        source: EvidenceSource::Observer,
        path: LedgerPath::parse(&format!("src/{sequence}.rs")).unwrap(),
        flags: EvidenceFlags::CONTENT,
        provider_cursor: sequence.to_be_bytes().to_vec(),
    }
}

#[test]
fn torn_tail_recovers_only_through_last_checked_record() {
    let identity = SegmentIdentity::test(ScopeId([7; 32]), 3, [9; 32]);
    let mut bytes = encoded_segment(&identity, &[event(1), event(2)]).unwrap();
    bytes.truncate(bytes.len() - 7);

    let recovered = recover_bytes(&bytes, &identity, PersistedLogLimits::default()).unwrap();

    assert_eq!(recovered.records, vec![event(1)]);
    assert!(recovered.requires_reconciliation);
}

#[test]
fn corrupt_middle_record_fails_closed() {
    let identity = SegmentIdentity::test(ScopeId([7; 32]), 3, [9; 32]);
    let mut bytes = encoded_segment(&identity, &[event(1), event(2)]).unwrap();
    let first = header_end(&bytes).unwrap();
    bytes[first + 16] ^= 0x40;

    let error = recover_bytes(&bytes, &identity, PersistedLogLimits::default()).unwrap_err();

    assert!(error.requires_reconciliation);
}

#[test]
fn payload_over_one_mib_is_rejected_before_append() {
    let mut record = event(1);
    record.provider_cursor = vec![0; MAX_RECORD_PAYLOAD_BYTES + 1];

    assert!(encode_record(&record, [0; 32]).is_err());
}

#[test]
fn recovery_rejects_wrong_identity_non_monotonic_sequences_and_count_caps() {
    let identity = SegmentIdentity::test(ScopeId([7; 32]), 3, [9; 32]);
    let bytes = encoded_segment(&identity, &[event(1), event(2)]).unwrap();
    let wrong = SegmentIdentity::test(ScopeId([8; 32]), 3, [9; 32]);
    assert!(recover_bytes(&bytes, &wrong, PersistedLogLimits::default()).is_err());

    let non_monotonic = encoded_segment(&identity, &[event(2), event(1)]).unwrap();
    assert!(recover_bytes(&non_monotonic, &identity, PersistedLogLimits::default()).is_err());

    let limits = PersistedLogLimits {
        max_unfolded_tail_records: 1,
        ..PersistedLogLimits::default()
    };
    assert!(recover_bytes(&bytes, &identity, limits).is_err());
}

#[test]
fn global_epoch_fences_same_epoch_replacement_without_any_mutation() {
    let fixture = Fixture::full_v18();
    let mut first = fixture.acquire_epoch(1, [1; 32]).unwrap();
    assert!(SegmentWriter::acquire(
        &fixture.database,
        &fixture.segments,
        fixture.scope,
        1,
        [2; 32],
        "test-provider",
        Vec::new(),
        Duration::from_secs(60),
    )
    .is_err());

    let connection = Connection::open(&fixture.database).unwrap();
    for (state, error_state, error_at) in [
        ("revoked", None, None),
        ("expired", None, None),
        ("error", Some("injected-terminal-state"), Some(1_i64)),
    ] {
        connection
            .execute(
                "UPDATE changed_path_observer_owners
                 SET lease_state=?1, error_state=?2, error_at=?3 WHERE scope_id=?4",
                params![state, error_state, error_at, fixture.scope.to_text()],
            )
            .unwrap();
        let database_before = database_snapshot(&fixture);
        let files_before = file_snapshot(&fixture);

        let error = fixture.acquire_epoch(1, [2; 32]).err().unwrap();

        assert!(
            error.to_string().contains("reconciliation"),
            "state {state}"
        );
        assert_eq!(
            database_snapshot(&fixture),
            database_before,
            "state {state}"
        );
        assert_eq!(file_snapshot(&fixture), files_before, "state {state}");
    }
    assert!(first.append(&[event(1)]).is_err());

    connection
        .execute(
            "UPDATE changed_path_scopes SET epoch=2 WHERE scope_id=?1",
            [fixture.scope.to_text()],
        )
        .unwrap();
    let replacement = fixture.acquire_epoch(2, [2; 32]).unwrap();
    drop(replacement);
    let epochs: Vec<(i64, i64)> = Connection::open(&fixture.database)
        .unwrap()
        .prepare(
            "SELECT epoch, first_sequence FROM changed_path_observer_segments
             ORDER BY epoch, first_sequence",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(epochs, vec![(1, 1), (2, 1)]);

    let recovered = recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 2,
            owner_token: [2; 32],
        },
        PersistedLogLimits::default(),
    )
    .unwrap();
    assert!(recovered.records.is_empty());
    assert!(!recovered.requires_reconciliation);

    assert!(!first.is_authorized());
}

#[test]
fn segment_id_is_exactly_derived_from_epoch_sequence_and_full_owner_token() {
    let mut token_b = [0x11; 32];
    token_b[31] = 0x12;
    let first = segment_id(1, 1, [0x11; 32]);
    let different_epoch = segment_id(2, 1, [0x11; 32]);
    let different_token_tail = segment_id(1, 1, token_b);

    assert_eq!(
        first,
        format!("{:020}-{:020}-{}", 1, 1, hex::encode([0x11; 32]))
    );
    assert_ne!(first, different_epoch);
    assert_ne!(first, different_token_tail);
    assert_eq!(segment_filename(&first).unwrap(), format!("{first}.cpl"));
    assert!(segment_filename(&format!("{:020}-{:020}-{}", 0, 1, hex::encode([0x11; 32]))).is_err());
    assert!(segment_filename(&segment_id(1, 1, [0xab; 32]).to_ascii_uppercase()).is_err());
}

#[test]
fn epoch_advance_makes_filesystem_name_unique_even_with_the_same_full_token() {
    let fixture = Fixture::full_v18();
    let token = [0x12; 32];
    drop(fixture.acquire_epoch(1, token).unwrap());
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE changed_path_scopes SET epoch=2 WHERE scope_id=?1",
            [fixture.scope.to_text()],
        )
        .unwrap();
    drop(fixture.acquire_epoch(2, token).unwrap());

    let files = file_snapshot(&fixture);
    assert_eq!(files.len(), 2);
    assert_ne!(files[0].0, files[1].0);
    assert!(files
        .iter()
        .any(|file| file.0.starts_with("00000000000000000001-")));
    assert!(files
        .iter()
        .any(|file| file.0.starts_with("00000000000000000002-")));
}

#[test]
fn invalid_other_epoch_filename_metadata_cannot_suppress_an_orphan() {
    let fixture = Fixture::new();
    let token = [0x16; 32];
    let writer = fixture.acquire(token);
    let other_token = [0x17; 32];
    let other_id = segment_id(2, 1, other_token);
    let orphan_filename = segment_filename(&other_id).unwrap();
    fs::write(fixture.segments.join(&orphan_filename), b"orphan").unwrap();
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id, epoch, segment_id, log_format_version, owner_token,
                 provider_id, first_sequence, last_sequence, durable_end_offset,
                 folded_end_offset, previous_segment_id, previous_segment_hash,
                 segment_hash, segment_path, state, created_at, sealed_at, updated_at)
             SELECT scope_id, 2, ?1, log_format_version, ?2,
                    provider_id, first_sequence, last_sequence, durable_end_offset,
                    folded_end_offset, previous_segment_id, previous_segment_hash,
                    segment_hash, ?3, state, created_at, sealed_at, updated_at
             FROM changed_path_observer_segments WHERE epoch=3",
            params![other_id, hex::encode(other_token), orphan_filename],
        )
        .unwrap();

    let recovered = recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: token,
        },
        PersistedLogLimits::default(),
    )
    .unwrap();
    assert!(recovered.requires_reconciliation);
    drop(writer);
}

#[test]
fn coordinated_segment_id_path_and_file_rename_cannot_defeat_derivation() {
    let fixture = Fixture::new();
    let token = [0x13; 32];
    let writer = fixture.acquire(token);
    let forged_id = segment_id(3, 1, [0x14; 32]);
    let forged_filename = segment_filename(&forged_id).unwrap();
    fs::rename(&writer.path, fixture.segments.join(&forged_filename)).unwrap();
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE changed_path_observer_segments SET segment_id=?1, segment_path=?2",
            params![forged_id, forged_filename],
        )
        .unwrap();

    let error = recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: token,
        },
        PersistedLogLimits::default(),
    )
    .unwrap_err();
    assert!(error.message.contains("derived"));
}

#[test]
fn recovery_fails_closed_for_every_invalid_current_owner_state() {
    let fixture = Fixture::new();
    let token = [0x15; 32];
    let writer = fixture.acquire(token);
    drop(writer);
    let connection = Connection::open(&fixture.database).unwrap();
    let original_expiry: i64 = connection
        .query_row(
            "SELECT expires_at FROM changed_path_observer_owners",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let expected = RecoveryScope {
        scope_id: fixture.scope,
        epoch: 3,
        owner_token: token,
    };
    for mutation in [
        "UPDATE changed_path_observer_owners SET owner_token=lower(hex(zeroblob(32)))",
        "UPDATE changed_path_observer_owners SET epoch=2",
        "UPDATE changed_path_observer_owners SET lease_state='revoked'",
        "UPDATE changed_path_observer_owners SET lease_state='expired'",
        "UPDATE changed_path_observer_owners SET expires_at=heartbeat_at",
        "UPDATE changed_path_observer_owners SET error_state='owner-error', error_at=1",
    ] {
        connection.execute_batch(mutation).unwrap();
        let error = recover_segments(
            &fixture.database,
            &fixture.segments,
            &expected,
            PersistedLogLimits::default(),
        )
        .unwrap_err();
        assert!(error.message.contains("owner"), "mutation: {mutation}");
        connection
            .execute(
                "UPDATE changed_path_observer_owners
                 SET epoch=3, owner_token=?1, lease_state='active', expires_at=?2,
                     error_state=NULL, error_at=NULL",
                params![hex::encode(token), original_expiry],
            )
            .unwrap();
    }
    connection
        .execute("DELETE FROM changed_path_observer_owners", [])
        .unwrap();
    let error = recover_segments(
        &fixture.database,
        &fixture.segments,
        &expected,
        PersistedLogLimits::default(),
    )
    .unwrap_err();
    assert!(error.message.contains("owner"));
}

#[test]
fn append_failure_revokes_writer_and_flush_never_publishes_ahead_of_sync() {
    let fixture = Fixture::new();
    let append_fault = Arc::new(FaultScript::new([FaultPoint::AppendWrite]));
    let mut append_writer = SegmentWriter::acquire_with_faults(
        &fixture.database,
        &fixture.segments,
        fixture.scope,
        3,
        [3; 32],
        "test-provider",
        Vec::new(),
        Duration::from_secs(60),
        append_fault,
    )
    .unwrap();
    assert!(append_writer.append(&[event(1)]).is_err());
    assert!(!append_writer.is_authorized());

    let fixture = Fixture::new();
    let sync_fault = Arc::new(FaultScript::new([FaultPoint::FileSync]));
    let mut sync_writer = SegmentWriter::acquire_with_faults(
        &fixture.database,
        &fixture.segments,
        fixture.scope,
        3,
        [4; 32],
        "test-provider",
        Vec::new(),
        Duration::from_secs(60),
        sync_fault,
    )
    .unwrap();
    sync_writer.append(&[event(1)]).unwrap();
    let durable_before_failed_sync = fixture.durable_offset();
    assert!(sync_writer.flush_durable().is_err());
    assert_eq!(fixture.durable_offset(), durable_before_failed_sync);
    assert!(!sync_writer.is_authorized());
}

#[test]
fn clean_rotation_publishes_hash_lineage_and_recovers_all_records() {
    let fixture = Fixture::new();
    let mut writer = fixture.acquire([5; 32]);
    writer.append(&[event(1)]).unwrap();
    writer.flush_durable().unwrap();
    writer.rotate().unwrap();
    writer.append(&[event(2)]).unwrap();
    writer.flush_durable().unwrap();

    let recovered = recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: [5; 32],
        },
        PersistedLogLimits::default(),
    )
    .unwrap();
    assert_eq!(recovered.records, vec![event(1), event(2)]);
    assert!(!recovered.requires_reconciliation);
}

#[test]
fn every_rotation_publication_fault_retires_the_writer() {
    for point in [
        FaultPoint::RotationOldSync,
        FaultPoint::SealPublication,
        FaultPoint::FirstDirectorySync,
        FaultPoint::NextHeaderCreate,
        FaultPoint::NextHeaderWrite,
        FaultPoint::NextHeaderSync,
        FaultPoint::SecondDirectorySync,
        FaultPoint::NextMetadataPublication,
    ] {
        let fixture = Fixture::full_v18();
        let faults = Arc::new(FaultScript::new([point]));
        let mut writer = SegmentWriter::acquire_with_faults(
            &fixture.database,
            &fixture.segments,
            fixture.scope,
            1,
            [point as u8 + 10; 32],
            "test-provider",
            Vec::new(),
            Duration::from_secs(60),
            faults,
        )
        .unwrap();
        writer.append(&[event(1)]).unwrap();
        writer.flush_durable().unwrap();
        let durable_before = fixture.durable_offset();
        let metadata_before = database_snapshot(&fixture).1;

        assert!(writer.rotate().is_err(), "fault {point:?} did not fail");
        assert!(
            !writer.is_authorized(),
            "fault {point:?} did not retire writer"
        );
        assert!(writer.append(&[event(2)]).is_err());
        assert_eq!(fixture.durable_offset(), durable_before, "fault {point:?}");
        assert_eq!(
            database_snapshot(&fixture).1,
            metadata_before,
            "fault {point:?}"
        );
        let error = recover_segments(
            &fixture.database,
            &fixture.segments,
            &RecoveryScope {
                scope_id: fixture.scope,
                epoch: 1,
                owner_token: [point as u8 + 10; 32],
            },
            PersistedLogLimits::default(),
        )
        .unwrap_err();
        assert!(
            error.requires_reconciliation && error.message.contains("owner"),
            "fault {point:?} did not fail for retired owner: {error:?}"
        );
    }
}

#[test]
fn append_batch_capacity_never_exceeds_persisted_remaining_bytes() {
    let fixture = Fixture::new();
    let token = [0x33; 32];
    let identity = SegmentIdentity::test(fixture.scope, 3, token);
    let header = encode_header(&identity).unwrap().len() as u64;
    let (first, _) = encode_record(&event(1), [0; 32]).unwrap();
    let remaining = first.len() as u64 + 8;
    let cap = header + remaining;
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE changed_path_scopes SET max_observer_log_bytes=?1, max_segment_bytes=?1",
            [cap as i64],
        )
        .unwrap();
    let faults = Arc::new(FaultScript::default());
    let mut writer = SegmentWriter::acquire_with_faults(
        &fixture.database,
        &fixture.segments,
        fixture.scope,
        3,
        token,
        "test-provider",
        Vec::new(),
        Duration::from_secs(60),
        Arc::clone(&faults),
    )
    .unwrap();

    assert!(writer.append(&[event(1), event(2)]).is_err());
    assert!(faults.max_batch_capacity() > 0);
    assert!(faults.max_batch_capacity() as u64 <= remaining);
}

#[test]
fn acquisition_and_rotation_count_headers_against_both_byte_caps() {
    let fixture = Fixture::new();
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE changed_path_scopes SET max_observer_log_bytes=16, max_segment_bytes=16",
            [],
        )
        .unwrap();
    assert!(fixture.acquire_epoch(3, [0x31; 32]).is_err());
    assert!(database_snapshot(&fixture).0.is_empty());
    assert!(!fixture.segments.exists());

    let fixture = Fixture::new();
    let token = [0x32; 32];
    let first_identity = SegmentIdentity::test(fixture.scope, 3, token);
    let first_header = encode_header(&first_identity).unwrap().len() as u64;
    let (record, _) = encode_record(&event(1), [0; 32]).unwrap();
    let mut next_identity = first_identity;
    next_identity.provider_cursor = event(1).provider_cursor;
    next_identity.previous_segment_hash = [1; 32];
    let next_header = encode_header(&next_identity).unwrap().len() as u64;
    let cap = first_header + record.len() as u64 + next_header - 1;
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE changed_path_scopes SET max_observer_log_bytes=?1, max_segment_bytes=?1",
            [cap as i64],
        )
        .unwrap();
    let mut writer = fixture.acquire(token);
    writer.append(&[event(1)]).unwrap();
    let before = database_snapshot(&fixture).1;
    assert!(writer.rotate().is_err());
    assert_eq!(database_snapshot(&fixture).1, before);
}

#[test]
fn version_one_header_is_canonical_and_identity_is_lossless() {
    let fixture = Fixture::new();
    let token = [0xab; 32];
    let writer = fixture.acquire(token);
    let bytes = fs::read(&writer.path).unwrap();
    assert_eq!(&bytes[..8], b"TRAILCPL");
    assert_eq!(u16::from_be_bytes(bytes[8..10].try_into().unwrap()), 1);
    let (header, end) = decode_header(&bytes).unwrap();
    assert_eq!(header.scope_id, fixture.scope);
    assert_eq!(header.owner_token, token);
    assert_eq!(encode_header(&header).unwrap(), bytes[..end]);

    let stored: String = Connection::open(&fixture.database)
        .unwrap()
        .query_row(
            "SELECT owner_token FROM changed_path_observer_owners",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, hex::encode(token));
    assert_eq!(stored, stored.to_ascii_lowercase());

    let mut unsupported = bytes;
    unsupported[9] = 2;
    assert!(decode_header(&unsupported).is_err());
}

#[test]
fn independently_parsed_version_one_record_has_exactly_six_fields_and_checksum() {
    let identity = SegmentIdentity::test(ScopeId([7; 32]), 3, [9; 32]);
    let bytes = encoded_segment(&identity, &[event(1)]).unwrap();
    let record_start = header_end(&bytes).unwrap();
    let body_len =
        u32::from_be_bytes(bytes[record_start..record_start + 4].try_into().unwrap()) as usize;

    assert_eq!(body_len + 4, bytes.len() - record_start);
    assert_eq!(bytes[record_start + 4 + 8], 1);
    let body = &bytes[record_start + 4..];
    let sequence = u64::from_be_bytes(body[..8].try_into().unwrap());
    let source = body[8];
    let payload_end = body.len() - 64;
    let payload = &body[9..payload_end];
    let previous_hash: [u8; 32] = body[payload_end..payload_end + 32].try_into().unwrap();
    let checksum: [u8; 32] = body[payload_end + 32..].try_into().unwrap();
    let independently_calculated: [u8; 32] = Sha256::digest(&body[..payload_end + 32]).into();
    assert_eq!(sequence, 1);
    assert_eq!(source, 1);
    assert_eq!(payload[0], 0x83);
    assert_eq!(previous_hash, [0; 32]);
    assert_eq!(checksum, independently_calculated);
    assert_eq!(
        4 + 8 + 1 + payload.len() + 32 + 32,
        bytes.len() - record_start
    );
}

#[test]
fn independently_crafted_noncanonical_cbor_is_rejected() {
    let identity = SegmentIdentity::test(ScopeId([7; 32]), 3, [9; 32]);
    let canonical = encode_header(&identity).unwrap();
    let payload_start = 14;
    let epoch_offset = payload_start + 1 + 2 + 32;
    assert_eq!(canonical[epoch_offset], 3);
    let mut noncanonical = canonical;
    let canonical_length = u32::from_be_bytes(noncanonical[10..14].try_into().unwrap());
    noncanonical[10..14].copy_from_slice(&(canonical_length + 1).to_be_bytes());
    noncanonical[epoch_offset] = 0x18;
    noncanonical.insert(epoch_offset + 1, 3);
    assert!(decode_header(&noncanonical).is_err());
}

#[test]
fn metadata_paths_are_derived_relative_bounded_and_never_followed() {
    let fixture = Fixture::new();
    let token = [0x71; 32];
    let writer = fixture.acquire(token);
    let filename = writer
        .path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let stored: String = Connection::open(&fixture.database)
        .unwrap()
        .query_row(
            "SELECT segment_path FROM changed_path_observer_segments",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, filename);

    let connection = Connection::open(&fixture.database).unwrap();
    connection
        .execute(
            "UPDATE changed_path_observer_segments SET segment_path='../escape.cpl'",
            [],
        )
        .unwrap();
    assert!(recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: token,
        },
        PersistedLogLimits::default(),
    )
    .is_err());

    connection
        .execute(
            "UPDATE changed_path_observer_segments SET segment_path=?1",
            ["x".repeat(MAX_SEGMENT_FILENAME_BYTES + 1)],
        )
        .unwrap();
    let error = recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: token,
        },
        PersistedLogLimits::default(),
    )
    .unwrap_err();
    assert!(error.message.contains("path") || error.message.contains("filename"));
}

#[cfg(unix)]
#[test]
fn recovery_rejects_symlink_segment_final_component_with_no_follow() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let writer = fixture.acquire([0x72; 32]);
    let path = writer.path.clone();
    let target = fixture._temp.path().join("outside.cpl");
    fs::copy(&path, &target).unwrap();
    drop(writer);
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    let error = recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: [0x72; 32],
        },
        PersistedLogLimits::default(),
    )
    .unwrap_err();
    assert!(error.message.contains("symlink") || error.message.contains("segment"));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_recovery_open_is_compiled_with_no_follow() {
    let fixture = Fixture::new();
    let writer = fixture.acquire([0x73; 32]);
    assert!(open_segment_no_follow(&writer.path).is_ok());
}

#[test]
fn recovery_open_is_read_only_and_does_not_create_a_missing_database() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing.sqlite");
    assert!(recover_segments(
        &missing,
        temp.path(),
        &RecoveryScope {
            scope_id: ScopeId([1; 32]),
            epoch: 1,
            owner_token: [2; 32],
        },
        PersistedLogLimits::default(),
    )
    .is_err());
    assert!(!missing.exists());
}

#[test]
fn read_only_recovery_does_not_create_wal_or_shm_sidecars_to_inspect() {
    let fixture = Fixture::new();
    let writer = fixture.acquire([0x7a; 32]);
    let expected = RecoveryScope {
        scope_id: fixture.scope,
        epoch: 3,
        owner_token: [0x7a; 32],
    };
    drop(writer);
    let wal = PathBuf::from(format!("{}-wal", fixture.database.display()));
    let shm = PathBuf::from(format!("{}-shm", fixture.database.display()));
    if wal.exists() {
        fs::remove_file(&wal).unwrap();
    }
    if shm.exists() {
        fs::remove_file(&shm).unwrap();
    }

    recover_segments(
        &fixture.database,
        &fixture.segments,
        &expected,
        PersistedLogLimits::default(),
    )
    .unwrap();

    assert!(!wal.exists());
    assert!(!shm.exists());
}

#[test]
fn recovery_rejects_segment_count_before_streaming_row_strings_or_records() {
    let fixture = Fixture::new();
    let mut writer = fixture.acquire([0x7b; 32]);
    writer.append(&[event(1)]).unwrap();
    writer.rotate().unwrap();
    drop(writer);
    let error = recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: [0x7b; 32],
        },
        PersistedLogLimits {
            max_unfolded_tail_records: 1,
            ..PersistedLogLimits::default()
        },
    )
    .unwrap_err();
    assert!(error.message.contains("segment count"));
}

#[test]
fn writer_applies_required_sqlite_runtime_pragmas() {
    let fixture = Fixture::new();
    let writer = fixture.acquire([0x74; 32]);
    assert_eq!(writer.runtime_pragmas(), ("wal".into(), 1, 1, 2));
}

#[test]
fn append_post_write_expiry_retires_without_publishing_memory_or_durability() {
    let fixture = Fixture::new();
    let faults = Arc::new(FaultScript::new([FaultPoint::AppendPostWriteLeaseExpiry]));
    let mut writer = SegmentWriter::acquire_with_faults(
        &fixture.database,
        &fixture.segments,
        fixture.scope,
        3,
        [0x75; 32],
        "test-provider",
        Vec::new(),
        Duration::from_secs(60),
        faults,
    )
    .unwrap();
    let offset = fixture.durable_offset();
    assert!(writer.append(&[event(1)]).is_err());
    assert_eq!(fixture.durable_offset(), offset);
    let error = recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: [0x75; 32],
        },
        PersistedLogLimits::default(),
    )
    .unwrap_err();
    assert!(error.requires_reconciliation);
    assert!(error.message.contains("owner"));
}

#[test]
fn flush_rejects_a_synchronized_file_length_different_from_claimed_offset() {
    let fixture = Fixture::new();
    let mut writer = fixture.acquire([0x76; 32]);
    writer.append(&[event(1)]).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&writer.path)
        .unwrap()
        .write_all(b"unclaimed")
        .unwrap();
    let durable = fixture.durable_offset();
    assert!(writer.flush_durable().is_err());
    assert_eq!(fixture.durable_offset(), durable);
}

#[test]
fn directory_sync_runs_on_the_current_host() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("entry"), b"durable-name").unwrap();
    sync_directory(temp.path()).unwrap();
}

#[test]
fn heartbeat_failure_retires_writer_immediately() {
    let fixture = Fixture::new();
    let faults = Arc::new(FaultScript::new([FaultPoint::Heartbeat]));
    let mut writer = SegmentWriter::acquire_with_faults(
        &fixture.database,
        &fixture.segments,
        fixture.scope,
        3,
        [0x55; 32],
        "test-provider",
        Vec::new(),
        Duration::from_secs(60),
        faults,
    )
    .unwrap();

    assert!(writer.heartbeat().is_err());
    assert!(!writer.is_authorized());
    assert!(writer.append(&[event(1)]).is_err());
}

#[test]
fn orphan_next_header_requires_reconciliation() {
    let fixture = Fixture::new();
    let mut writer = fixture.acquire([0x66; 32]);
    writer.append(&[event(1)]).unwrap();
    writer.flush_durable().unwrap();
    fs::write(fixture.segments.join("orphan.cpl"), b"TRAILCPL").unwrap();

    let recovered = recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: [0x66; 32],
        },
        PersistedLogLimits::default(),
    )
    .unwrap();

    assert_eq!(recovered.records, vec![event(1)]);
    assert!(recovered.requires_reconciliation);
}

#[test]
fn broken_segment_lineage_fails_closed() {
    let fixture = Fixture::new();
    let mut writer = fixture.acquire([0x77; 32]);
    writer.append(&[event(1)]).unwrap();
    writer.rotate().unwrap();
    writer.append(&[event(2)]).unwrap();
    writer.flush_durable().unwrap();
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE changed_path_observer_segments SET previous_segment_hash=?1
                 WHERE previous_segment_id IS NOT NULL",
            [hex::encode([0xff; 32])],
        )
        .unwrap();

    assert!(recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: [0x77; 32],
        },
        PersistedLogLimits::default(),
    )
    .is_err());
}

#[test]
fn trailing_bytes_in_a_sealed_middle_segment_fail_closed() {
    let fixture = Fixture::new();
    let mut writer = fixture.acquire([0x79; 32]);
    writer.append(&[event(1)]).unwrap();
    writer.rotate().unwrap();
    let sealed_path: String = Connection::open(&fixture.database)
        .unwrap()
        .query_row(
            "SELECT segment_path FROM changed_path_observer_segments WHERE state='sealed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    OpenOptions::new()
        .append(true)
        .open(fixture.segments.join(sealed_path))
        .unwrap()
        .write_all(b"corrupt-middle")
        .unwrap();

    assert!(recover_segments(
        &fixture.database,
        &fixture.segments,
        &RecoveryScope {
            scope_id: fixture.scope,
            epoch: 3,
            owner_token: [0x79; 32],
        },
        PersistedLogLimits::default(),
    )
    .is_err());
}

#[test]
fn append_enforces_persisted_total_log_byte_cap() {
    let fixture = Fixture::new();
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE changed_path_scopes
                 SET max_observer_log_bytes=12000, max_segment_bytes=8000",
            [],
        )
        .unwrap();
    let mut writer = fixture.acquire([0x88; 32]);
    let mut first = event(1);
    first.path = LedgerPath::parse(&format!("first-{}", "a".repeat(6000))).unwrap();
    writer.append(&[first]).unwrap();
    writer.rotate().unwrap();
    let mut second = event(2);
    second.path = LedgerPath::parse(&format!("second-{}", "b".repeat(6000))).unwrap();

    assert!(writer.append(&[second]).is_err());
    assert!(!writer.is_authorized());
}

#[test]
fn writer_sql_matches_the_fresh_v18_schema() {
    let workspace = tempfile::tempdir().unwrap();
    Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
    let db = Trail::open(workspace.path()).unwrap();
    let scope = ScopeId([0x91; 32]);
    ChangedPathLedger::new(&db.conn)
        .begin_scope(
            &ScopeIdentity {
                scope_id: scope,
                kind: ScopeKind::Workspace,
                owner_id: "writer-full-schema".into(),
            },
            &BaselineIdentity {
                ref_name: "refs/branches/main".into(),
                ref_generation: 1,
                change_id: ChangeId("change-writer-schema".into()),
                root_id: ObjectId("root-writer-schema".into()),
            },
            &PolicyIdentity {
                fingerprint: [0x92; 32],
                generation: 1,
            },
            &FilesystemIdentity(vec![0x93]),
            &ProviderIdentity {
                identity: vec![0x94],
                capabilities: ProviderCapabilities {
                    durable_cursor: false,
                    linearizable_fence: false,
                    rename_pairing: true,
                    overflow_scope: true,
                    filesystem_supported: true,
                    clean_proof_allowed: false,
                    power_loss_durability: false,
                },
            },
        )
        .unwrap();
    let database = db.db_dir.join(crate::db::DB_RELATIVE_PATH);
    let segments = db.db_dir.join("observer-test");
    let mut writer = SegmentWriter::acquire(
        &database,
        &segments,
        scope,
        1,
        [0x95; 32],
        "full-schema-provider",
        Vec::new(),
        Duration::from_secs(60),
    )
    .unwrap();

    writer.append(&[event(1)]).unwrap();
    writer.flush_durable().unwrap();
    writer.rotate().unwrap();
}
