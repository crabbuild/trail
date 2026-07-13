use super::*;
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
