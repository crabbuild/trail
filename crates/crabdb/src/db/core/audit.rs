use super::*;

impl CrabDb {
    pub(crate) fn record_external_mutation_audit(
        &mut self,
        mut input: ExternalMutationAuditInput,
    ) -> Result<String> {
        let _lock = self.acquire_write_lock()?;
        if let Some(lane_handle) = input.lane_id.clone() {
            if let Some((lane_id, ref_name)) = self.external_mutation_lane_identity(&lane_handle)? {
                input.lane_id = Some(lane_id);
                if input.target_ref.is_none() {
                    input.target_ref = Some(ref_name);
                }
            }
        }
        if input.lane_id.is_none() || input.target_ref.is_none() {
            if let Some((lane_id, ref_name)) = input
                .turn_id
                .as_deref()
                .map(|turn_id| self.external_mutation_turn_identity(turn_id))
                .transpose()?
                .flatten()
            {
                if input.lane_id.is_none() {
                    input.lane_id = Some(lane_id);
                }
                if input.target_ref.is_none() {
                    input.target_ref = Some(ref_name);
                }
            }
        }
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            input.actor,
            input.surface,
            input.command,
            input.status,
            input.status_code.unwrap_or_default(),
            now_nanos()
        );
        let audit_id = format!("audit_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        let summary = input.summary.map(redact_sensitive_json);
        let summary_json = summary.as_ref().map(serde_json::to_string).transpose()?;
        self.conn.execute(
            "INSERT INTO external_mutation_audit \
             (audit_id, actor, surface, command, target_ref, lane_id, status, status_code, change_id, summary_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                audit_id,
                input.actor,
                input.surface,
                input.command,
                input.target_ref,
                input.lane_id,
                input.status,
                input.status_code,
                input.change_id.map(|change_id| change_id.0),
                summary_json,
                now_ts()
            ],
        )?;
        Ok(audit_id)
    }

    pub fn list_external_mutation_audit(
        &self,
        limit: usize,
    ) -> Result<Vec<ExternalMutationAuditRecord>> {
        let limit = normalize_query_limit(limit, 1000)?;
        let mut stmt = self.conn.prepare(
            "SELECT audit_id, actor, surface, command, target_ref, lane_id, status, status_code, change_id, summary_json, created_at \
             FROM external_mutation_audit ORDER BY created_at DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], external_mutation_audit_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn external_mutation_lane_identity(&self, lane: &str) -> Result<Option<(String, String)>> {
        self.conn
            .query_row(
                "SELECT b.lane_id, b.ref_name \
                 FROM lane_branches b JOIN lanes l ON l.lane_id = b.lane_id \
                 WHERE b.lane_id = ?1 OR b.ref_name = ?1 OR l.name = ?1",
                params![lane],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(Error::from)
    }

    fn external_mutation_turn_identity(&self, turn_id: &str) -> Result<Option<(String, String)>> {
        self.conn
            .query_row(
                "SELECT t.lane_id, b.ref_name \
                 FROM lane_turns t JOIN lane_branches b ON b.lane_id = t.lane_id \
                 WHERE t.turn_id = ?1",
                params![turn_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(Error::from)
    }
}

fn external_mutation_audit_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ExternalMutationAuditRecord> {
    let summary_json: Option<String> = row.get(9)?;
    let summary =
        summary_json.and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
    Ok(ExternalMutationAuditRecord {
        audit_id: row.get(0)?,
        actor: row.get(1)?,
        surface: row.get(2)?,
        command: row.get(3)?,
        target_ref: row.get(4)?,
        lane_id: row.get(5)?,
        status: row.get(6)?,
        status_code: row.get(7)?,
        change_id: row.get::<_, Option<String>>(8)?.map(ChangeId),
        summary,
        created_at: row.get(10)?,
    })
}
