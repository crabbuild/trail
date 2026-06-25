use super::*;

impl CrabDb {
    pub fn rebuild_indexes(&mut self) -> Result<IndexRebuildReport> {
        let _lock = self.acquire_write_lock()?;
        self.rebuild_indexes_unlocked()
    }

    pub(crate) fn rebuild_indexes_unlocked(&self) -> Result<IndexRebuildReport> {
        let (operation_objects, mut errors) = self.operation_objects()?;
        let reachable_changes =
            self.reachable_operation_changes(&operation_objects, &mut errors)?;
        self.conn.execute_batch(
            "\
            DELETE FROM operations;
            DELETE FROM operation_parents;
            DELETE FROM file_history;
            DELETE FROM line_history;
            DELETE FROM messages;
            ",
        )?;

        let mut by_change = operation_objects
            .into_iter()
            .map(|object| (object.operation.change_id.0.clone(), object))
            .collect::<HashMap<_, _>>();
        let mut changes = reachable_changes.into_iter().collect::<Vec<_>>();
        changes.sort();

        let mut report = IndexRebuildReport {
            errors,
            ..IndexRebuildReport::default()
        };
        for change_id in changes {
            let Some(object) = by_change.remove(&change_id) else {
                report.errors.push(format!(
                    "reachable operation missing from object map: {change_id}"
                ));
                continue;
            };
            report.operations += 1;
            report.operation_parents += object.operation.parents.len() as u64;
            for change in &object.operation.changes {
                if change.file_id.is_some() {
                    report.file_history_rows += 1;
                    report.line_history_rows += change.line_changes.len() as u64;
                }
            }
            self.index_operation(&object.operation, &object.object_id)?;
        }

        for (object_id, message) in self.message_objects(&mut report.errors)? {
            self.index_message(&message, &object_id)?;
            report.messages += 1;
        }

        Ok(report)
    }

    pub(crate) fn operation_objects(&self) -> Result<(Vec<OperationObject>, Vec<String>)> {
        let mut stmt = self
            .conn
            .prepare("SELECT object_id, bytes FROM objects WHERE kind = ?1 ORDER BY object_id")?;
        let rows = stmt.query_map(params![OPERATION_KIND], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut objects = Vec::new();
        let mut errors = Vec::new();
        for row in rows {
            let (object_id, bytes) = row?;
            match from_cbor::<Operation>(&bytes) {
                Ok(operation) => objects.push(OperationObject {
                    object_id: ObjectId(object_id),
                    operation,
                }),
                Err(err) => errors.push(format!(
                    "failed to decode operation object {object_id}: {err}"
                )),
            }
        }
        Ok((objects, errors))
    }

    pub(crate) fn message_objects(
        &self,
        errors: &mut Vec<String>,
    ) -> Result<Vec<(ObjectId, Message)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT object_id, bytes FROM objects WHERE kind = ?1 ORDER BY object_id")?;
        let rows = stmt.query_map(params![MESSAGE_KIND], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            let (object_id, bytes) = row?;
            match from_cbor::<Message>(&bytes) {
                Ok(message) => messages.push((ObjectId(object_id), message)),
                Err(err) => errors.push(format!(
                    "failed to decode message object {object_id}: {err}"
                )),
            }
        }
        Ok(messages)
    }

    pub(crate) fn reachable_operation_changes(
        &self,
        operation_objects: &[OperationObject],
        errors: &mut Vec<String>,
    ) -> Result<HashSet<String>> {
        let by_change = operation_objects
            .iter()
            .map(|object| (object.operation.change_id.0.clone(), object))
            .collect::<HashMap<_, _>>();
        let by_object = operation_objects
            .iter()
            .map(|object| {
                (
                    object.object_id.0.clone(),
                    object.operation.change_id.0.clone(),
                )
            })
            .collect::<HashMap<_, _>>();

        let mut stack = Vec::new();
        for reference in self.all_refs()? {
            match by_object.get(&reference.operation_id.0) {
                Some(change_id) => stack.push(change_id.clone()),
                None => errors.push(format!(
                    "ref {} points to missing operation object {}",
                    reference.name, reference.operation_id.0
                )),
            }
        }

        let mut reachable = HashSet::new();
        while let Some(change_id) = stack.pop() {
            if !reachable.insert(change_id.clone()) {
                continue;
            }
            let Some(object) = by_change.get(&change_id) else {
                errors.push(format!(
                    "operation {change_id} is reachable but missing from object table"
                ));
                continue;
            };
            for parent in &object.operation.parents {
                stack.push(parent.0.clone());
            }
        }
        Ok(reachable)
    }
}
