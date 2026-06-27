use super::*;

impl CrabDb {
    pub(crate) fn allocate_session_id(&self, lane_id: &str, title: Option<&str>) -> String {
        let seed = format!(
            "{}:{}:{}:{}",
            self.config.workspace.id.0,
            lane_id,
            title.unwrap_or("session"),
            now_nanos()
        );
        format!("session_{}", crate::ids::short_hash(seed.as_bytes(), 16))
    }

    pub(crate) fn ensure_lane_session(
        &self,
        lane_id: &str,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<()> {
        validate_session_id(session_id)?;
        if let Some(existing) = self.try_lane_session(session_id)? {
            if existing.lane_id != lane_id {
                return Err(Error::InvalidInput(format!(
                    "session `{session_id}` belongs to another lane"
                )));
            }
            return Ok(());
        }
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO lane_sessions \
             (session_id, lane_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, lane_id, title, now],
        )?;
        Ok(())
    }

    pub(crate) fn preflight_lane_session_owner(
        &self,
        lane_id: &str,
        session_id: &str,
    ) -> Result<()> {
        validate_session_id(session_id)?;
        if let Some(existing) = self.try_lane_session(session_id)? {
            if existing.lane_id != lane_id {
                return Err(Error::InvalidInput(format!(
                    "session `{session_id}` belongs to another lane"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn try_lane_session(&self, session_id: &str) -> Result<Option<LaneSession>> {
        self.conn
            .query_row(
                "SELECT session_id, lane_id, title, status, started_at, ended_at, metadata_json \
                 FROM lane_sessions WHERE session_id = ?1",
                params![session_id],
                lane_session_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn lane_session(&self, session_id: &str) -> Result<LaneSession> {
        self.try_lane_session(session_id)?
            .ok_or_else(|| Error::InvalidInput(format!("session `{session_id}` not found")))
    }

    pub(crate) fn open_lane_turn(
        &self,
        lane_id: &str,
        session_id: Option<&str>,
        base_change: &ChangeId,
        before_change: &ChangeId,
        metadata_json: Option<&serde_json::Value>,
    ) -> Result<String> {
        if let Some(session_id) = session_id {
            self.ensure_lane_session(lane_id, session_id, None)?;
        }
        let seed = format!(
            "{}:{}:{}:{}:{}",
            lane_id,
            session_id.unwrap_or("none"),
            base_change.0,
            before_change.0,
            now_nanos()
        );
        let turn_id = format!("turn_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.conn.execute(
            "INSERT INTO lane_turns \
             (turn_id, lane_id, session_id, base_change, before_change, after_change, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, 'started', ?6, NULL, ?7)",
            params![
                turn_id,
                lane_id,
                session_id,
                base_change.0,
                before_change.0,
                now_ts(),
                metadata_json.map(serde_json::to_string).transpose()?
            ],
        )?;
        Ok(turn_id)
    }

    pub(crate) fn finish_lane_turn(
        &self,
        turn_id: &str,
        status: &str,
        after_change: Option<&ChangeId>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE lane_turns SET status = ?1, after_change = ?2, ended_at = ?3 WHERE turn_id = ?4",
            params![
                status,
                after_change.map(|change_id| change_id.0.clone()),
                now_ts(),
                turn_id
            ],
        )?;
        Ok(())
    }

    pub(crate) fn update_lane_turn_progress(
        &self,
        turn_id: &str,
        status: &str,
        after_change: Option<&ChangeId>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE lane_turns SET status = ?1, after_change = ?2 WHERE turn_id = ?3",
            params![
                status,
                after_change.map(|change_id| change_id.0.clone()),
                turn_id
            ],
        )?;
        Ok(())
    }

    pub(crate) fn lane_turn(&self, turn_id: &str) -> Result<LaneTurn> {
        self.conn
            .query_row(
                "SELECT turn_id, lane_id, session_id, base_change, before_change, after_change, status, started_at, ended_at, metadata_json \
                 FROM lane_turns WHERE turn_id = ?1",
                params![turn_id],
                lane_turn_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("turn `{turn_id}` not found")))
    }

    pub(crate) fn lane_session_turns(&self, session_id: &str) -> Result<Vec<LaneTurn>> {
        let mut stmt = self.conn.prepare(
            "SELECT turn_id, lane_id, session_id, base_change, before_change, after_change, status, started_at, ended_at, metadata_json \
             FROM lane_turns WHERE session_id = ?1 ORDER BY started_at ASC, turn_id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], lane_turn_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn lane_session_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT object_id FROM messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(MESSAGE_KIND, &ObjectId(object_id)))
            .collect()
    }

    pub(crate) fn lane_session_events(&self, session_id: &str) -> Result<Vec<LaneEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, lane_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM lane_events WHERE session_id = ?1 ORDER BY created_at ASC, event_id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], lane_event_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn lane_session_operations(&self, session_id: &str) -> Result<Vec<TimelineEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![session_id], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn lane_turn_messages(&self, turn_id: &str) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT object_id FROM messages \
             WHERE message_id IN ( \
                 SELECT message_id FROM lane_events \
                 WHERE turn_id = ?1 AND message_id IS NOT NULL \
             ) \
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(MESSAGE_KIND, &ObjectId(object_id)))
            .collect()
    }

    pub(crate) fn lane_turn_events(&self, turn_id: &str) -> Result<Vec<LaneEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, lane_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM lane_events WHERE turn_id = ?1 ORDER BY created_at ASC, event_id ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], lane_event_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn lane_turn_operations(&self, turn_id: &str) -> Result<Vec<TimelineEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT o.change_id, o.kind, o.branch, o.actor_id, o.message, o.created_at, o.path_count \
             FROM operations o \
             JOIN lane_events e ON e.change_id = o.change_id \
             WHERE e.turn_id = ?1 \
             ORDER BY o.created_at ASC, o.change_id ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn lane_event(&self, event_id: &str) -> Result<LaneEventRecord> {
        self.conn
            .query_row(
                "SELECT event_id, lane_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
                 FROM lane_events WHERE event_id = ?1",
                params![event_id],
                lane_event_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("event `{event_id}` not found")))
    }
}
