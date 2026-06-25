use super::*;

impl CrabDb {
    pub(crate) fn put_text_content_from_lines(&self, lines: &[LineEntry]) -> Result<ObjectId> {
        let bytes = materialize_lines(lines);
        let mut order_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut index_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        for (idx, entry) in lines.iter().enumerate() {
            let key = order_key(idx as u64 + 1);
            order_builder.add(key.clone(), cbor(entry)?);
            index_builder.add(entry.line_id.encode_key(), key);
        }
        let order_tree = order_builder.build()?;
        let index_tree = index_builder.build()?;
        let content = TextContent {
            version: TEXT_OBJECT_VERSION,
            content_hash: sha256_hex(&bytes),
            line_count: lines.len() as u64,
            byte_count: bytes.len() as u64,
            order_map_root: tree_root_hex(&order_tree),
            line_index_map_root: tree_root_hex(&index_tree),
            representation: TextRepresentation::TreeText,
        };
        self.put_object(TEXT_CONTENT_KIND, TEXT_OBJECT_VERSION, &content)
    }

    pub(crate) fn put_blob(&self, bytes: Vec<u8>) -> Result<ObjectId> {
        let blob = Blob {
            version: BLOB_OBJECT_VERSION,
            content_hash: sha256_hex(&bytes),
            bytes,
        };
        self.put_object(BLOB_KIND, BLOB_OBJECT_VERSION, &blob)
    }

    pub(crate) fn put_object<T: Serialize>(
        &self,
        kind: &str,
        version: u16,
        value: &T,
    ) -> Result<ObjectId> {
        let bytes = cbor(value)?;
        let object_id = ObjectId::for_bytes(kind, version, &bytes);
        self.conn.execute(
            "INSERT OR IGNORE INTO objects \
             (object_id, kind, version, codec, hash_alg, size_bytes, bytes, created_at) \
             VALUES (?1, ?2, ?3, 'cbor', 'sha256', ?4, ?5, ?6)",
            params![
                object_id.0,
                kind,
                version as i64,
                bytes.len() as i64,
                bytes,
                now_ts()
            ],
        )?;
        Ok(object_id)
    }

    pub(crate) fn get_object<T: serde::de::DeserializeOwned>(
        &self,
        kind: &'static str,
        object_id: &ObjectId,
    ) -> Result<T> {
        let row: Option<(String, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT kind, bytes FROM objects WHERE object_id = ?1",
                params![object_id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((actual_kind, bytes)) = row else {
            return Err(Error::ObjectNotFound {
                kind,
                id: object_id.0.clone(),
            });
        };
        if actual_kind != kind {
            return Err(Error::Corrupt(format!(
                "object {} has kind {}, expected {}",
                object_id.0, actual_kind, kind
            )));
        }
        from_cbor(&bytes)
    }

    pub(crate) fn store_operation(&self, operation: &Operation) -> Result<ObjectId> {
        let operation_id = self.put_object(OPERATION_KIND, OP_OBJECT_VERSION, operation)?;
        self.index_operation(operation, &operation_id)?;
        Ok(operation_id)
    }

    pub(crate) fn index_operation(
        &self,
        operation: &Operation,
        operation_id: &ObjectId,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO operations \
             (change_id, operation_id, kind, branch, before_root, after_root, actor_kind, actor_id, session_id, message, path_count, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                operation.change_id.0,
                operation_id.0,
                format!("{:?}", operation.kind),
                operation.branch,
                operation.before_root.as_ref().map(|id| id.0.clone()),
                operation.after_root.0,
                format!("{:?}", operation.actor.kind),
                operation.actor.id,
                operation.session_id,
                operation.message,
                operation.changes.len() as i64,
                operation.created_at
            ],
        )?;
        for (idx, parent) in operation.parents.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO operation_parents (change_id, parent_change_id, position) VALUES (?1, ?2, ?3)",
                params![operation.change_id.0, parent.0, idx as i64],
            )?;
        }
        for change in &operation.changes {
            if let Some(file_id) = &change.file_id {
                self.conn.execute(
                    "INSERT INTO file_history \
                     (file_id, change_id, path, old_path, kind, before_hash, after_hash, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        file_id_key(file_id),
                        operation.change_id.0,
                        change.path,
                        change.old_path,
                        format!("{:?}", change.kind),
                        change.before_hash,
                        change.after_hash,
                        operation.created_at
                    ],
                )?;
                for line in &change.line_changes {
                    self.conn.execute(
                        "INSERT INTO line_history \
                         (line_id, file_id, change_id, path, line_number, kind, text_hash, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            line.line_id_key(),
                            file_id_key(file_id),
                            operation.change_id.0,
                            change.path,
                            line.new_line_number.or(line.old_line_number).map(|n| n as i64),
                            format!("{:?}", line.kind),
                            line.after_hash.clone().or_else(|| line.before_hash.clone()),
                            operation.created_at
                        ],
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn store_message(
        &self,
        role: &str,
        body: &str,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        change_id: Option<&ChangeId>,
        created_at: i64,
    ) -> Result<MessageId> {
        let id_seed = change_id.cloned().unwrap_or_else(|| {
            let seed = format!(
                "{}:{}:{}:{}:{}",
                self.config.workspace.id.0,
                role,
                agent_id.unwrap_or("none"),
                created_at,
                now_nanos()
            );
            ChangeId(format!(
                "msg_seed_{}",
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
            agent_id: agent_id.map(str::to_string),
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
             (message_id, role, body, agent_id, session_id, change_id, object_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                message.id.0.clone(),
                message.role.clone(),
                message.body.clone(),
                message.agent_id.clone(),
                message.session_id.clone(),
                message.change_id.as_ref().map(|id| id.0.clone()),
                object_id.0.clone(),
                message.created_at
            ],
        )?;
        Ok(())
    }
}
