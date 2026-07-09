use super::*;

impl Trail {
    pub(crate) fn insert_lane_event(
        &self,
        lane_id: &str,
        event_type: &str,
        change_id: Option<&ChangeId>,
        message_id: Option<&MessageId>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        self.insert_lane_event_with_context(
            lane_id, None, None, event_type, change_id, message_id, payload,
        )
    }

    pub(crate) fn insert_lane_event_with_context(
        &self,
        lane_id: &str,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        event_type: &str,
        change_id: Option<&ChangeId>,
        message_id: Option<&MessageId>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        validate_lane_event_type_for_storage(event_type)?;
        let event_seed = format!(
            "{}:{}:{}:{}:{}:{}:{}",
            lane_id,
            session_id.unwrap_or("none"),
            turn_id.unwrap_or("none"),
            event_type,
            change_id.map(|id| id.0.as_str()).unwrap_or("none"),
            message_id.map(|id| id.0.as_str()).unwrap_or("none"),
            now_nanos()
        );
        let event_id = format!("evt_{}", crate::ids::short_hash(event_seed.as_bytes(), 16));
        let raw_payload_json = serde_json::to_string(payload)?;
        self.ensure_lane_event_payload_limit(event_type, &raw_payload_json)?;
        let payload = redact_sensitive_json(payload.clone());
        let payload_json = serde_json::to_string(&payload)?;
        self.ensure_lane_event_payload_limit(event_type, &payload_json)?;
        let created_at = now_ts();
        self.conn.execute(
            "INSERT INTO lane_events \
             (event_id, lane_id, turn_id, session_id, event_type, change_id, message_id, payload_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                event_id,
                lane_id,
                turn_id,
                session_id,
                event_type,
                change_id.map(|id| id.0.clone()),
                message_id.map(|id| id.0.clone()),
                payload_json,
                created_at
            ],
        )?;
        self.index_lane_trace_span_event(
            &event_id, lane_id, session_id, turn_id, event_type, &payload, created_at,
        )?;
        Ok(event_id)
    }

    fn ensure_lane_event_payload_limit(&self, event_type: &str, payload_json: &str) -> Result<()> {
        let payload_bytes = payload_json.as_bytes().len() as u64;
        let max_event_payload_bytes = self.config.lane.max_event_payload_bytes;
        if max_event_payload_bytes > 0 && payload_bytes > max_event_payload_bytes {
            return Err(Error::InvalidInput(format!(
                "lane event payload is {payload_bytes} bytes, exceeding lane.max_event_payload_bytes {max_event_payload_bytes}"
            )));
        }
        let max_trace_payload_bytes = self.config.lane.max_trace_payload_bytes;
        if matches!(event_type, "span_started" | "span_ended")
            && max_trace_payload_bytes > 0
            && payload_bytes > max_trace_payload_bytes
        {
            return Err(Error::InvalidInput(format!(
                "lane trace payload is {payload_bytes} bytes, exceeding lane.max_trace_payload_bytes {max_trace_payload_bytes}"
            )));
        }
        Ok(())
    }

    pub(crate) fn index_lane_trace_span_event(
        &self,
        event_id: &str,
        lane_id: &str,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        event_type: &str,
        payload: &serde_json::Value,
        created_at: i64,
    ) -> Result<()> {
        if !matches!(event_type, "span_started" | "span_ended") {
            return Ok(());
        }
        let Some(span_id) = payload_string(payload, "span_id") else {
            return Ok(());
        };
        let trace_id = payload_string(payload, "trace_id");
        self.conn.execute(
            "INSERT OR REPLACE INTO lane_trace_span_events \
             (span_id, event_id, event_type, trace_id, lane_id, session_id, turn_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                span_id, event_id, event_type, trace_id, lane_id, session_id, turn_id, created_at
            ],
        )?;
        Ok(())
    }

    pub(crate) fn rebuild_lane_trace_span_event_index(&self) -> Result<u64> {
        self.conn
            .execute("DELETE FROM lane_trace_span_events", [])?;
        let rows = {
            let mut stmt = self.conn.prepare(
                "SELECT event_id, lane_id, session_id, turn_id, event_type, payload_json, created_at \
                 FROM lane_events \
                 WHERE event_type IN ('span_started', 'span_ended') \
                 ORDER BY created_at ASC, rowid ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        let mut indexed = 0u64;
        for (event_id, lane_id, session_id, turn_id, event_type, payload_json, created_at) in rows {
            let payload = match serde_json::from_str::<serde_json::Value>(&payload_json) {
                Ok(payload) => payload,
                Err(_) => continue,
            };
            self.index_lane_trace_span_event(
                &event_id,
                &lane_id,
                session_id.as_deref(),
                turn_id.as_deref(),
                &event_type,
                &payload,
                created_at,
            )?;
            indexed += 1;
        }
        Ok(indexed)
    }

    pub(crate) fn messages_for_change(&self, change_id: &ChangeId) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT object_id FROM messages WHERE change_id = ?1 ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map(params![change_id.0], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(MESSAGE_KIND, &ObjectId(object_id)))
            .collect()
    }

    pub(crate) fn message(&self, message_id: &str) -> Result<Message> {
        let object_id: Option<String> = self
            .conn
            .query_row(
                "SELECT object_id FROM messages WHERE message_id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object_id) = object_id else {
            return Err(Error::InvalidInput(format!(
                "message `{message_id}` not found"
            )));
        };
        self.get_object(MESSAGE_KIND, &ObjectId(object_id))
    }

    pub(crate) fn object_info(&self, object_id: &str) -> Result<ObjectInfo> {
        self.conn
            .query_row(
                "SELECT object_id, kind, version, size_bytes, created_at FROM objects WHERE object_id = ?1",
                params![object_id],
                |row| {
                    Ok(ObjectInfo {
                        object_id: ObjectId(row.get(0)?),
                        kind: row.get(1)?,
                        version: row.get::<_, i64>(2)? as u16,
                        size_bytes: row.get::<_, i64>(3)? as u64,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("object `{object_id}` not found")))
    }

    pub(crate) fn file_history_by_path(&self, path: &str) -> Result<Vec<FileHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, change_id, path, old_path, kind, before_hash, after_hash, created_at \
             FROM file_history WHERE path = ?1 OR old_path = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![path], file_history_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn file_history_by_file_id(&self, file_id: &str) -> Result<Vec<FileHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, change_id, path, old_path, kind, before_hash, after_hash, created_at \
             FROM file_history WHERE file_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![file_id], file_history_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn line_history_by_line_id(&self, line_id: &str) -> Result<Vec<LineHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id, path, line_number, kind, text_hash, created_at \
             FROM line_history WHERE line_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![line_id], line_history_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn session_change_ids(&self, session_id: &str) -> Result<Vec<ChangeId>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id FROM operations WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| Ok(ChangeId(row.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn lane_change_ids(&self, lane: &str) -> Result<Vec<ChangeId>> {
        let branch = self.lane_branch(lane)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id FROM operations \
             WHERE branch = ?1 OR actor_id = ?2 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![branch.ref_name, branch.lane_id], |row| {
            Ok(ChangeId(row.get(0)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn operation(&self, change_id: &ChangeId) -> Result<Operation> {
        let object_id: Option<String> = self
            .conn
            .query_row(
                "SELECT operation_id FROM operations WHERE change_id = ?1",
                params![change_id.0],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object_id) = object_id else {
            return Err(Error::OperationNotFound(change_id.0.clone()));
        };
        self.get_object(OPERATION_KIND, &ObjectId(object_id))
    }
}

fn validate_lane_event_type_for_storage(event_type: &str) -> Result<()> {
    if event_type.is_empty() {
        return Err(Error::InvalidInput(
            "event type cannot be empty".to_string(),
        ));
    }
    if event_type.trim() != event_type {
        return Err(Error::InvalidInput(
            "event type cannot contain leading or trailing whitespace".to_string(),
        ));
    }
    if contains_sensitive_text(event_type) {
        return Err(Error::InvalidInput(
            "secret scan rejected lane event type; remove credentials from event metadata"
                .to_string(),
        ));
    }
    if event_type.len() > 128 {
        return Err(Error::InvalidInput(
            "event type cannot exceed 128 bytes".to_string(),
        ));
    }
    if !event_type
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(Error::InvalidInput(
            "event type may contain only ASCII letters, digits, `_`, `-`, and `.`".to_string(),
        ));
    }
    Ok(())
}
