use super::*;

impl CrabDb {
    pub(crate) fn insert_agent_event(
        &self,
        agent_id: &str,
        event_type: &str,
        change_id: Option<&ChangeId>,
        message_id: Option<&MessageId>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        self.insert_agent_event_with_context(
            agent_id, None, None, event_type, change_id, message_id, payload,
        )
    }

    pub(crate) fn insert_agent_event_with_context(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        event_type: &str,
        change_id: Option<&ChangeId>,
        message_id: Option<&MessageId>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        let event_seed = format!(
            "{}:{}:{}:{}:{}:{}:{}",
            agent_id,
            session_id.unwrap_or("none"),
            turn_id.unwrap_or("none"),
            event_type,
            change_id.map(|id| id.0.as_str()).unwrap_or("none"),
            message_id.map(|id| id.0.as_str()).unwrap_or("none"),
            now_nanos()
        );
        let event_id = format!("evt_{}", crate::ids::short_hash(event_seed.as_bytes(), 16));
        let payload = redact_sensitive_json(payload.clone());
        self.conn.execute(
            "INSERT INTO agent_events \
             (event_id, agent_id, turn_id, session_id, event_type, change_id, message_id, payload_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                event_id,
                agent_id,
                turn_id,
                session_id,
                event_type,
                change_id.map(|id| id.0.clone()),
                message_id.map(|id| id.0.clone()),
                serde_json::to_string(&payload)?,
                now_ts()
            ],
        )?;
        Ok(event_id)
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

    pub(crate) fn agent_change_ids(&self, agent: &str) -> Result<Vec<ChangeId>> {
        let branch = self.agent_branch(agent)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id FROM operations \
             WHERE branch = ?1 OR actor_id = ?2 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![branch.ref_name, branch.agent_id], |row| {
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
