use super::*;

impl Trail {
    pub(crate) fn store_message(
        &self,
        role: &str,
        body: &str,
        lane_id: Option<&str>,
        session_id: Option<&str>,
        change_id: Option<&ChangeId>,
        created_at: i64,
    ) -> Result<MessageId> {
        let id_seed = change_id.cloned().unwrap_or_else(|| {
            let seed = format!(
                "{}:{}:{}:{}:{}",
                self.config.workspace.id.0,
                role,
                lane_id.unwrap_or("none"),
                created_at,
                now_nanos()
            );
            ChangeId(format!(
                "change_message_seed_{}",
                crate::ids::short_hash(seed.as_bytes(), 16)
            ))
        });
        let body = redact_sensitive_text(body);
        let message_id = MessageId::new(&id_seed, role, &body);
        let message = Message {
            version: MESSAGE_OBJECT_VERSION,
            id: message_id.clone(),
            role: role.to_string(),
            body,
            lane_id: lane_id.map(str::to_string),
            session_id: session_id.map(str::to_string),
            change_id: change_id.cloned(),
            created_at,
        };
        let object_id = self.put_object(MESSAGE_KIND, MESSAGE_OBJECT_VERSION, &message)?;
        self.index_message(&message, &object_id)?;
        Ok(message_id)
    }

    pub(crate) fn index_anchor(&self, anchor: &Anchor, object_id: &ObjectId) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO anchors \
             (anchor_id, label, file_id, line_id, object_id, created_path, created_line, created_change, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                anchor.id.0.clone(),
                anchor.label.clone(),
                file_id_key(&anchor.file_id),
                line_id_key_value(&anchor.line_id),
                object_id.0.clone(),
                anchor.created_path.clone(),
                anchor.created_line as i64,
                anchor.created_change.0.clone(),
                anchor.created_at
            ],
        )?;
        Ok(())
    }

    pub(crate) fn anchor(&self, anchor_id: &str) -> Result<Anchor> {
        let object_id: Option<String> = self
            .conn
            .query_row(
                "SELECT object_id FROM anchors WHERE anchor_id = ?1",
                params![anchor_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object_id) = object_id else {
            return Err(Error::InvalidInput(format!(
                "anchor `{anchor_id}` not found"
            )));
        };
        self.get_object(ANCHOR_KIND, &ObjectId(object_id))
    }

    pub(crate) fn index_message(&self, message: &Message, object_id: &ObjectId) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO messages \
             (message_id, role, body, lane_id, session_id, change_id, object_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                message.id.0.clone(),
                message.role.clone(),
                message.body.clone(),
                message.lane_id.clone(),
                message.session_id.clone(),
                message.change_id.as_ref().map(|id| id.0.clone()),
                object_id.0.clone(),
                message.created_at
            ],
        )?;
        Ok(())
    }
}
