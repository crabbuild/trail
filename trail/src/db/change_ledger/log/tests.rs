use super::*;
use crate::db::change_ledger::{
    BaselineIdentity, ChangedPathLedger, EvidenceFlags, EvidenceSource, FilesystemIdentity,
    LedgerPath, PolicyIdentity, ProviderCapabilities, ProviderIdentity, ScopeId, ScopeIdentity,
    ScopeKind,
};
use crate::{ChangeId, InitImportMode, ObjectId, Trail};
use rusqlite::{params, Connection};
use std::fs::{self, OpenOptions};
use std::io::Write;
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
        SegmentWriter::acquire(
            &self.database,
            &self.segments,
            self.scope,
            3,
            token,
            "test-provider",
            b"cursor-0".to_vec(),
            Duration::from_secs(60),
        )
        .unwrap()
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
fn live_lease_is_exclusive_and_replaced_owner_fails_before_append() {
    let fixture = Fixture::new();
    let mut first = fixture.acquire([1; 32]);
    assert!(SegmentWriter::acquire(
        &fixture.database,
        &fixture.segments,
        fixture.scope,
        3,
        [2; 32],
        "test-provider",
        Vec::new(),
        Duration::from_secs(60),
    )
    .is_err());

    let connection = Connection::open(&fixture.database).unwrap();
    connection
        .execute(
            "UPDATE changed_path_observer_owners
                 SET lease_state = 'revoked', expires_at = 0 WHERE scope_id = ?1",
            [fixture.scope.to_text()],
        )
        .unwrap();
    let _replacement = fixture.acquire([2; 32]);

    assert!(first.append(&[event(1)]).is_err());
    assert!(!first.is_authorized());
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

    let connection = Connection::open(&fixture.database).unwrap();
    connection
        .execute(
            "UPDATE changed_path_observer_owners
                 SET lease_state = 'revoked', expires_at = 0 WHERE scope_id = ?1",
            [fixture.scope.to_text()],
        )
        .unwrap();
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
        let fixture = Fixture::new();
        let faults = Arc::new(FaultScript::new([point]));
        let mut writer = SegmentWriter::acquire_with_faults(
            &fixture.database,
            &fixture.segments,
            fixture.scope,
            3,
            [point as u8 + 10; 32],
            "test-provider",
            Vec::new(),
            Duration::from_secs(60),
            faults,
        )
        .unwrap();
        writer.append(&[event(1)]).unwrap();

        assert!(writer.rotate().is_err(), "fault {point:?} did not fail");
        assert!(
            !writer.is_authorized(),
            "fault {point:?} did not retire writer"
        );
        assert!(writer.append(&[event(2)]).is_err());
    }
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
fn version_one_record_has_only_the_specified_six_fields() {
    let identity = SegmentIdentity::test(ScopeId([7; 32]), 3, [9; 32]);
    let bytes = encoded_segment(&identity, &[event(1)]).unwrap();
    let record_start = header_end(&bytes).unwrap();
    let body_len =
        u32::from_be_bytes(bytes[record_start..record_start + 4].try_into().unwrap()) as usize;

    assert_eq!(body_len + 4, bytes.len() - record_start);
    assert_eq!(bytes[record_start + 4 + 8], 1);
    assert_eq!(bytes[record_start + 4 + 8 + 1], 0x83);
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
        .open(sealed_path)
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
