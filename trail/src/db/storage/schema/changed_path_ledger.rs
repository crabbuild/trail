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
             epoch INTEGER NOT NULL DEFAULT 1 CHECK (epoch >= 1),
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
                 CHECK(dependency_kind IN ('builtin','trail_config','trailignore','gitignore','git_info_exclude','git_excludes_file','git_config','normalization','mode','case_policy')),
             content_identity BLOB NOT NULL,
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
                 CHECK (state IN ('open', 'sealed', 'retired', 'corrupt')),
             created_at INTEGER NOT NULL,
             sealed_at INTEGER,
             updated_at INTEGER NOT NULL,
             PRIMARY KEY (scope_id, epoch, segment_id),
             UNIQUE (scope_id, epoch, first_sequence),
             CHECK (folded_end_offset <= durable_end_offset)
         );
         CREATE INDEX changed_path_observer_segments_state_idx
             ON changed_path_observer_segments(scope_id, epoch, state, last_sequence);";

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
    let tables: [(&str, &[Column]); 10] = [
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
                ("epoch", "INTEGER", true, Some("1"), 0),
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
    ];
    for (table, expected) in tables {
        if !table_columns_match(conn, table, expected)? {
            return Ok(false);
        }
    }

    let table_fragments: [(&str, &[&str]); 10] = [
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
                "CHECK (epoch >= 1)",
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
            "changed_path_observer_segments",
            &[
                "CHECK (length(segment_id) > 0)",
                "CHECK (epoch >= 1)",
                "CHECK (log_format_version = 1)",
                "CHECK (first_sequence >= 1)",
                "CHECK (last_sequence IS NULL OR last_sequence >= first_sequence)",
                "CHECK (durable_end_offset >= 0)",
                "CHECK (folded_end_offset >= 0)",
                "CHECK (state IN ('open', 'sealed', 'retired', 'corrupt'))",
                "UNIQUE (scope_id, epoch, first_sequence)",
                "CHECK (folded_end_offset <= durable_end_offset)",
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
    ];
    for (table, fragments) in table_fragments {
        if !table_sql_contains(conn, table, fragments)? {
            return Ok(false);
        }
    }

    let primary_keys: [(&str, &[&str]); 10] = [
        ("changed_path_scopes", &["scope_id"]),
        ("changed_path_entries", &["scope_id", "normalized_path"]),
        ("changed_path_prefixes", &["scope_id", "normalized_prefix"]),
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
            "changed_path_observer_segments",
            &["scope_id", "epoch", "segment_id"],
        ),
        ("changed_path_observer_owners", &["scope_id"]),
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
    for table in [
        "changed_path_entries",
        "changed_path_prefixes",
        "changed_path_intents",
        "changed_path_intent_paths",
        "changed_path_intent_prefixes",
        "changed_path_reconciliations",
        "changed_path_reconciliation_rows",
    ] {
        if origin_index_count(conn, table, "u")? != 0 {
            return Ok(false);
        }
    }

    let named_indexes: [(&str, &str, &[&str]); 12] = [
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
    ];
    for (table, index, columns) in named_indexes {
        if !named_index_matches(conn, table, index, columns)? {
            return Ok(false);
        }
    }

    let expected_foreign_keys: [(&str, &[(&str, &str, &str, &str, &str)]); 10] = [
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
    Ok(true)
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
