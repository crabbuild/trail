use super::*;

impl CrabDb {
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
}
