use super::*;

const CHANGED_PATH_LEDGER_SCHEMA_VERSION: i64 = 1;
const CHANGED_PATH_OBSERVER_LOG_FORMAT_VERSION: i64 = 1;

pub(super) const CHANGED_PATH_LEDGER_SCHEMA_V18: &str =
        "CREATE TABLE changed_path_scopes (
             scope_id TEXT NOT NULL PRIMARY KEY CHECK (length(scope_id) > 0),
             schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version = 1),
             scope_kind TEXT NOT NULL CHECK (scope_kind IN ('workspace', 'materialized_lane', 'workspace_view')),
             owner_id TEXT NOT NULL CHECK (length(owner_id) > 0),
             scope_root TEXT NOT NULL,
             scope_root_identity TEXT NOT NULL,
             filesystem_identity TEXT NOT NULL,
             filesystem_kind TEXT NOT NULL,
             case_sensitive INTEGER NOT NULL CHECK (case_sensitive IN (0, 1)),
             ref_name TEXT NOT NULL,
             ref_generation INTEGER NOT NULL CHECK (ref_generation >= 0),
             change_id TEXT NOT NULL,
             baseline_root_id TEXT NOT NULL,
             policy_fingerprint TEXT NOT NULL,
             policy_dependency_generation INTEGER NOT NULL CHECK (policy_dependency_generation >= 0),
             trust_state TEXT NOT NULL DEFAULT 'reconciling'
                 CHECK (trust_state IN ('trusted', 'reconciling', 'overflow', 'untrusted_gap', 'stale_baseline', 'corrupt')),
             trust_reason TEXT NOT NULL DEFAULT 'fresh_create',
             continuity_generation INTEGER NOT NULL DEFAULT 1 CHECK (continuity_generation >= 1),
             epoch INTEGER NOT NULL DEFAULT 1 CHECK (epoch >= 1),
             max_candidate_rows INTEGER NOT NULL DEFAULT 250000 CHECK(max_candidate_rows>0),
             max_prefix_rows INTEGER NOT NULL DEFAULT 16384 CHECK(max_prefix_rows>0),
             max_observer_log_bytes INTEGER NOT NULL DEFAULT 268435456 CHECK(max_observer_log_bytes>0),
             max_segment_bytes INTEGER NOT NULL DEFAULT 16777216 CHECK(max_segment_bytes>0 AND max_segment_bytes<=max_observer_log_bytes),
             max_unfolded_tail_records INTEGER NOT NULL DEFAULT 65536 CHECK(max_unfolded_tail_records>0),
             provider_id TEXT,
             provider_identity TEXT,
             durable_cursor INTEGER NOT NULL DEFAULT 0 CHECK (durable_cursor IN (0, 1)),
             linearizable_fence INTEGER NOT NULL DEFAULT 0 CHECK (linearizable_fence IN (0, 1)),
             rename_pairing INTEGER NOT NULL DEFAULT 0 CHECK (rename_pairing IN (0, 1)),
             overflow_scope INTEGER NOT NULL DEFAULT 0 CHECK (overflow_scope IN (0, 1)),
             filesystem_supported INTEGER NOT NULL DEFAULT 0 CHECK (filesystem_supported IN (0, 1)),
             clean_proof_allowed INTEGER NOT NULL DEFAULT 0 CHECK (clean_proof_allowed IN (0, 1)),
             power_loss_durability INTEGER NOT NULL DEFAULT 0 CHECK (power_loss_durability IN (0, 1)),
             provider_cursor BLOB,
             provider_fence BLOB,
             durable_offset INTEGER NOT NULL DEFAULT 0 CHECK (durable_offset >= 0),
             folded_offset INTEGER NOT NULL DEFAULT 0 CHECK (folded_offset >= 0),
             observer_owner_token TEXT,
             observer_heartbeat_at INTEGER,
             observer_error_state TEXT,
             observer_error_at INTEGER,
             retired_at INTEGER,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             UNIQUE (scope_kind, owner_id),
             CHECK (folded_offset <= durable_offset),
             CHECK ((observer_error_state IS NULL AND observer_error_at IS NULL)
                    OR (observer_error_state IS NOT NULL AND observer_error_at IS NOT NULL))
         );
         CREATE TABLE changed_path_intents (
             intent_id TEXT NOT NULL PRIMARY KEY CHECK (length(intent_id) > 0),
             schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version = 1),
             scope_id TEXT NOT NULL
                 REFERENCES changed_path_scopes(scope_id) ON UPDATE CASCADE ON DELETE CASCADE,
             producer TEXT NOT NULL,
             expected_scope_epoch INTEGER NOT NULL CHECK (expected_scope_epoch >= 1),
             expected_ref_name TEXT NOT NULL,
             expected_ref_generation INTEGER NOT NULL CHECK (expected_ref_generation >= 0),
             expected_change_id TEXT NOT NULL,
             expected_root_id TEXT NOT NULL,
             target_change_id TEXT NOT NULL,
             target_root_id TEXT NOT NULL,
             target_operation_id TEXT,
             start_cursor BLOB,
             lifecycle_state TEXT NOT NULL DEFAULT 'prepared'
                 CHECK (lifecycle_state IN ('prepared', 'filesystem_applied', 'published', 'acknowledged', 'aborted')),
             verified_cut BLOB,
             failure_reason TEXT,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL
         );
         CREATE INDEX changed_path_intents_scope_state_idx
             ON changed_path_intents(scope_id, lifecycle_state, updated_at);
         CREATE INDEX changed_path_intents_gc_idx
             ON changed_path_intents(lifecycle_state, target_root_id, target_operation_id);
         CREATE TABLE changed_path_entries (
             scope_id TEXT NOT NULL
                 REFERENCES changed_path_scopes(scope_id) ON UPDATE CASCADE ON DELETE CASCADE,
             normalized_path TEXT COLLATE BINARY NOT NULL CHECK (length(normalized_path) > 0),
             event_flags INTEGER NOT NULL CHECK (event_flags >= 0),
             source_mask INTEGER NOT NULL CHECK (source_mask >= 0),
             first_sequence INTEGER NOT NULL CHECK (first_sequence >= 0),
             last_sequence INTEGER NOT NULL CHECK (last_sequence >= first_sequence),
             provider_id TEXT,
             provider_sequence INTEGER CHECK (provider_sequence IS NULL OR provider_sequence >= 0),
             intent_id TEXT
                 REFERENCES changed_path_intents(intent_id) ON UPDATE CASCADE ON DELETE SET NULL,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             PRIMARY KEY (scope_id, normalized_path)
         );
         CREATE INDEX changed_path_entries_sequence_idx
             ON changed_path_entries(scope_id, last_sequence);
         CREATE INDEX changed_path_entries_provider_idx
             ON changed_path_entries(scope_id, provider_id, provider_sequence);
         CREATE INDEX changed_path_entries_intent_idx
             ON changed_path_entries(intent_id);
         CREATE TABLE changed_path_prefixes (
             scope_id TEXT NOT NULL
                 REFERENCES changed_path_scopes(scope_id) ON UPDATE CASCADE ON DELETE CASCADE,
             normalized_prefix TEXT COLLATE BINARY NOT NULL CHECK (length(normalized_prefix) > 0),
             completeness_reason TEXT NOT NULL,
             event_flags INTEGER NOT NULL CHECK (event_flags >= 0),
             source_mask INTEGER NOT NULL CHECK (source_mask >= 0),
             first_sequence INTEGER NOT NULL CHECK (first_sequence >= 0),
             last_sequence INTEGER NOT NULL CHECK (last_sequence >= first_sequence),
             provider_id TEXT,
             provider_sequence INTEGER CHECK (provider_sequence IS NULL OR provider_sequence >= 0),
             intent_id TEXT
                 REFERENCES changed_path_intents(intent_id) ON UPDATE CASCADE ON DELETE SET NULL,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             PRIMARY KEY (scope_id, normalized_prefix)
         );
         CREATE INDEX changed_path_prefixes_sequence_idx
             ON changed_path_prefixes(scope_id, last_sequence);
         CREATE INDEX changed_path_prefixes_provider_idx
             ON changed_path_prefixes(scope_id, provider_id, provider_sequence);
         CREATE INDEX changed_path_prefixes_intent_idx
             ON changed_path_prefixes(intent_id);
         CREATE TABLE changed_path_policy_dependencies (
             scope_id TEXT NOT NULL
                 REFERENCES changed_path_scopes(scope_id) ON UPDATE CASCADE ON DELETE CASCADE,
             dependency_identity TEXT COLLATE BINARY NOT NULL CHECK(length(dependency_identity)>0),
             dependency_kind TEXT NOT NULL
                 CHECK(dependency_kind IN ('builtin','trail_config','ignore','trailignore','gitignore','git_info_exclude','git_excludes_file','git_config','normalization','mode','case_policy')),
             content_identity BLOB NOT NULL
                 CHECK(typeof(content_identity)='blob' AND length(content_identity)=32),
             metadata_identity BLOB NOT NULL,
             observable INTEGER NOT NULL CHECK(observable IN (0,1)),
             generation INTEGER NOT NULL CHECK(generation>=0),
             last_source_sequence INTEGER NOT NULL DEFAULT 0 CHECK(last_source_sequence>=0),
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             PRIMARY KEY(scope_id,dependency_identity,dependency_kind)
         ) WITHOUT ROWID;
         CREATE INDEX changed_path_policy_dependencies_generation_idx
             ON changed_path_policy_dependencies(scope_id,generation,last_source_sequence);
         CREATE TABLE changed_path_intent_paths (
             intent_id TEXT NOT NULL
                 REFERENCES changed_path_intents(intent_id) ON UPDATE CASCADE ON DELETE CASCADE,
             normalized_path TEXT COLLATE BINARY NOT NULL CHECK (length(normalized_path) > 0),
             event_flags INTEGER NOT NULL CHECK (event_flags >= 0),
             PRIMARY KEY (intent_id, normalized_path)
         );
         CREATE TABLE changed_path_intent_prefixes (
             intent_id TEXT NOT NULL
                 REFERENCES changed_path_intents(intent_id) ON UPDATE CASCADE ON DELETE CASCADE,
             normalized_prefix TEXT COLLATE BINARY NOT NULL CHECK (length(normalized_prefix) > 0),
             completeness_reason TEXT NOT NULL,
             event_flags INTEGER NOT NULL CHECK (event_flags >= 0),
             PRIMARY KEY (intent_id, normalized_prefix)
         );
         CREATE TABLE changed_path_reconciliations (
             attempt_id TEXT NOT NULL PRIMARY KEY CHECK (length(attempt_id) > 0),
             schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version = 1),
             scope_id TEXT NOT NULL
                 REFERENCES changed_path_scopes(scope_id) ON UPDATE CASCADE ON DELETE CASCADE,
             expected_scope_epoch INTEGER NOT NULL CHECK (expected_scope_epoch >= 1),
             expected_ref_name TEXT NOT NULL,
             expected_ref_generation INTEGER NOT NULL CHECK (expected_ref_generation >= 0),
             expected_change_id TEXT NOT NULL,
             expected_root_id TEXT NOT NULL,
             filesystem_identity TEXT NOT NULL,
             policy_fingerprint TEXT NOT NULL,
             policy_dependency_generation INTEGER NOT NULL CHECK (policy_dependency_generation >= 0),
             provider_id TEXT,
             provider_identity TEXT,
             start_cursor BLOB,
             start_fence BLOB,
             mode TEXT NOT NULL CHECK (mode IN ('full', 'prefix')),
             reason TEXT NOT NULL,
             completeness_class TEXT NOT NULL
                 CHECK (completeness_class IN ('complete', 'provider_complete_prefix', 'point_in_time_untrusted')),
             staged_store_location TEXT NOT NULL DEFAULT 'sqlite',
             state TEXT NOT NULL DEFAULT 'prepared'
                 CHECK (state IN ('prepared', 'staging', 'ready', 'published', 'abandoned', 'failed')),
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL
         );
         CREATE INDEX changed_path_reconciliations_scope_state_idx
             ON changed_path_reconciliations(scope_id, state, updated_at);
         CREATE TABLE changed_path_reconciliation_rows (
             attempt_id TEXT NOT NULL
                 REFERENCES changed_path_reconciliations(attempt_id) ON UPDATE CASCADE ON DELETE CASCADE,
             normalized_path TEXT COLLATE BINARY NOT NULL CHECK (length(normalized_path) > 0),
             row_kind TEXT NOT NULL CHECK (row_kind IN ('entry', 'deletion')),
             file_kind TEXT,
             content_hash TEXT,
             executable INTEGER CHECK (executable IS NULL OR executable IN (0, 1)),
             size_bytes INTEGER CHECK (size_bytes IS NULL OR size_bytes >= 0),
             before_identity TEXT,
             after_identity TEXT,
             source_sequence INTEGER CHECK (source_sequence IS NULL OR source_sequence >= 0),
             staged_at INTEGER NOT NULL,
             PRIMARY KEY (attempt_id, normalized_path)
         );
         CREATE INDEX changed_path_reconciliation_rows_kind_idx
             ON changed_path_reconciliation_rows(attempt_id, row_kind, normalized_path COLLATE BINARY);
         CREATE TABLE changed_path_reconciliation_guards (
             attempt_id TEXT NOT NULL
                 REFERENCES changed_path_reconciliations(attempt_id) ON UPDATE CASCADE ON DELETE CASCADE,
             relative_path BLOB NOT NULL CHECK (length(relative_path) > 0),
             directory_identity BLOB NOT NULL CHECK (length(directory_identity) > 0),
             staged_at INTEGER NOT NULL,
             PRIMARY KEY (attempt_id, relative_path)
         ) WITHOUT ROWID;
         CREATE TABLE changed_path_observer_owners (
             scope_id TEXT NOT NULL PRIMARY KEY
                 REFERENCES changed_path_scopes(scope_id) ON UPDATE CASCADE ON DELETE CASCADE,
             epoch INTEGER NOT NULL CHECK (epoch >= 1),
             owner_token TEXT NOT NULL UNIQUE CHECK (length(owner_token) > 0),
             provider_id TEXT NOT NULL,
             provider_identity TEXT NOT NULL,
             lease_state TEXT NOT NULL DEFAULT 'active'
                 CHECK (lease_state IN ('active', 'revoked', 'expired', 'error')),
             fence_nonce BLOB,
             acquired_at INTEGER NOT NULL,
             heartbeat_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             error_state TEXT,
             error_at INTEGER,
             updated_at INTEGER NOT NULL,
             CHECK (acquired_at <= heartbeat_at AND heartbeat_at <= expires_at),
             CHECK ((error_state IS NULL AND error_at IS NULL)
                    OR (error_state IS NOT NULL AND error_at IS NOT NULL))
         );
         CREATE TRIGGER changed_path_observer_owner_fail_closed
         AFTER UPDATE OF lease_state ON changed_path_observer_owners
         WHEN OLD.lease_state = 'active' AND NEW.lease_state <> 'active'
         BEGIN
             UPDATE changed_path_scopes
             SET trust_state = CASE
                     WHEN trust_state IN ('trusted', 'reconciling') THEN 'untrusted_gap'
                     ELSE trust_state
                 END,
                 trust_reason = CASE
                     WHEN trust_state IN ('trusted', 'reconciling')
                         THEN 'observer_owner_' || NEW.lease_state
                     ELSE trust_reason
                 END,
                 continuity_generation = continuity_generation + 1,
                 updated_at = NEW.updated_at
             WHERE scope_id = NEW.scope_id AND epoch = NEW.epoch;
         END;
         CREATE INDEX changed_path_observer_owners_state_idx
             ON changed_path_observer_owners(lease_state, expires_at);
         CREATE TABLE changed_path_observer_segments (
             scope_id TEXT NOT NULL
                 REFERENCES changed_path_scopes(scope_id) ON UPDATE CASCADE ON DELETE CASCADE,
             epoch INTEGER NOT NULL CHECK (epoch >= 1),
             segment_id TEXT NOT NULL CHECK (length(segment_id) > 0),
             log_format_version INTEGER NOT NULL DEFAULT 1 CHECK (log_format_version = 1),
             owner_token TEXT NOT NULL,
             provider_id TEXT NOT NULL,
             first_sequence INTEGER NOT NULL CHECK (first_sequence >= 1),
             last_sequence INTEGER CHECK (last_sequence IS NULL OR last_sequence >= first_sequence),
             durable_end_offset INTEGER NOT NULL DEFAULT 0 CHECK (durable_end_offset >= 0),
             folded_end_offset INTEGER NOT NULL DEFAULT 0 CHECK (folded_end_offset >= 0),
             previous_segment_id TEXT,
             previous_segment_hash TEXT,
             segment_hash TEXT,
             segment_path TEXT NOT NULL,
             state TEXT NOT NULL DEFAULT 'open'
                 CHECK (state IN ('open', 'sealed', 'retiring', 'retired', 'corrupt')),
             retirement_source_state TEXT
                 CHECK (retirement_source_state IS NULL OR
                        retirement_source_state IN ('open','sealed')),
             retirement_file_length INTEGER CHECK (retirement_file_length IS NULL OR retirement_file_length>=0),
             retirement_file_hash TEXT CHECK (retirement_file_hash IS NULL OR length(retirement_file_hash)=64),
             retirement_durable_hash TEXT CHECK (retirement_durable_hash IS NULL OR length(retirement_durable_hash)=64),
             retirement_source_device TEXT,
             retirement_source_inode TEXT,
             created_at INTEGER NOT NULL,
             sealed_at INTEGER,
             updated_at INTEGER NOT NULL,
             PRIMARY KEY (scope_id, epoch, segment_id),
             UNIQUE (scope_id, epoch, first_sequence),
             CHECK (folded_end_offset <= durable_end_offset),
             CHECK ((retirement_source_device IS NULL)=(retirement_source_inode IS NULL)),
             CHECK (state<>'retired' OR
                    (retirement_source_state IS NOT NULL AND retirement_file_length IS NOT NULL
                     AND retirement_file_hash IS NOT NULL AND retirement_durable_hash IS NOT NULL
                     AND retirement_source_device IS NOT NULL))
         );
         CREATE INDEX changed_path_observer_segments_state_idx
             ON changed_path_observer_segments(scope_id, epoch, state, last_sequence);
         CREATE TABLE changed_path_segment_quarantine_allocations (
             attempt_nonce TEXT PRIMARY KEY CHECK (length(attempt_nonce) = 64),
             scope_id TEXT NOT NULL,
             epoch INTEGER NOT NULL CHECK (epoch >= 1),
             segment_id TEXT NOT NULL CHECK (length(segment_id) > 0),
             quarantine_leaf TEXT NOT NULL UNIQUE CHECK (length(quarantine_leaf) > 0),
             scope_directory_device TEXT NOT NULL CHECK (length(scope_directory_device) > 0),
             scope_directory_inode TEXT NOT NULL CHECK (length(scope_directory_inode) > 0),
             identity_policy TEXT NOT NULL
                 CHECK (identity_policy = 'direct_noreplace_same_directory_v1'),
             source_segment_device TEXT NOT NULL CHECK (length(source_segment_device) > 0),
             source_segment_inode TEXT NOT NULL CHECK (length(source_segment_inode) > 0),
             quarantine_device TEXT,
             quarantine_inode TEXT,
             observed_conflict_device TEXT,
             observed_conflict_inode TEXT,
             retained_reason TEXT,
             state TEXT NOT NULL DEFAULT 'allocating'
                 CHECK (state IN ('allocating', 'allocated', 'bound', 'abandoned')),
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             allocated_at INTEGER,
             bound_at INTEGER,
             abandoned_at INTEGER,
             FOREIGN KEY (scope_id, epoch, segment_id)
                 REFERENCES changed_path_observer_segments(scope_id, epoch, segment_id)
                 ON UPDATE CASCADE ON DELETE CASCADE,
             CHECK ((quarantine_device IS NULL) = (quarantine_inode IS NULL)),
             CHECK (quarantine_device IS NULL OR
                    (quarantine_device=source_segment_device AND
                     quarantine_inode=source_segment_inode)),
             CHECK ((observed_conflict_device IS NULL) =
                    (observed_conflict_inode IS NULL)),
             CHECK (state NOT IN ('allocated', 'bound') OR
                    (quarantine_device IS NOT NULL AND allocated_at IS NOT NULL)),
             CHECK ((state = 'bound' AND bound_at IS NOT NULL) OR
                    (state <> 'bound' AND bound_at IS NULL)),
             CHECK ((state = 'abandoned' AND retained_reason IS NOT NULL
                      AND abandoned_at IS NOT NULL) OR
                    (state <> 'abandoned' AND retained_reason IS NULL
                      AND abandoned_at IS NULL))
         );
         CREATE INDEX changed_path_segment_quarantine_allocations_state_idx
             ON changed_path_segment_quarantine_allocations(scope_id, epoch, segment_id, state);
         CREATE UNIQUE INDEX changed_path_segment_quarantine_allocations_active_idx
             ON changed_path_segment_quarantine_allocations(scope_id, epoch, segment_id)
             WHERE state IN ('allocating', 'allocated', 'bound');
         CREATE TABLE changed_path_segment_deletions (
             scope_id TEXT NOT NULL,
             epoch INTEGER NOT NULL CHECK (epoch >= 1),
             segment_id TEXT NOT NULL CHECK (length(segment_id) > 0),
             original_leaf TEXT NOT NULL CHECK (length(original_leaf) > 0),
             quarantine_leaf TEXT NOT NULL CHECK (length(quarantine_leaf) > 0),
             allocation_nonce TEXT NOT NULL UNIQUE CHECK (length(allocation_nonce) = 64),
             log_format_version INTEGER NOT NULL CHECK (log_format_version=1),
             provider_id TEXT NOT NULL CHECK (length(provider_id)>0),
             folded_end_offset INTEGER NOT NULL CHECK (folded_end_offset>=0),
             retirement_continuity_generation INTEGER NOT NULL CHECK (retirement_continuity_generation>=1),
             retirement_fence_nonce BLOB NOT NULL CHECK (length(retirement_fence_nonce)=32),
             scope_directory_device TEXT NOT NULL CHECK (length(scope_directory_device) > 0),
             scope_directory_inode TEXT NOT NULL CHECK (length(scope_directory_inode) > 0),
             quarantine_device TEXT NOT NULL CHECK (length(quarantine_device) > 0),
             quarantine_inode TEXT NOT NULL CHECK (length(quarantine_inode) > 0),
             segment_device TEXT NOT NULL CHECK (length(segment_device) > 0),
             segment_inode TEXT NOT NULL CHECK (length(segment_inode) > 0),
             file_length INTEGER NOT NULL CHECK (file_length >= 0),
             file_hash TEXT NOT NULL CHECK (length(file_hash) = 64),
             durable_end_offset INTEGER NOT NULL CHECK (durable_end_offset >= 0),
             durable_hash TEXT NOT NULL CHECK (length(durable_hash) = 64),
             max_observer_log_bytes INTEGER NOT NULL CHECK (max_observer_log_bytes > 0),
             max_segment_bytes INTEGER NOT NULL CHECK (max_segment_bytes > 0),
             max_unfolded_tail_records INTEGER NOT NULL CHECK (max_unfolded_tail_records > 0),
             owner_token TEXT NOT NULL CHECK (length(owner_token) = 64),
             first_sequence INTEGER NOT NULL CHECK (first_sequence >= 1),
             last_sequence INTEGER,
             previous_segment_id TEXT,
             previous_segment_hash TEXT NOT NULL CHECK (length(previous_segment_hash) = 64),
             source_state TEXT NOT NULL CHECK (source_state IN ('open', 'sealed')),
             state TEXT NOT NULL DEFAULT 'quiesced' CHECK (state = 'quiesced'),
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             completed_at INTEGER,
             PRIMARY KEY (scope_id, epoch, segment_id),
             UNIQUE (scope_id, epoch, original_leaf),
             UNIQUE (scope_id, epoch, quarantine_leaf),
             FOREIGN KEY (scope_id, epoch, segment_id)
                 REFERENCES changed_path_observer_segments(scope_id, epoch, segment_id)
                 ON UPDATE CASCADE ON DELETE CASCADE,
             FOREIGN KEY (allocation_nonce)
                 REFERENCES changed_path_segment_quarantine_allocations(attempt_nonce)
                 ON UPDATE CASCADE ON DELETE CASCADE,
             CHECK (quarantine_device=segment_device AND quarantine_inode=segment_inode),
             CHECK (completed_at IS NOT NULL)
         );
         CREATE INDEX changed_path_segment_deletions_state_idx
             ON changed_path_segment_deletions(scope_id, epoch, state);";

pub(super) fn create_changed_path_ledger_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(CHANGED_PATH_LEDGER_SCHEMA_V18)
        .map_err(Into::into)
}

pub(super) fn changed_path_ledger_schema_complete(conn: &Connection) -> Result<bool> {
    let foreign_keys = conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))?;
    if foreign_keys != 1
        || CHANGED_PATH_LEDGER_SCHEMA_VERSION != 1
        || CHANGED_PATH_OBSERVER_LOG_FORMAT_VERSION != 1
    {
        return Ok(false);
    }
    let user_version = conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    if user_version != TRAIL_SCHEMA_VERSION {
        return Ok(false);
    }
    let schema_meta_exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_meta')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !schema_meta_exists {
        return Ok(false);
    }
    let expected_version = TRAIL_SCHEMA_VERSION.to_string();
    let meta_version = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            params![SCHEMA_META_VERSION_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if meta_version.as_deref() != Some(expected_version.as_str()) {
        return Ok(false);
    }
    if !ledger_master_matches(conn)? {
        return Ok(false);
    }
    schema_structure_complete(conn)
}

fn ledger_master_matches(conn: &Connection) -> Result<bool> {
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(CHANGED_PATH_LEDGER_SCHEMA_V18)?;
    Ok(ledger_schema_objects(conn)? == ledger_schema_objects(&expected)?)
}

fn ledger_schema_objects(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut statement = conn.prepare(
        "SELECT type, name, COALESCE(sql, '') FROM sqlite_master
         WHERE name LIKE 'changed_path_%'
         ORDER BY type, name",
    )?;
    let objects = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                normalized_sql(&row.get::<_, String>(2)?),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(objects)
}

fn schema_structure_complete(conn: &Connection) -> Result<bool> {
    type Column = (&'static str, &'static str, bool, Option<&'static str>, i64);
    let tables: [(&str, &[Column]); 14] = [
        (
            "changed_path_scopes",
            &[
                ("scope_id", "TEXT", true, None, 1),
                ("schema_version", "INTEGER", true, Some("1"), 0),
                ("scope_kind", "TEXT", true, None, 0),
                ("owner_id", "TEXT", true, None, 0),
                ("scope_root", "TEXT", true, None, 0),
                ("scope_root_identity", "TEXT", true, None, 0),
                ("filesystem_identity", "TEXT", true, None, 0),
                ("filesystem_kind", "TEXT", true, None, 0),
                ("case_sensitive", "INTEGER", true, None, 0),
                ("ref_name", "TEXT", true, None, 0),
                ("ref_generation", "INTEGER", true, None, 0),
                ("change_id", "TEXT", true, None, 0),
                ("baseline_root_id", "TEXT", true, None, 0),
                ("policy_fingerprint", "TEXT", true, None, 0),
                ("policy_dependency_generation", "INTEGER", true, None, 0),
                ("trust_state", "TEXT", true, Some("'reconciling'"), 0),
                ("trust_reason", "TEXT", true, Some("'fresh_create'"), 0),
                ("continuity_generation", "INTEGER", true, Some("1"), 0),
                ("epoch", "INTEGER", true, Some("1"), 0),
                ("max_candidate_rows", "INTEGER", true, Some("250000"), 0),
                ("max_prefix_rows", "INTEGER", true, Some("16384"), 0),
                (
                    "max_observer_log_bytes",
                    "INTEGER",
                    true,
                    Some("268435456"),
                    0,
                ),
                ("max_segment_bytes", "INTEGER", true, Some("16777216"), 0),
                (
                    "max_unfolded_tail_records",
                    "INTEGER",
                    true,
                    Some("65536"),
                    0,
                ),
                ("provider_id", "TEXT", false, None, 0),
                ("provider_identity", "TEXT", false, None, 0),
                ("durable_cursor", "INTEGER", true, Some("0"), 0),
                ("linearizable_fence", "INTEGER", true, Some("0"), 0),
                ("rename_pairing", "INTEGER", true, Some("0"), 0),
                ("overflow_scope", "INTEGER", true, Some("0"), 0),
                ("filesystem_supported", "INTEGER", true, Some("0"), 0),
                ("clean_proof_allowed", "INTEGER", true, Some("0"), 0),
                ("power_loss_durability", "INTEGER", true, Some("0"), 0),
                ("provider_cursor", "BLOB", false, None, 0),
                ("provider_fence", "BLOB", false, None, 0),
                ("durable_offset", "INTEGER", true, Some("0"), 0),
                ("folded_offset", "INTEGER", true, Some("0"), 0),
                ("observer_owner_token", "TEXT", false, None, 0),
                ("observer_heartbeat_at", "INTEGER", false, None, 0),
                ("observer_error_state", "TEXT", false, None, 0),
                ("observer_error_at", "INTEGER", false, None, 0),
                ("retired_at", "INTEGER", false, None, 0),
                ("created_at", "INTEGER", true, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_entries",
            &[
                ("scope_id", "TEXT", true, None, 1),
                ("normalized_path", "TEXT", true, None, 2),
                ("event_flags", "INTEGER", true, None, 0),
                ("source_mask", "INTEGER", true, None, 0),
                ("first_sequence", "INTEGER", true, None, 0),
                ("last_sequence", "INTEGER", true, None, 0),
                ("provider_id", "TEXT", false, None, 0),
                ("provider_sequence", "INTEGER", false, None, 0),
                ("intent_id", "TEXT", false, None, 0),
                ("created_at", "INTEGER", true, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_prefixes",
            &[
                ("scope_id", "TEXT", true, None, 1),
                ("normalized_prefix", "TEXT", true, None, 2),
                ("completeness_reason", "TEXT", true, None, 0),
                ("event_flags", "INTEGER", true, None, 0),
                ("source_mask", "INTEGER", true, None, 0),
                ("first_sequence", "INTEGER", true, None, 0),
                ("last_sequence", "INTEGER", true, None, 0),
                ("provider_id", "TEXT", false, None, 0),
                ("provider_sequence", "INTEGER", false, None, 0),
                ("intent_id", "TEXT", false, None, 0),
                ("created_at", "INTEGER", true, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_policy_dependencies",
            &[
                ("scope_id", "TEXT", true, None, 1),
                ("dependency_identity", "TEXT", true, None, 2),
                ("dependency_kind", "TEXT", true, None, 3),
                ("content_identity", "BLOB", true, None, 0),
                ("metadata_identity", "BLOB", true, None, 0),
                ("observable", "INTEGER", true, None, 0),
                ("generation", "INTEGER", true, None, 0),
                ("last_source_sequence", "INTEGER", true, Some("0"), 0),
                ("created_at", "INTEGER", true, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_intents",
            &[
                ("intent_id", "TEXT", true, None, 1),
                ("schema_version", "INTEGER", true, Some("1"), 0),
                ("scope_id", "TEXT", true, None, 0),
                ("producer", "TEXT", true, None, 0),
                ("expected_scope_epoch", "INTEGER", true, None, 0),
                ("expected_ref_name", "TEXT", true, None, 0),
                ("expected_ref_generation", "INTEGER", true, None, 0),
                ("expected_change_id", "TEXT", true, None, 0),
                ("expected_root_id", "TEXT", true, None, 0),
                ("target_change_id", "TEXT", true, None, 0),
                ("target_root_id", "TEXT", true, None, 0),
                ("target_operation_id", "TEXT", false, None, 0),
                ("start_cursor", "BLOB", false, None, 0),
                ("lifecycle_state", "TEXT", true, Some("'prepared'"), 0),
                ("verified_cut", "BLOB", false, None, 0),
                ("failure_reason", "TEXT", false, None, 0),
                ("created_at", "INTEGER", true, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_intent_paths",
            &[
                ("intent_id", "TEXT", true, None, 1),
                ("normalized_path", "TEXT", true, None, 2),
                ("event_flags", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_intent_prefixes",
            &[
                ("intent_id", "TEXT", true, None, 1),
                ("normalized_prefix", "TEXT", true, None, 2),
                ("completeness_reason", "TEXT", true, None, 0),
                ("event_flags", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_reconciliations",
            &[
                ("attempt_id", "TEXT", true, None, 1),
                ("schema_version", "INTEGER", true, Some("1"), 0),
                ("scope_id", "TEXT", true, None, 0),
                ("expected_scope_epoch", "INTEGER", true, None, 0),
                ("expected_ref_name", "TEXT", true, None, 0),
                ("expected_ref_generation", "INTEGER", true, None, 0),
                ("expected_change_id", "TEXT", true, None, 0),
                ("expected_root_id", "TEXT", true, None, 0),
                ("filesystem_identity", "TEXT", true, None, 0),
                ("policy_fingerprint", "TEXT", true, None, 0),
                ("policy_dependency_generation", "INTEGER", true, None, 0),
                ("provider_id", "TEXT", false, None, 0),
                ("provider_identity", "TEXT", false, None, 0),
                ("start_cursor", "BLOB", false, None, 0),
                ("start_fence", "BLOB", false, None, 0),
                ("mode", "TEXT", true, None, 0),
                ("reason", "TEXT", true, None, 0),
                ("completeness_class", "TEXT", true, None, 0),
                ("staged_store_location", "TEXT", true, Some("'sqlite'"), 0),
                ("state", "TEXT", true, Some("'prepared'"), 0),
                ("created_at", "INTEGER", true, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_reconciliation_rows",
            &[
                ("attempt_id", "TEXT", true, None, 1),
                ("normalized_path", "TEXT", true, None, 2),
                ("row_kind", "TEXT", true, None, 0),
                ("file_kind", "TEXT", false, None, 0),
                ("content_hash", "TEXT", false, None, 0),
                ("executable", "INTEGER", false, None, 0),
                ("size_bytes", "INTEGER", false, None, 0),
                ("before_identity", "TEXT", false, None, 0),
                ("after_identity", "TEXT", false, None, 0),
                ("source_sequence", "INTEGER", false, None, 0),
                ("staged_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_reconciliation_guards",
            &[
                ("attempt_id", "TEXT", true, None, 1),
                ("relative_path", "BLOB", true, None, 2),
                ("directory_identity", "BLOB", true, None, 0),
                ("staged_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_observer_segments",
            &[
                ("scope_id", "TEXT", true, None, 1),
                ("epoch", "INTEGER", true, None, 2),
                ("segment_id", "TEXT", true, None, 3),
                ("log_format_version", "INTEGER", true, Some("1"), 0),
                ("owner_token", "TEXT", true, None, 0),
                ("provider_id", "TEXT", true, None, 0),
                ("first_sequence", "INTEGER", true, None, 0),
                ("last_sequence", "INTEGER", false, None, 0),
                ("durable_end_offset", "INTEGER", true, Some("0"), 0),
                ("folded_end_offset", "INTEGER", true, Some("0"), 0),
                ("previous_segment_id", "TEXT", false, None, 0),
                ("previous_segment_hash", "TEXT", false, None, 0),
                ("segment_hash", "TEXT", false, None, 0),
                ("segment_path", "TEXT", true, None, 0),
                ("state", "TEXT", true, Some("'open'"), 0),
                ("retirement_source_state", "TEXT", false, None, 0),
                ("retirement_file_length", "INTEGER", false, None, 0),
                ("retirement_file_hash", "TEXT", false, None, 0),
                ("retirement_durable_hash", "TEXT", false, None, 0),
                ("retirement_source_device", "TEXT", false, None, 0),
                ("retirement_source_inode", "TEXT", false, None, 0),
                ("created_at", "INTEGER", true, None, 0),
                ("sealed_at", "INTEGER", false, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_observer_owners",
            &[
                ("scope_id", "TEXT", true, None, 1),
                ("epoch", "INTEGER", true, None, 0),
                ("owner_token", "TEXT", true, None, 0),
                ("provider_id", "TEXT", true, None, 0),
                ("provider_identity", "TEXT", true, None, 0),
                ("lease_state", "TEXT", true, Some("'active'"), 0),
                ("fence_nonce", "BLOB", false, None, 0),
                ("acquired_at", "INTEGER", true, None, 0),
                ("heartbeat_at", "INTEGER", true, None, 0),
                ("expires_at", "INTEGER", true, None, 0),
                ("error_state", "TEXT", false, None, 0),
                ("error_at", "INTEGER", false, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
            ],
        ),
        (
            "changed_path_segment_quarantine_allocations",
            &[
                ("attempt_nonce", "TEXT", false, None, 1),
                ("scope_id", "TEXT", true, None, 0),
                ("epoch", "INTEGER", true, None, 0),
                ("segment_id", "TEXT", true, None, 0),
                ("quarantine_leaf", "TEXT", true, None, 0),
                ("scope_directory_device", "TEXT", true, None, 0),
                ("scope_directory_inode", "TEXT", true, None, 0),
                ("identity_policy", "TEXT", true, None, 0),
                ("source_segment_device", "TEXT", true, None, 0),
                ("source_segment_inode", "TEXT", true, None, 0),
                ("quarantine_device", "TEXT", false, None, 0),
                ("quarantine_inode", "TEXT", false, None, 0),
                ("observed_conflict_device", "TEXT", false, None, 0),
                ("observed_conflict_inode", "TEXT", false, None, 0),
                ("retained_reason", "TEXT", false, None, 0),
                ("state", "TEXT", true, Some("'allocating'"), 0),
                ("created_at", "INTEGER", true, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
                ("allocated_at", "INTEGER", false, None, 0),
                ("bound_at", "INTEGER", false, None, 0),
                ("abandoned_at", "INTEGER", false, None, 0),
            ],
        ),
        (
            "changed_path_segment_deletions",
            &[
                ("scope_id", "TEXT", true, None, 1),
                ("epoch", "INTEGER", true, None, 2),
                ("segment_id", "TEXT", true, None, 3),
                ("original_leaf", "TEXT", true, None, 0),
                ("quarantine_leaf", "TEXT", true, None, 0),
                ("allocation_nonce", "TEXT", true, None, 0),
                ("log_format_version", "INTEGER", true, None, 0),
                ("provider_id", "TEXT", true, None, 0),
                ("folded_end_offset", "INTEGER", true, None, 0),
                ("retirement_continuity_generation", "INTEGER", true, None, 0),
                ("retirement_fence_nonce", "BLOB", true, None, 0),
                ("scope_directory_device", "TEXT", true, None, 0),
                ("scope_directory_inode", "TEXT", true, None, 0),
                ("quarantine_device", "TEXT", true, None, 0),
                ("quarantine_inode", "TEXT", true, None, 0),
                ("segment_device", "TEXT", true, None, 0),
                ("segment_inode", "TEXT", true, None, 0),
                ("file_length", "INTEGER", true, None, 0),
                ("file_hash", "TEXT", true, None, 0),
                ("durable_end_offset", "INTEGER", true, None, 0),
                ("durable_hash", "TEXT", true, None, 0),
                ("max_observer_log_bytes", "INTEGER", true, None, 0),
                ("max_segment_bytes", "INTEGER", true, None, 0),
                ("max_unfolded_tail_records", "INTEGER", true, None, 0),
                ("owner_token", "TEXT", true, None, 0),
                ("first_sequence", "INTEGER", true, None, 0),
                ("last_sequence", "INTEGER", false, None, 0),
                ("previous_segment_id", "TEXT", false, None, 0),
                ("previous_segment_hash", "TEXT", true, None, 0),
                ("source_state", "TEXT", true, None, 0),
                ("state", "TEXT", true, Some("'quiesced'"), 0),
                ("created_at", "INTEGER", true, None, 0),
                ("updated_at", "INTEGER", true, None, 0),
                ("completed_at", "INTEGER", false, None, 0),
            ],
        ),
    ];
    for (table, expected) in tables {
        if !table_columns_match(conn, table, expected)? {
            return Ok(false);
        }
    }

    let table_fragments: [(&str, &[&str]); 14] = [
        (
            "changed_path_scopes",
            &[
                "CHECK (length(scope_id) > 0)",
                "CHECK (schema_version = 1)",
                "CHECK (scope_kind IN ('workspace', 'materialized_lane', 'workspace_view'))",
                "CHECK (length(owner_id) > 0)",
                "CHECK (case_sensitive IN (0, 1))",
                "CHECK (ref_generation >= 0)",
                "CHECK (policy_dependency_generation >= 0)",
                "CHECK (trust_state IN ('trusted', 'reconciling', 'overflow', 'untrusted_gap', 'stale_baseline', 'corrupt'))",
                "CHECK (continuity_generation >= 1)",
                "CHECK (epoch >= 1)",
                "CHECK(max_candidate_rows>0)",
                "CHECK(max_prefix_rows>0)",
                "CHECK(max_observer_log_bytes>0)",
                "CHECK(max_segment_bytes>0 AND max_segment_bytes<=max_observer_log_bytes)",
                "CHECK(max_unfolded_tail_records>0)",
                "CHECK (durable_cursor IN (0, 1))",
                "CHECK (linearizable_fence IN (0, 1))",
                "CHECK (rename_pairing IN (0, 1))",
                "CHECK (overflow_scope IN (0, 1))",
                "CHECK (filesystem_supported IN (0, 1))",
                "CHECK (clean_proof_allowed IN (0, 1))",
                "CHECK (power_loss_durability IN (0, 1))",
                "CHECK (durable_offset >= 0)",
                "CHECK (folded_offset >= 0)",
                "UNIQUE (scope_kind, owner_id)",
                "CHECK (folded_offset <= durable_offset)",
                "CHECK ((observer_error_state IS NULL AND observer_error_at IS NULL) OR (observer_error_state IS NOT NULL AND observer_error_at IS NOT NULL))",
            ],
        ),
        (
            "changed_path_entries",
            &[
                "CHECK (length(normalized_path) > 0)",
                "CHECK (event_flags >= 0)",
                "CHECK (source_mask >= 0)",
                "CHECK (first_sequence >= 0)",
                "CHECK (provider_sequence IS NULL OR provider_sequence >= 0)",
                "CHECK (last_sequence >= first_sequence)",
            ],
        ),
        (
            "changed_path_prefixes",
            &[
                "CHECK (length(normalized_prefix) > 0)",
                "CHECK (event_flags >= 0)",
                "CHECK (source_mask >= 0)",
                "CHECK (first_sequence >= 0)",
                "CHECK (provider_sequence IS NULL OR provider_sequence >= 0)",
                "CHECK (last_sequence >= first_sequence)",
            ],
        ),
        (
            "changed_path_policy_dependencies",
            &[
                "dependency_identity TEXT COLLATE BINARY NOT NULL",
                "CHECK(dependency_kind IN ('builtin','trail_config','ignore','trailignore','gitignore','git_info_exclude','git_excludes_file','git_config','normalization','mode','case_policy'))",
                "CHECK(typeof(content_identity)='blob' AND length(content_identity)=32)",
                "CHECK(observable IN (0,1))",
                "PRIMARY KEY(scope_id,dependency_identity,dependency_kind)",
                "WITHOUT ROWID",
            ],
        ),
        (
            "changed_path_intents",
            &[
                "CHECK (length(intent_id) > 0)",
                "CHECK (schema_version = 1)",
                "CHECK (expected_scope_epoch >= 1)",
                "CHECK (expected_ref_generation >= 0)",
                "CHECK (lifecycle_state IN ('prepared', 'filesystem_applied', 'published', 'acknowledged', 'aborted'))",
            ],
        ),
        (
            "changed_path_intent_paths",
            &[
                "CHECK (length(normalized_path) > 0)",
                "CHECK (event_flags >= 0)",
            ],
        ),
        (
            "changed_path_intent_prefixes",
            &[
                "CHECK (length(normalized_prefix) > 0)",
                "CHECK (event_flags >= 0)",
            ],
        ),
        (
            "changed_path_reconciliations",
            &[
                "CHECK (length(attempt_id) > 0)",
                "CHECK (schema_version = 1)",
                "CHECK (expected_scope_epoch >= 1)",
                "CHECK (expected_ref_generation >= 0)",
                "CHECK (policy_dependency_generation >= 0)",
                "CHECK (mode IN ('full', 'prefix'))",
                "CHECK (completeness_class IN ('complete', 'provider_complete_prefix', 'point_in_time_untrusted'))",
                "CHECK (state IN ('prepared', 'staging', 'ready', 'published', 'abandoned', 'failed'))",
            ],
        ),
        (
            "changed_path_reconciliation_rows",
            &[
                "CHECK (length(normalized_path) > 0)",
                "CHECK (row_kind IN ('entry', 'deletion'))",
                "CHECK (executable IS NULL OR executable IN (0, 1))",
                "CHECK (size_bytes IS NULL OR size_bytes >= 0)",
                "CHECK (source_sequence IS NULL OR source_sequence >= 0)",
            ],
        ),
        (
            "changed_path_reconciliation_guards",
            &[
                "CHECK (length(relative_path) > 0)",
                "CHECK (length(directory_identity) > 0)",
                "WITHOUT ROWID",
            ],
        ),
        (
            "changed_path_observer_segments",
            &[
                "CHECK (length(segment_id) > 0)",
                "CHECK (epoch >= 1)",
                "CHECK (log_format_version = 1)",
                "CHECK (first_sequence >= 1)",
                "CHECK (last_sequence IS NULL OR last_sequence >= first_sequence)",
                "CHECK (durable_end_offset >= 0)",
                "CHECK (folded_end_offset >= 0)",
                "CHECK (state IN ('open', 'sealed', 'retiring', 'retired', 'corrupt'))",
                "UNIQUE (scope_id, epoch, first_sequence)",
                "CHECK (folded_end_offset <= durable_end_offset)",
                "CHECK ((retirement_source_device IS NULL)=(retirement_source_inode IS NULL))",
            ],
        ),
        (
            "changed_path_observer_owners",
            &[
                "CHECK (length(owner_token) > 0)",
                "CHECK (epoch >= 1)",
                "CHECK (lease_state IN ('active', 'revoked', 'expired', 'error'))",
                "CHECK (acquired_at <= heartbeat_at AND heartbeat_at <= expires_at)",
                "CHECK ((error_state IS NULL AND error_at IS NULL) OR (error_state IS NOT NULL AND error_at IS NOT NULL))",
                "owner_token TEXT NOT NULL UNIQUE",
            ],
        ),
        (
            "changed_path_segment_quarantine_allocations",
            &[
                "CHECK (length(attempt_nonce) = 64)",
                "CHECK (epoch >= 1)",
                "CHECK (identity_policy = 'direct_noreplace_same_directory_v1')",
                "CHECK (state IN ('allocating', 'allocated', 'bound', 'abandoned'))",
                "CHECK (quarantine_device IS NULL OR (quarantine_device=source_segment_device AND quarantine_inode=source_segment_inode))",
            ],
        ),
        (
            "changed_path_segment_deletions",
            &[
                "CHECK (epoch >= 1)",
                "CHECK (length(allocation_nonce) = 64)",
                "CHECK (log_format_version=1)",
                "CHECK (folded_end_offset>=0)",
                "CHECK (retirement_continuity_generation>=1)",
                "CHECK (length(retirement_fence_nonce)=32)",
                "CHECK (state = 'quiesced')",
                "CHECK (quarantine_device=segment_device AND quarantine_inode=segment_inode)",
                "UNIQUE (scope_id, epoch, original_leaf)",
                "UNIQUE (scope_id, epoch, quarantine_leaf)",
            ],
        ),
    ];
    for (table, fragments) in table_fragments {
        if !table_sql_contains(conn, table, fragments)? {
            return Ok(false);
        }
    }

    let primary_keys: [(&str, &[&str]); 14] = [
        ("changed_path_scopes", &["scope_id"]),
        ("changed_path_entries", &["scope_id", "normalized_path"]),
        ("changed_path_prefixes", &["scope_id", "normalized_prefix"]),
        (
            "changed_path_policy_dependencies",
            &["scope_id", "dependency_identity", "dependency_kind"],
        ),
        ("changed_path_intents", &["intent_id"]),
        (
            "changed_path_intent_paths",
            &["intent_id", "normalized_path"],
        ),
        (
            "changed_path_intent_prefixes",
            &["intent_id", "normalized_prefix"],
        ),
        ("changed_path_reconciliations", &["attempt_id"]),
        (
            "changed_path_reconciliation_rows",
            &["attempt_id", "normalized_path"],
        ),
        (
            "changed_path_reconciliation_guards",
            &["attempt_id", "relative_path"],
        ),
        (
            "changed_path_observer_segments",
            &["scope_id", "epoch", "segment_id"],
        ),
        ("changed_path_observer_owners", &["scope_id"]),
        (
            "changed_path_segment_quarantine_allocations",
            &["attempt_nonce"],
        ),
        (
            "changed_path_segment_deletions",
            &["scope_id", "epoch", "segment_id"],
        ),
    ];
    for (table, columns) in primary_keys {
        if !origin_index_matches(conn, table, "pk", columns)? {
            return Ok(false);
        }
    }
    for (table, columns) in [
        (
            "changed_path_scopes",
            &["scope_kind", "owner_id"].as_slice(),
        ),
        (
            "changed_path_observer_segments",
            &["scope_id", "epoch", "first_sequence"].as_slice(),
        ),
        ("changed_path_observer_owners", &["owner_token"].as_slice()),
    ] {
        if !origin_index_matches(conn, table, "u", columns)? {
            return Ok(false);
        }
    }
    if !origin_indexes_match(
        conn,
        "changed_path_segment_quarantine_allocations",
        "u",
        &[&["quarantine_leaf"]],
    )? || !origin_indexes_match(
        conn,
        "changed_path_segment_deletions",
        "u",
        &[
            &["allocation_nonce"],
            &["scope_id", "epoch", "original_leaf"],
            &["scope_id", "epoch", "quarantine_leaf"],
        ],
    )? {
        return Ok(false);
    }
    for table in [
        "changed_path_entries",
        "changed_path_prefixes",
        "changed_path_intents",
        "changed_path_intent_paths",
        "changed_path_intent_prefixes",
        "changed_path_reconciliations",
        "changed_path_reconciliation_rows",
        "changed_path_reconciliation_guards",
    ] {
        if origin_index_count(conn, table, "u")? != 0 {
            return Ok(false);
        }
    }

    let named_indexes: [(&str, &str, &[&str]); 15] = [
        (
            "changed_path_intents",
            "changed_path_intents_scope_state_idx",
            &["scope_id", "lifecycle_state", "updated_at"],
        ),
        (
            "changed_path_intents",
            "changed_path_intents_gc_idx",
            &["lifecycle_state", "target_root_id", "target_operation_id"],
        ),
        (
            "changed_path_entries",
            "changed_path_entries_sequence_idx",
            &["scope_id", "last_sequence"],
        ),
        (
            "changed_path_entries",
            "changed_path_entries_provider_idx",
            &["scope_id", "provider_id", "provider_sequence"],
        ),
        (
            "changed_path_entries",
            "changed_path_entries_intent_idx",
            &["intent_id"],
        ),
        (
            "changed_path_prefixes",
            "changed_path_prefixes_sequence_idx",
            &["scope_id", "last_sequence"],
        ),
        (
            "changed_path_prefixes",
            "changed_path_prefixes_provider_idx",
            &["scope_id", "provider_id", "provider_sequence"],
        ),
        (
            "changed_path_prefixes",
            "changed_path_prefixes_intent_idx",
            &["intent_id"],
        ),
        (
            "changed_path_policy_dependencies",
            "changed_path_policy_dependencies_generation_idx",
            &["scope_id", "generation", "last_source_sequence"],
        ),
        (
            "changed_path_reconciliations",
            "changed_path_reconciliations_scope_state_idx",
            &["scope_id", "state", "updated_at"],
        ),
        (
            "changed_path_reconciliation_rows",
            "changed_path_reconciliation_rows_kind_idx",
            &["attempt_id", "row_kind", "normalized_path"],
        ),
        (
            "changed_path_observer_owners",
            "changed_path_observer_owners_state_idx",
            &["lease_state", "expires_at"],
        ),
        (
            "changed_path_observer_segments",
            "changed_path_observer_segments_state_idx",
            &["scope_id", "epoch", "state", "last_sequence"],
        ),
        (
            "changed_path_segment_quarantine_allocations",
            "changed_path_segment_quarantine_allocations_state_idx",
            &["scope_id", "epoch", "segment_id", "state"],
        ),
        (
            "changed_path_segment_deletions",
            "changed_path_segment_deletions_state_idx",
            &["scope_id", "epoch", "state"],
        ),
    ];
    for (table, index, columns) in named_indexes {
        if !named_index_matches(conn, table, index, columns)? {
            return Ok(false);
        }
    }
    if !named_unique_partial_index_matches(
        conn,
        "changed_path_segment_quarantine_allocations",
        "changed_path_segment_quarantine_allocations_active_idx",
        &["scope_id", "epoch", "segment_id"],
    )? {
        return Ok(false);
    }

    let expected_foreign_keys: [(&str, &[(&str, &str, &str, &str, &str)]); 14] = [
        ("changed_path_scopes", &[]),
        (
            "changed_path_entries",
            &[
                (
                    "scope_id",
                    "changed_path_scopes",
                    "scope_id",
                    "CASCADE",
                    "CASCADE",
                ),
                (
                    "intent_id",
                    "changed_path_intents",
                    "intent_id",
                    "CASCADE",
                    "SET NULL",
                ),
            ],
        ),
        (
            "changed_path_prefixes",
            &[
                (
                    "scope_id",
                    "changed_path_scopes",
                    "scope_id",
                    "CASCADE",
                    "CASCADE",
                ),
                (
                    "intent_id",
                    "changed_path_intents",
                    "intent_id",
                    "CASCADE",
                    "SET NULL",
                ),
            ],
        ),
        (
            "changed_path_policy_dependencies",
            &[(
                "scope_id",
                "changed_path_scopes",
                "scope_id",
                "CASCADE",
                "CASCADE",
            )],
        ),
        (
            "changed_path_intents",
            &[(
                "scope_id",
                "changed_path_scopes",
                "scope_id",
                "CASCADE",
                "CASCADE",
            )],
        ),
        (
            "changed_path_intent_paths",
            &[(
                "intent_id",
                "changed_path_intents",
                "intent_id",
                "CASCADE",
                "CASCADE",
            )],
        ),
        (
            "changed_path_intent_prefixes",
            &[(
                "intent_id",
                "changed_path_intents",
                "intent_id",
                "CASCADE",
                "CASCADE",
            )],
        ),
        (
            "changed_path_reconciliations",
            &[(
                "scope_id",
                "changed_path_scopes",
                "scope_id",
                "CASCADE",
                "CASCADE",
            )],
        ),
        (
            "changed_path_reconciliation_rows",
            &[(
                "attempt_id",
                "changed_path_reconciliations",
                "attempt_id",
                "CASCADE",
                "CASCADE",
            )],
        ),
        (
            "changed_path_reconciliation_guards",
            &[(
                "attempt_id",
                "changed_path_reconciliations",
                "attempt_id",
                "CASCADE",
                "CASCADE",
            )],
        ),
        (
            "changed_path_observer_segments",
            &[(
                "scope_id",
                "changed_path_scopes",
                "scope_id",
                "CASCADE",
                "CASCADE",
            )],
        ),
        (
            "changed_path_observer_owners",
            &[(
                "scope_id",
                "changed_path_scopes",
                "scope_id",
                "CASCADE",
                "CASCADE",
            )],
        ),
        (
            "changed_path_segment_quarantine_allocations",
            &[
                (
                    "scope_id",
                    "changed_path_observer_segments",
                    "scope_id",
                    "CASCADE",
                    "CASCADE",
                ),
                (
                    "epoch",
                    "changed_path_observer_segments",
                    "epoch",
                    "CASCADE",
                    "CASCADE",
                ),
                (
                    "segment_id",
                    "changed_path_observer_segments",
                    "segment_id",
                    "CASCADE",
                    "CASCADE",
                ),
            ],
        ),
        (
            "changed_path_segment_deletions",
            &[
                (
                    "scope_id",
                    "changed_path_observer_segments",
                    "scope_id",
                    "CASCADE",
                    "CASCADE",
                ),
                (
                    "epoch",
                    "changed_path_observer_segments",
                    "epoch",
                    "CASCADE",
                    "CASCADE",
                ),
                (
                    "segment_id",
                    "changed_path_observer_segments",
                    "segment_id",
                    "CASCADE",
                    "CASCADE",
                ),
                (
                    "allocation_nonce",
                    "changed_path_segment_quarantine_allocations",
                    "attempt_nonce",
                    "CASCADE",
                    "CASCADE",
                ),
            ],
        ),
    ];
    for (table, expected) in expected_foreign_keys {
        if !foreign_keys_match(conn, table, expected)? {
            return Ok(false);
        }
    }

    for (table, _) in tables {
        let mut statement = conn.prepare(&format!("PRAGMA foreign_key_check({table})"))?;
        let mut rows = statement.query([])?;
        if rows.next()?.is_some() {
            return Ok(false);
        }
    }
    let invalid_retirement_graph: bool = conn.query_row(
        "WITH invalid AS (
             SELECT allocation.attempt_nonce
             FROM changed_path_segment_quarantine_allocations allocation
             LEFT JOIN changed_path_segment_deletions deletion
               ON deletion.allocation_nonce=allocation.attempt_nonce
             GROUP BY allocation.attempt_nonce
             HAVING COUNT(deletion.allocation_nonce)<>CASE WHEN allocation.state='bound' THEN 1 ELSE 0 END
                OR (allocation.state='bound' AND (
                    MIN(deletion.state)<>'quiesced' OR MIN(deletion.completed_at) IS NULL
                    OR MIN(deletion.scope_id)<>allocation.scope_id
                    OR MIN(deletion.epoch)<>allocation.epoch
                    OR MIN(deletion.segment_id)<>allocation.segment_id
                    OR MIN(deletion.quarantine_leaf)<>allocation.quarantine_leaf
                    OR MIN(deletion.scope_directory_device)<>allocation.scope_directory_device
                    OR MIN(deletion.scope_directory_inode)<>allocation.scope_directory_inode
                    OR MIN(deletion.segment_device)<>allocation.source_segment_device
                    OR MIN(deletion.segment_inode)<>allocation.source_segment_inode
                    OR MIN(deletion.quarantine_device)<>allocation.quarantine_device
                    OR MIN(deletion.quarantine_inode)<>allocation.quarantine_inode))
             UNION ALL
             SELECT deletion.allocation_nonce
             FROM changed_path_segment_deletions deletion
             LEFT JOIN changed_path_segment_quarantine_allocations allocation
               ON allocation.attempt_nonce=deletion.allocation_nonce
             WHERE allocation.attempt_nonce IS NULL OR allocation.state<>'bound'
                OR deletion.state<>'quiesced' OR deletion.completed_at IS NULL
             UNION ALL
             SELECT segment_id
             FROM changed_path_segment_quarantine_allocations
             WHERE state IN ('allocating','allocated','bound')
             GROUP BY scope_id,epoch,segment_id HAVING COUNT(*)<>1
             UNION ALL
             SELECT attempt_nonce
             FROM changed_path_segment_quarantine_allocations
             WHERE quarantine_device IS NOT NULL
               AND (quarantine_device<>source_segment_device
                    OR quarantine_inode<>source_segment_inode)
             UNION ALL
             SELECT segment_id
             FROM changed_path_segment_deletions
             WHERE quarantine_device<>segment_device OR quarantine_inode<>segment_inode
             UNION ALL
             SELECT scope.scope_id
             FROM changed_path_scopes scope
             WHERE scope.retired_at IS NULL
               AND scope.trust_reason<>'scope_retiring'
               AND (EXISTS(
                       SELECT 1 FROM changed_path_segment_quarantine_allocations allocation
                       WHERE allocation.scope_id=scope.scope_id)
                    OR EXISTS(
                       SELECT 1 FROM changed_path_segment_deletions deletion
                       WHERE deletion.scope_id=scope.scope_id)
                    OR EXISTS(
                       SELECT 1 FROM changed_path_observer_segments segment
                       WHERE segment.scope_id=scope.scope_id
                         AND segment.state IN ('retiring','retired')))
             UNION ALL
             SELECT scope.scope_id
             FROM changed_path_scopes scope
             WHERE scope.retired_at IS NOT NULL
               AND (scope.trust_state<>'untrusted_gap' OR scope.trust_reason<>'scope_retired'
                    OR EXISTS(
                       SELECT 1 FROM changed_path_observer_segments segment
                       WHERE segment.scope_id=scope.scope_id
                         AND (segment.epoch<>scope.epoch OR segment.state<>'retired'))
                    OR EXISTS(
                       SELECT 1 FROM changed_path_observer_segments segment
                       WHERE segment.scope_id=scope.scope_id
                         AND ((SELECT COUNT(*)
                               FROM changed_path_segment_quarantine_allocations allocation
                               WHERE allocation.scope_id=segment.scope_id
                                 AND allocation.epoch=segment.epoch
                                 AND allocation.segment_id=segment.segment_id
                                 AND allocation.state='bound')<>1
                              OR (SELECT COUNT(*)
                                  FROM changed_path_segment_deletions deletion
                                  WHERE deletion.scope_id=segment.scope_id
                                    AND deletion.epoch=segment.epoch
                                    AND deletion.segment_id=segment.segment_id)<>1))
                    OR (EXISTS(SELECT 1 FROM changed_path_observer_segments segment
                               WHERE segment.scope_id=scope.scope_id)
                        AND NOT EXISTS(SELECT 1 FROM changed_path_observer_owners owner
                                       WHERE owner.scope_id=scope.scope_id
                                         AND owner.epoch=scope.epoch
                                         AND owner.lease_state='revoked'
                                         AND length(owner.fence_nonce)=32)))
             UNION ALL
             SELECT scope.scope_id
             FROM changed_path_scopes scope
             WHERE scope.retired_at IS NULL AND scope.trust_reason='scope_retiring'
               AND (scope.trust_state<>'untrusted_gap'
                    OR EXISTS(
                       SELECT 1 FROM changed_path_observer_segments segment
                       WHERE segment.scope_id=scope.scope_id
                         AND (segment.epoch<>scope.epoch OR segment.state<>'retiring'
                              OR segment.retirement_source_state IS NULL))
                    OR EXISTS(
                       SELECT 1 FROM changed_path_observer_owners owner
                       WHERE owner.scope_id=scope.scope_id
                         AND (owner.epoch<>scope.epoch OR owner.lease_state<>'revoked'
                              OR length(owner.fence_nonce)<>32))
                    OR (EXISTS(SELECT 1 FROM changed_path_observer_segments segment
                               WHERE segment.scope_id=scope.scope_id)
                        AND NOT EXISTS(SELECT 1 FROM changed_path_observer_owners owner
                                       WHERE owner.scope_id=scope.scope_id
                                         AND owner.epoch=scope.epoch
                                         AND owner.lease_state='revoked'
                                         AND length(owner.fence_nonce)=32))
                    OR EXISTS(
                       SELECT 1 FROM changed_path_segment_deletions deletion
                       WHERE deletion.scope_id=scope.scope_id))
             UNION ALL
             SELECT deletion.segment_id
             FROM changed_path_segment_deletions deletion
             JOIN changed_path_observer_segments segment
               ON segment.scope_id=deletion.scope_id AND segment.epoch=deletion.epoch
              AND segment.segment_id=deletion.segment_id
             JOIN changed_path_segment_quarantine_allocations allocation
               ON allocation.attempt_nonce=deletion.allocation_nonce
             JOIN changed_path_scopes scope ON scope.scope_id=segment.scope_id
             JOIN changed_path_observer_owners owner ON owner.scope_id=segment.scope_id
             WHERE scope.retired_at IS NULL OR scope.trust_reason<>'scope_retired'
                OR segment.state<>'retired' OR allocation.state<>'bound'
                OR owner.epoch<>segment.epoch OR owner.lease_state<>'revoked'
                OR deletion.retirement_continuity_generation<>scope.continuity_generation
                OR deletion.retirement_fence_nonce<>owner.fence_nonce
                OR deletion.original_leaf<>segment.segment_path
                OR deletion.log_format_version<>segment.log_format_version
                OR deletion.owner_token<>segment.owner_token
                OR deletion.owner_token<>owner.owner_token
                OR deletion.provider_id<>segment.provider_id
                OR deletion.provider_id<>owner.provider_id
                OR deletion.provider_id IS NOT scope.provider_id
                OR deletion.first_sequence<>segment.first_sequence
                OR deletion.last_sequence IS NOT segment.last_sequence
                OR deletion.durable_end_offset<>segment.durable_end_offset
                OR deletion.folded_end_offset<>segment.folded_end_offset
                OR deletion.previous_segment_id IS NOT segment.previous_segment_id
                OR deletion.previous_segment_hash<>
                   COALESCE(segment.previous_segment_hash,
                     '0000000000000000000000000000000000000000000000000000000000000000')
                OR (segment.segment_hash IS NOT NULL
                    AND deletion.file_hash<>segment.segment_hash)
                OR deletion.source_state<>segment.retirement_source_state
                OR deletion.file_length<>segment.retirement_file_length
                OR deletion.file_hash<>segment.retirement_file_hash
                OR deletion.durable_hash<>segment.retirement_durable_hash
                OR deletion.segment_device<>segment.retirement_source_device
                OR deletion.segment_inode<>segment.retirement_source_inode
                OR allocation.source_segment_device<>segment.retirement_source_device
                OR allocation.source_segment_inode<>segment.retirement_source_inode
         ) SELECT EXISTS(SELECT 1 FROM invalid)",
        [],
        |row| row.get(0),
    )?;
    if invalid_retirement_graph {
        return Ok(false);
    }
    policy_dependencies_canonical(conn)
}

fn policy_dependencies_canonical(conn: &Connection) -> Result<bool> {
    let invalid_row: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM changed_path_policy_dependencies dependency
             JOIN changed_path_scopes scope ON scope.scope_id=dependency.scope_id
             WHERE typeof(dependency.content_identity)<>'blob'
                OR length(dependency.content_identity)<>32
                OR dependency.generation<>scope.policy_dependency_generation
         )",
        [],
        |row| row.get(0),
    )?;
    if invalid_row {
        return Ok(false);
    }

    let mut statement = conn.prepare(
        "SELECT dependency_identity,dependency_kind
         FROM changed_path_policy_dependencies
         ORDER BY scope_id,dependency_kind COLLATE BINARY,dependency_identity COLLATE BINARY",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let identity = row.get::<_, String>(0)?;
        let kind = row.get::<_, String>(1)?;
        if !policy_dependency_identity_is_canonical(&identity, &kind) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn policy_dependency_identity_is_canonical(identity: &str, kind: &str) -> bool {
    if let Some(encoded_path) = identity.strip_prefix("path:") {
        if !matches!(
            kind,
            "trail_config"
                | "ignore"
                | "trailignore"
                | "gitignore"
                | "git_info_exclude"
                | "git_excludes_file"
                | "git_config"
        ) || encoded_path.is_empty()
            || encoded_path.len() % 2 != 0
            || !encoded_path
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return false;
        }
        let Ok(path_bytes) = hex::decode(encoded_path) else {
            return false;
        };
        let path = policy_path_from_bytes(path_bytes);
        return path.is_absolute()
            && lexical_normalize_policy_path(&path) == path
            && policy_path_identity(&path) == identity;
    }

    match kind {
        "builtin" => identity == "builtin:recording-policy",
        "trail_config" => identity == "trail-config:recording",
        "normalization" => identity == "normalization:path",
        "mode" => identity == "mode:filesystem-entry",
        "case_policy" => identity == "case-policy:scope",
        "git_config" => identity
            .strip_prefix("git-env:")
            .is_some_and(canonical_nonempty_hex),
        "ignore" | "trailignore" | "gitignore" | "git_info_exclude" | "git_excludes_file" => false,
        _ => false,
    }
}

fn canonical_nonempty_hex(value: &str) -> bool {
    !value.is_empty()
        && value.len() % 2 == 0
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(unix)]
fn policy_path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn policy_path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(unix)]
fn policy_path_identity(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    format!("path:{}", hex::encode(path.as_os_str().as_bytes()))
}

#[cfg(not(unix))]
fn policy_path_identity(path: &Path) -> String {
    format!("path:{}", hex::encode(path.to_string_lossy().as_bytes()))
}

fn lexical_normalize_policy_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn table_columns_match(
    conn: &Connection,
    table: &str,
    expected: &[(&str, &str, bool, Option<&str>, i64)],
) -> Result<bool> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let actual = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(actual.len() == expected.len()
        && actual.iter().zip(expected).all(
            |((name, ty, not_null, default_value, pk), expected)| {
                name == expected.0
                    && ty == expected.1
                    && not_null == &expected.2
                    && default_value.as_deref() == expected.3
                    && pk == &expected.4
            },
        ))
}

fn normalized_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn table_sql_contains(conn: &Connection, table: &str, fragments: &[&str]) -> Result<bool> {
    let sql = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(sql) = sql else {
        return Ok(false);
    };
    let sql = normalized_sql(&sql);
    Ok(fragments
        .iter()
        .all(|fragment| sql.contains(&normalized_sql(fragment))))
}

fn index_key_columns(conn: &Connection, index: &str) -> Result<Option<Vec<(String, String)>>> {
    let mut statement =
        conn.prepare("SELECT name, coll FROM pragma_index_xinfo(?1) WHERE key = 1 ORDER BY seqno")?;
    let columns = statement
        .query_map(params![index], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Error::from)?;
    let columns = columns
        .into_iter()
        .map(|(name, collation)| Some((name?, collation?)))
        .collect::<Option<Vec<_>>>();
    Ok(columns)
}

fn origin_index_matches(
    conn: &Connection,
    table: &str,
    origin: &str,
    expected_columns: &[&str],
) -> Result<bool> {
    let mut statement = conn.prepare(
        "SELECT name FROM pragma_index_list(?1)
         WHERE origin = ?2 AND [unique] = 1 AND partial = 0",
    )?;
    let names = statement
        .query_map(params![table, origin], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if names.len() != 1 {
        return Ok(false);
    }
    for name in names {
        let Some(columns) = index_key_columns(conn, &name)? else {
            return Ok(false);
        };
        if columns.len() == expected_columns.len()
            && columns
                .iter()
                .zip(expected_columns)
                .all(|((actual, collation), expected)| actual == expected && collation == "BINARY")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn origin_index_count(conn: &Connection, table: &str, origin: &str) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM pragma_index_list(?1) WHERE origin = ?2",
        params![table, origin],
        |row| row.get::<_, i64>(0),
    )
    .map_err(Error::from)
}

fn origin_indexes_match(
    conn: &Connection,
    table: &str,
    origin: &str,
    expected: &[&[&str]],
) -> Result<bool> {
    let mut statement = conn.prepare(
        "SELECT name FROM pragma_index_list(?1)
         WHERE origin=?2 AND [unique]=1 AND partial=0",
    )?;
    let names = statement
        .query_map(params![table, origin], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut actual = names
        .iter()
        .map(|name| {
            Ok(index_key_columns(conn, name)?
                .unwrap_or_default()
                .into_iter()
                .map(|(column, _)| column)
                .collect::<Vec<_>>())
        })
        .collect::<Result<Vec<_>>>()?;
    actual.sort();
    let mut expected = expected
        .iter()
        .map(|columns| {
            columns
                .iter()
                .map(|column| (*column).to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    expected.sort();
    Ok(actual == expected)
}

fn named_index_matches(
    conn: &Connection,
    table: &str,
    index: &str,
    expected_columns: &[&str],
) -> Result<bool> {
    let metadata = conn
        .query_row(
            "SELECT [unique], origin, partial FROM pragma_index_list(?1) WHERE name = ?2",
            params![table, index],
            |row| {
                Ok((
                    row.get::<_, bool>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()?;
    if metadata != Some((false, "c".to_string(), false)) {
        return Ok(false);
    }
    let Some(columns) = index_key_columns(conn, index)? else {
        return Ok(false);
    };
    Ok(columns.len() == expected_columns.len()
        && columns
            .iter()
            .zip(expected_columns)
            .all(|((actual, collation), expected)| actual == expected && collation == "BINARY"))
}

fn named_unique_partial_index_matches(
    conn: &Connection,
    table: &str,
    index: &str,
    expected_columns: &[&str],
) -> Result<bool> {
    let metadata = conn
        .query_row(
            "SELECT [unique],origin,partial FROM pragma_index_list(?1) WHERE name=?2",
            params![table, index],
            |row| {
                Ok((
                    row.get::<_, bool>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()?;
    if metadata != Some((true, "c".into(), true)) {
        return Ok(false);
    }
    let Some(columns) = index_key_columns(conn, index)? else {
        return Ok(false);
    };
    Ok(columns.len() == expected_columns.len()
        && columns
            .iter()
            .zip(expected_columns)
            .all(|((actual, collation), expected)| actual == expected && collation == "BINARY"))
}

fn foreign_keys_match(
    conn: &Connection,
    table: &str,
    expected: &[(&str, &str, &str, &str, &str)],
) -> Result<bool> {
    let mut statement = conn.prepare(&format!("PRAGMA foreign_key_list({table})"))?;
    let actual = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(3)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let Some(mut actual) = actual
        .into_iter()
        .map(|(from, table, to, on_update, on_delete, match_kind)| {
            Some((from, table, to?, on_update, on_delete, match_kind))
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(false);
    };
    actual.sort();
    let mut expected = expected
        .iter()
        .map(|(from, target, to, on_update, on_delete)| {
            (
                (*from).to_string(),
                (*target).to_string(),
                (*to).to_string(),
                (*on_update).to_string(),
                (*on_delete).to_string(),
                "NONE".to_string(),
            )
        })
        .collect::<Vec<_>>();
    expected.sort();
    Ok(actual == expected)
}
