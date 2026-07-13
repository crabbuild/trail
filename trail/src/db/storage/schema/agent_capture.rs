use super::*;

impl Trail {
    pub(super) fn ensure_agent_capture_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_hook_installations (
                 installation_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 scope TEXT NOT NULL,
                 config_path TEXT NOT NULL,
                 lane_id TEXT,
                 manifest_digest TEXT NOT NULL,
                 manifest_signature_json TEXT,
                 ownership_inventory_json TEXT NOT NULL,
                 config_before_digest TEXT,
                 config_after_digest TEXT NOT NULL,
                 adapter_version TEXT NOT NULL,
                 provider_version_range TEXT,
                 detected_provider_version TEXT,
                 capability_status TEXT NOT NULL,
                 status TEXT NOT NULL,
                 installed_at INTEGER NOT NULL,
                 verified_at INTEGER,
                 last_receipt_at INTEGER,
                 metadata_json TEXT
             );
             CREATE UNIQUE INDEX IF NOT EXISTS agent_hook_installations_target_idx
                 ON agent_hook_installations(workspace_id, provider, scope, config_path);
             CREATE INDEX IF NOT EXISTS agent_hook_installations_status_idx
                 ON agent_hook_installations(provider, status, verified_at);

             CREATE TABLE IF NOT EXISTS agent_capture_runs (
                 capture_run_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 lane_id TEXT,
                 workdir TEXT NOT NULL,
                 canonical_workdir TEXT NOT NULL,
                 owner_agent TEXT NOT NULL,
                 owner_session_id TEXT NOT NULL,
                 executor_agent TEXT,
                 work_item_id TEXT,
                 status TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 expires_at INTEGER NOT NULL,
                 ended_at INTEGER,
                 metadata_json TEXT
             );
             CREATE INDEX IF NOT EXISTS agent_capture_runs_active_workdir_idx
                 ON agent_capture_runs(workspace_id, canonical_workdir, expires_at)
                 WHERE status = 'active';
             CREATE INDEX IF NOT EXISTS agent_capture_runs_owner_idx
                 ON agent_capture_runs(owner_agent, owner_session_id, updated_at);

             CREATE TABLE IF NOT EXISTS lane_agent_sessions (
                 mapping_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 native_session_id TEXT NOT NULL,
                 parent_native_session_id TEXT,
                 trail_session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 capture_run_id TEXT REFERENCES agent_capture_runs(capture_run_id),
                 primary_transport TEXT NOT NULL,
                 transcript_identity TEXT,
                 transcript_offset INTEGER,
                 resume_json TEXT,
                 last_attestation_id TEXT,
                 status TEXT NOT NULL,
                 pending_turn_outcome TEXT,
                 session_close_requested INTEGER NOT NULL DEFAULT 0,
                 capture_epoch INTEGER NOT NULL DEFAULT 1,
                 finalization_owner TEXT,
                 finalization_lease_expires_at INTEGER,
                 next_receive_sequence INTEGER NOT NULL DEFAULT 1,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 UNIQUE(workspace_id, provider, native_session_id)
             );
             CREATE INDEX IF NOT EXISTS lane_agent_sessions_trail_session_idx
                 ON lane_agent_sessions(trail_session_id, updated_at);
             CREATE INDEX IF NOT EXISTS lane_agent_sessions_lane_idx
                 ON lane_agent_sessions(lane_id, updated_at);
             CREATE INDEX IF NOT EXISTS lane_agent_sessions_run_idx
                 ON lane_agent_sessions(capture_run_id, updated_at);
             CREATE INDEX IF NOT EXISTS lane_agent_sessions_finalization_idx
                 ON lane_agent_sessions(status, finalization_lease_expires_at)
                 WHERE status = 'finalizing';

             CREATE TABLE IF NOT EXISTS lane_agent_session_aliases (
                 workspace_id TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 native_session_alias TEXT NOT NULL,
                 mapping_id TEXT NOT NULL REFERENCES lane_agent_sessions(mapping_id),
                 reason TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 PRIMARY KEY(workspace_id, provider, native_session_alias)
             );
             CREATE INDEX IF NOT EXISTS lane_agent_session_aliases_mapping_idx
                 ON lane_agent_session_aliases(mapping_id);

             CREATE TABLE IF NOT EXISTS lane_artifacts (
                 artifact_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT REFERENCES lane_turns(turn_id),
                 provider TEXT NOT NULL,
                 artifact_kind TEXT NOT NULL,
                 format TEXT NOT NULL,
                 source TEXT NOT NULL,
                 source_locator_redacted TEXT,
                 content_object_id TEXT,
                 content_digest TEXT NOT NULL,
                 size_bytes INTEGER NOT NULL,
                 start_offset INTEGER,
                 end_offset INTEGER,
                 redaction_profile TEXT,
                 retention_status TEXT NOT NULL,
                 trust TEXT NOT NULL,
                 supersedes_artifact_id TEXT REFERENCES lane_artifacts(artifact_id),
                 created_at INTEGER NOT NULL,
                 metadata_json TEXT
             );
             CREATE INDEX IF NOT EXISTS lane_artifacts_session_idx
                 ON lane_artifacts(session_id, created_at, artifact_id);
             CREATE INDEX IF NOT EXISTS lane_artifacts_turn_idx
                 ON lane_artifacts(turn_id, created_at, artifact_id);
             CREATE INDEX IF NOT EXISTS lane_artifacts_digest_idx
                 ON lane_artifacts(content_digest, artifact_kind);

             CREATE TABLE IF NOT EXISTS agent_hook_receipts (
                 receipt_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 installation_id TEXT REFERENCES agent_hook_installations(installation_id),
                 mapping_id TEXT REFERENCES lane_agent_sessions(mapping_id),
                 provider TEXT NOT NULL,
                 native_event TEXT NOT NULL,
                 native_session_id TEXT,
                 native_turn_id TEXT,
                 transport TEXT NOT NULL,
                 dedupe_key TEXT NOT NULL,
                 payload_digest TEXT NOT NULL,
                 raw_object_id TEXT NOT NULL REFERENCES objects(object_id),
                 raw_artifact_id TEXT REFERENCES lane_artifacts(artifact_id),
                 receive_sequence INTEGER,
                 connection_id TEXT,
                 direction TEXT,
                 connection_sequence INTEGER,
                 status TEXT NOT NULL,
                 attempt_count INTEGER NOT NULL DEFAULT 0,
                 next_attempt_at INTEGER,
                 diagnostic TEXT,
                 occurred_at INTEGER,
                 received_at INTEGER NOT NULL,
                 processed_at INTEGER,
                 updated_at INTEGER NOT NULL,
                 UNIQUE(workspace_id, provider, dedupe_key),
                 UNIQUE(mapping_id, receive_sequence)
             );
             CREATE INDEX IF NOT EXISTS agent_hook_receipts_replay_idx
                 ON agent_hook_receipts(status, next_attempt_at, received_at);
             CREATE INDEX IF NOT EXISTS agent_hook_receipts_session_idx
                 ON agent_hook_receipts(workspace_id, provider, native_session_id, received_at);
             CREATE INDEX IF NOT EXISTS agent_hook_receipts_turn_idx
                 ON agent_hook_receipts(native_turn_id, received_at);

             CREATE TABLE IF NOT EXISTS lane_turn_evidence_manifests (
                 manifest_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT NOT NULL UNIQUE REFERENCES lane_turns(turn_id),
                 schema_version INTEGER NOT NULL,
                 object_id TEXT NOT NULL,
                 digest TEXT NOT NULL UNIQUE,
                 created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS lane_turn_evidence_manifests_session_idx
                 ON lane_turn_evidence_manifests(session_id, created_at);

             CREATE TABLE IF NOT EXISTS lane_provenance_nodes (
                 provenance_node_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT REFERENCES lane_turns(turn_id),
                 node_kind TEXT NOT NULL,
                 summary TEXT NOT NULL,
                 event_id TEXT,
                 span_id TEXT,
                 message_id TEXT,
                 change_id TEXT,
                 artifact_id TEXT REFERENCES lane_artifacts(artifact_id),
                 source_confidence TEXT NOT NULL,
                 classifier_version TEXT,
                 created_at INTEGER NOT NULL,
                 attributes_json TEXT
             );
             CREATE INDEX IF NOT EXISTS lane_provenance_nodes_session_idx
                 ON lane_provenance_nodes(session_id, turn_id, node_kind, created_at);
             CREATE INDEX IF NOT EXISTS lane_provenance_nodes_change_idx
                 ON lane_provenance_nodes(change_id, created_at);

             CREATE TABLE IF NOT EXISTS lane_provenance_edges (
                 provenance_edge_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 from_node_id TEXT NOT NULL REFERENCES lane_provenance_nodes(provenance_node_id),
                 to_node_id TEXT NOT NULL REFERENCES lane_provenance_nodes(provenance_node_id),
                 relation TEXT NOT NULL,
                 source_confidence TEXT NOT NULL,
                 receipt_id TEXT REFERENCES agent_hook_receipts(receipt_id),
                 created_at INTEGER NOT NULL,
                 attributes_json TEXT
             );
             CREATE UNIQUE INDEX IF NOT EXISTS lane_provenance_edges_identity_idx
                 ON lane_provenance_edges(
                     from_node_id, to_node_id, relation, COALESCE(receipt_id, '')
                 );
             CREATE INDEX IF NOT EXISTS lane_provenance_edges_from_idx
                 ON lane_provenance_edges(from_node_id, relation);
             CREATE INDEX IF NOT EXISTS lane_provenance_edges_to_idx
                 ON lane_provenance_edges(to_node_id, relation);

             CREATE TABLE IF NOT EXISTS lane_session_attestations (
                 attestation_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 capture_run_id TEXT REFERENCES agent_capture_runs(capture_run_id),
                 previous_attestation_id TEXT REFERENCES lane_session_attestations(attestation_id),
                 statement_object_id TEXT NOT NULL,
                 statement_digest TEXT NOT NULL UNIQUE,
                 signature_json TEXT,
                 status TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 superseded_by TEXT REFERENCES lane_session_attestations(attestation_id),
                 metadata_json TEXT
             );
             CREATE INDEX IF NOT EXISTS lane_session_attestations_session_idx
                 ON lane_session_attestations(session_id, created_at);

             CREATE TABLE IF NOT EXISTS agent_attestation_key_revocations (
                 key_id TEXT PRIMARY KEY,
                 public_key_hex TEXT NOT NULL,
                 reason TEXT NOT NULL,
                 revoked_at INTEGER NOT NULL,
                 metadata_json TEXT
             );
             CREATE INDEX IF NOT EXISTS agent_attestation_key_revocations_time_idx
                 ON agent_attestation_key_revocations(revoked_at, key_id);

             CREATE TABLE IF NOT EXISTS lane_session_attestation_turns (
                 attestation_id TEXT NOT NULL REFERENCES lane_session_attestations(attestation_id),
                 turn_id TEXT NOT NULL REFERENCES lane_turns(turn_id),
                 change_id TEXT,
                 evidence_manifest_id TEXT NOT NULL REFERENCES lane_turn_evidence_manifests(manifest_id),
                 PRIMARY KEY(attestation_id, turn_id)
             );

             CREATE TABLE IF NOT EXISTS lane_learnings (
                 learning_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT REFERENCES lane_turns(turn_id),
                 scope TEXT NOT NULL,
                 body TEXT NOT NULL,
                 status TEXT NOT NULL,
                 confidence REAL,
                 source_artifact_id TEXT REFERENCES lane_artifacts(artifact_id),
                 anchor_json TEXT,
                 created_at INTEGER NOT NULL,
                 reviewed_at INTEGER,
                 reviewer TEXT,
                 expires_at INTEGER,
                 superseded_by TEXT REFERENCES lane_learnings(learning_id),
                 metadata_json TEXT
             );
             CREATE INDEX IF NOT EXISTS lane_learnings_scope_idx
                 ON lane_learnings(lane_id, scope, status, created_at);
             CREATE INDEX IF NOT EXISTS lane_learnings_session_idx
                 ON lane_learnings(session_id, turn_id, created_at);

             CREATE TABLE IF NOT EXISTS git_agent_links (
                 git_agent_link_id TEXT PRIMARY KEY,
                 git_commit TEXT NOT NULL,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT REFERENCES lane_turns(turn_id),
                 from_change TEXT,
                 through_change TEXT,
                 confidence TEXT NOT NULL,
                 source TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 metadata_json TEXT
             );
             CREATE UNIQUE INDEX IF NOT EXISTS git_agent_links_identity_idx
                 ON git_agent_links(git_commit, session_id, COALESCE(turn_id, ''), source);
             CREATE INDEX IF NOT EXISTS git_agent_links_session_idx
                 ON git_agent_links(session_id, created_at);
             CREATE INDEX IF NOT EXISTS git_agent_links_turn_idx
                 ON git_agent_links(turn_id, created_at);",
        )?;
        ensure_column(&self.conn, "agent_hook_receipts", "connection_id", "TEXT")?;
        ensure_column(&self.conn, "agent_hook_receipts", "direction", "TEXT")?;
        ensure_column(
            &self.conn,
            "agent_hook_receipts",
            "connection_sequence",
            "INTEGER",
        )?;
        self.conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS agent_hook_receipts_connection_sequence_idx
             ON agent_hook_receipts(connection_id, direction, connection_sequence)
             WHERE connection_id IS NOT NULL
               AND direction IS NOT NULL
               AND connection_sequence IS NOT NULL;",
        )?;
        Ok(())
    }
}

pub(in crate::db::storage) fn agent_capture_schema_complete(conn: &Connection) -> Result<bool> {
    const REQUIRED_TABLES: [&str; 14] = [
        "agent_hook_installations",
        "agent_capture_runs",
        "lane_agent_sessions",
        "lane_agent_session_aliases",
        "lane_artifacts",
        "agent_hook_receipts",
        "lane_turn_evidence_manifests",
        "lane_provenance_nodes",
        "lane_provenance_edges",
        "lane_session_attestations",
        "agent_attestation_key_revocations",
        "lane_session_attestation_turns",
        "lane_learnings",
        "git_agent_links",
    ];
    for table in REQUIRED_TABLES {
        let exists = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Ok(false);
        }
    }

    let mapping_columns = schema_table_columns(conn, "lane_agent_sessions")?;
    let receipt_columns = schema_table_columns(conn, "agent_hook_receipts")?;
    let artifact_columns = schema_table_columns(conn, "lane_artifacts")?;
    Ok(mapping_columns.contains("capture_epoch")
        && mapping_columns.contains("finalization_owner")
        && mapping_columns.contains("finalization_lease_expires_at")
        && mapping_columns.contains("next_receive_sequence")
        && receipt_columns.contains("receive_sequence")
        && receipt_columns.contains("raw_object_id")
        && receipt_columns.contains("attempt_count")
        && receipt_columns.contains("next_attempt_at")
        && receipt_columns.contains("connection_id")
        && receipt_columns.contains("direction")
        && receipt_columns.contains("connection_sequence")
        && artifact_columns.contains("retention_status")
        && artifact_columns.contains("trust"))
}

fn schema_table_columns(conn: &Connection, table: &str) -> Result<BTreeSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(Error::from)?;
    Ok(columns)
}
