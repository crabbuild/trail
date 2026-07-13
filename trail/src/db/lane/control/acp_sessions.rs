use super::*;

impl Trail {
    pub fn upsert_lane_acp_session(
        &mut self,
        acp_session_id: &str,
        upstream_session_id: Option<&str>,
        lane: &str,
        trail_session_id: &str,
        cwd: &str,
        path_mappings: &[AcpPathMapping],
        provider: Option<&str>,
        model: Option<&str>,
        upstream_command_json: Option<&str>,
        status: &str,
    ) -> Result<LaneAcpSession> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let acp_session_id = validate_external_id("ACP session id", acp_session_id)?;
        let upstream_session_id = upstream_session_id
            .map(|value| validate_external_id("upstream ACP session id", value))
            .transpose()?;
        let cwd = cwd.trim();
        if cwd.is_empty() {
            return Err(Error::InvalidInput(
                "ACP session cwd cannot be empty".to_string(),
            ));
        }
        let status = validate_acp_session_status(status)?;
        let path_mappings_json = serde_json::to_string(path_mappings)?;
        let branch = self.lane_branch(lane)?;
        let session = self.lane_session(trail_session_id)?;
        if session.lane_id != branch.lane_id {
            return Err(Error::InvalidInput(format!(
                "session `{trail_session_id}` does not belong to lane `{lane}`"
            )));
        }

        let now = now_ts();
        let existing = self.try_lane_acp_session(acp_session_id)?;
        let created_at = existing
            .as_ref()
            .map(|mapping| mapping.created_at)
            .unwrap_or(now);
        self.conn.execute(
            "INSERT INTO lane_acp_sessions \
             (acp_session_id, upstream_session_id, lane_id, trail_session_id, cwd, path_mappings_json, provider, model, upstream_command_json, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT(acp_session_id) DO UPDATE SET \
                upstream_session_id = excluded.upstream_session_id, \
                lane_id = excluded.lane_id, \
                trail_session_id = excluded.trail_session_id, \
                cwd = excluded.cwd, \
                path_mappings_json = excluded.path_mappings_json, \
                provider = excluded.provider, \
                model = excluded.model, \
                upstream_command_json = excluded.upstream_command_json, \
                status = excluded.status, \
                updated_at = excluded.updated_at",
            params![
                acp_session_id,
                upstream_session_id,
                branch.lane_id,
                trail_session_id,
                cwd,
                path_mappings_json,
                provider,
                model,
                upstream_command_json,
                status,
                created_at,
                now
            ],
        )?;
        self.lane_acp_session(acp_session_id)
    }

    pub fn update_lane_acp_session_status(
        &mut self,
        acp_session_id: &str,
        status: &str,
    ) -> Result<LaneAcpSession> {
        let _lock = self.acquire_write_lock()?;
        let acp_session_id = validate_external_id("ACP session id", acp_session_id)?;
        let status = validate_acp_session_status(status)?;
        let existing = self.lane_acp_session(acp_session_id)?;
        self.conn.execute(
            "UPDATE lane_acp_sessions SET status = ?1, updated_at = ?2 WHERE acp_session_id = ?3",
            params![status, now_ts(), acp_session_id],
        )?;
        self.insert_lane_event_with_context(
            &existing.lane_id,
            Some(&existing.trail_session_id),
            None,
            "acp_session_status_changed",
            None,
            None,
            &serde_json::json!({
                "protocol": "acp",
                "acp_session_id": acp_session_id,
                "status": status
            }),
        )?;
        self.lane_acp_session(acp_session_id)
    }

    pub fn lane_acp_session(&self, acp_session_id: &str) -> Result<LaneAcpSession> {
        self.try_lane_acp_session(acp_session_id)?
            .ok_or_else(|| Error::InvalidInput(format!("ACP session `{acp_session_id}` not found")))
    }

    pub fn list_lane_acp_sessions(&self, lane: Option<&str>) -> Result<AcpSessionListReport> {
        let sessions = if let Some(lane) = lane {
            let branch = self.lane_branch(lane)?;
            let mut stmt = self.conn.prepare(
                "SELECT acp_session_id, upstream_session_id, lane_id, trail_session_id, cwd, path_mappings_json, provider, model, upstream_command_json, status, created_at, updated_at \
                 FROM lane_acp_sessions WHERE lane_id = ?1 ORDER BY updated_at DESC, acp_session_id DESC",
            )?;
            let rows = stmt.query_map(params![branch.lane_id], lane_acp_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT acp_session_id, upstream_session_id, lane_id, trail_session_id, cwd, path_mappings_json, provider, model, upstream_command_json, status, created_at, updated_at \
                 FROM lane_acp_sessions ORDER BY updated_at DESC, acp_session_id DESC",
            )?;
            let rows = stmt.query_map([], lane_acp_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(AcpSessionListReport { sessions })
    }

    pub fn try_lane_acp_session(&self, acp_session_id: &str) -> Result<Option<LaneAcpSession>> {
        let acp_session_id = validate_external_id("ACP session id", acp_session_id)?;
        self.conn
            .query_row(
                "SELECT acp_session_id, upstream_session_id, lane_id, trail_session_id, cwd, path_mappings_json, provider, model, upstream_command_json, status, created_at, updated_at \
                 FROM lane_acp_sessions WHERE acp_session_id = ?1",
                params![acp_session_id],
                lane_acp_session_row,
            )
            .optional()
            .map_err(Error::from)
    }
}

pub(super) fn lane_acp_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LaneAcpSession> {
    let path_mappings_json: String = row.get(5)?;
    let path_mappings = serde_json::from_str(&path_mappings_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(LaneAcpSession {
        acp_session_id: row.get(0)?,
        upstream_session_id: row.get(1)?,
        lane_id: row.get(2)?,
        trail_session_id: row.get(3)?,
        cwd: row.get(4)?,
        path_mappings,
        provider: row.get(6)?,
        model: row.get(7)?,
        upstream_command_json: row.get(8)?,
        status: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn validate_external_id<'a>(label: &str, value: &'a str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::InvalidInput(format!("{label} cannot be empty")));
    }
    if value.len() > 512 {
        return Err(Error::InvalidInput(format!(
            "{label} cannot exceed 512 bytes"
        )));
    }
    Ok(value)
}

fn validate_acp_session_status(value: &str) -> Result<&'static str> {
    match value.trim() {
        "starting" => Ok("starting"),
        "active" => Ok("active"),
        "loaded" => Ok("loaded"),
        "resumed" => Ok("resumed"),
        "closed" => Ok("closed"),
        "failed" => Ok("failed"),
        "cancelled" => Ok("cancelled"),
        other => Err(Error::InvalidInput(format!(
            "ACP session status must be starting, active, loaded, resumed, closed, failed, or cancelled, got `{other}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_mappings_round_trip_through_create_list_and_update() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "mapping fixture\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        db.spawn_lane("mapping", None, false, None, None).unwrap();
        let session = db
            .start_lane_session("mapping", Some("mapping".to_string()), None)
            .unwrap()
            .session;
        let mappings = vec![
            AcpPathMapping {
                original: "/workspace/packages/app".to_string(),
                effective: "/lane/packages/app".to_string(),
                isolated: true,
            },
            AcpPathMapping {
                original: "/external".to_string(),
                effective: "/external".to_string(),
                isolated: false,
            },
        ];

        let created = db
            .upsert_lane_acp_session(
                "acp-mapping",
                Some("upstream-mapping"),
                "mapping",
                &session.session_id,
                "/lane/packages/app",
                &mappings,
                Some("test"),
                None,
                None,
                "active",
            )
            .unwrap();
        assert_eq!(created.path_mappings, mappings);
        assert_eq!(
            db.list_lane_acp_sessions(Some("mapping")).unwrap().sessions[0].path_mappings,
            mappings
        );
        assert_eq!(
            db.update_lane_acp_session_status("acp-mapping", "closed")
                .unwrap()
                .path_mappings,
            mappings
        );
    }
}
