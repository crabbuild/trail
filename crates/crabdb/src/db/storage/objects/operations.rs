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
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let result = self.index_operation_in_transaction(operation, operation_id);
        if result.is_ok() {
            self.conn.execute_batch("COMMIT;")?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK;");
        }
        result
    }

    fn index_operation_in_transaction(
        &self,
        operation: &Operation,
        operation_id: &ObjectId,
    ) -> Result<()> {
        let path_count = self.operation_index_path_count(operation)?;
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
                path_count as i64,
                operation.created_at
            ],
        )?;
        let mut parent_insert = self.conn.prepare_cached(
            "INSERT INTO operation_parents (change_id, parent_change_id, position) VALUES (?1, ?2, ?3)",
        )?;
        for (idx, parent) in operation.parents.iter().enumerate() {
            parent_insert.execute(params![operation.change_id.0, parent.0, idx as i64])?;
        }
        drop(parent_insert);
        let mut file_insert = self.conn.prepare_cached(
            "INSERT INTO file_history \
             (file_id, change_id, path, old_path, kind, before_hash, after_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let mut line_insert = self.conn.prepare_cached(
            "INSERT INTO line_history \
             (line_id, file_id, change_id, path, line_number, kind, text_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for change in &operation.changes {
            if let Some(file_id) = &change.file_id {
                let file_id = file_id_key(file_id);
                let kind = format!("{:?}", change.kind);
                file_insert.execute(params![
                    file_id,
                    operation.change_id.0,
                    change.path,
                    change.old_path,
                    kind,
                    change.before_hash,
                    change.after_hash,
                    operation.created_at
                ])?;
                for line in &change.line_changes {
                    let line_kind = format!("{:?}", line.kind);
                    line_insert.execute(params![
                        line.line_id_key(),
                        file_id,
                        operation.change_id.0,
                        change.path,
                        line.new_line_number
                            .or(line.old_line_number)
                            .map(|n| n as i64),
                        line_kind,
                        line.after_hash.clone().or_else(|| line.before_hash.clone()),
                        operation.created_at
                    ])?;
                }
            }
        }
        Ok(())
    }

    fn operation_index_path_count(&self, operation: &Operation) -> Result<u64> {
        if !operation.changes.is_empty() || operation.before_root.is_some() {
            return Ok(operation.changes.len() as u64);
        }
        if !matches!(
            operation.kind,
            OperationKind::Init | OperationKind::GitImport
        ) {
            return Ok(0);
        }
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &operation.after_root)?;
        Ok(root.file_count)
    }
}
