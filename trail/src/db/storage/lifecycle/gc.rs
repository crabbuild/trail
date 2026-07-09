use super::*;

impl Trail {
    pub fn gc(&mut self, dry_run: bool) -> Result<GcReport> {
        let _lock = self.acquire_write_lock()?;
        let reachable = self.reachable_object_ids()?;
        let known_kinds = known_gc_object_kinds();
        let mut stmt = self
            .conn
            .prepare("SELECT object_id, kind FROM objects ORDER BY object_id")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut prunable = Vec::new();
        let mut total_known = 0;
        let mut preserved_unknown = 0;
        for row in rows {
            let (object_id, kind) = row?;
            if known_kinds.contains(kind.as_str()) {
                total_known += 1;
                if !reachable.contains(&object_id) {
                    prunable.push(object_id);
                }
            } else {
                preserved_unknown += 1;
            }
        }
        let mut report = GcReport {
            dry_run,
            total_known_objects: total_known,
            reachable_objects: reachable.len() as u64,
            prunable_objects: prunable.len() as u64,
            pruned_objects: 0,
            preserved_unknown_objects: preserved_unknown,
            errors: Vec::new(),
        };
        if !dry_run {
            for object_id in &prunable {
                self.conn.execute(
                    "DELETE FROM objects WHERE object_id = ?1",
                    params![object_id],
                )?;
                report.pruned_objects += 1;
            }
            let rebuild = self.rebuild_indexes_unlocked()?;
            report.errors.extend(rebuild.errors);
        }
        Ok(report)
    }

    pub(crate) fn reachable_object_ids(&self) -> Result<HashSet<String>> {
        let (operation_objects, mut errors) = self.operation_objects()?;
        let reachable_changes =
            self.reachable_operation_changes(&operation_objects, &mut errors)?;
        let by_change = operation_objects
            .iter()
            .map(|object| (object.operation.change_id.0.clone(), object))
            .collect::<HashMap<_, _>>();
        let mut reachable = HashSet::new();

        for reference in self.all_refs()? {
            reachable.insert(reference.root_id.0.clone());
            reachable.insert(reference.operation_id.0.clone());
            self.collect_root_reachable(&reference.root_id, &mut reachable, &mut errors);
        }

        for change_id in &reachable_changes {
            let Some(object) = by_change.get(change_id) else {
                continue;
            };
            reachable.insert(object.object_id.0.clone());
            if let Some(root_id) = &object.operation.before_root {
                self.collect_root_reachable(root_id, &mut reachable, &mut errors);
            }
            self.collect_root_reachable(&object.operation.after_root, &mut reachable, &mut errors);
        }

        for (object_id, _message) in self.message_objects(&mut errors)? {
            reachable.insert(object_id.0);
        }

        self.collect_lane_event_object_refs(&mut reachable, &mut errors)?;

        let mut stmt = self.conn.prepare("SELECT object_id FROM anchors")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            reachable.insert(row?);
        }

        if !errors.is_empty() {
            // GC should be conservative. Surface corruption to the caller rather
            // than deleting objects when reachability is uncertain.
            return Err(Error::Corrupt(errors.join("; ")));
        }
        Ok(reachable)
    }

    pub(crate) fn collect_lane_event_object_refs(
        &self,
        reachable: &mut HashSet<String>,
        errors: &mut Vec<String>,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, payload_json FROM lane_events ORDER BY created_at, event_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (event_id, payload_json) = row?;
            let payload = match serde_json::from_str::<serde_json::Value>(&payload_json) {
                Ok(payload) => payload,
                Err(err) => {
                    errors.push(format!("failed to decode lane event {event_id}: {err}"));
                    continue;
                }
            };
            for key in ["stdout_object", "stderr_object"] {
                if let Some(object_id) = payload.get(key).and_then(|value| value.as_str()) {
                    reachable.insert(object_id.to_string());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn collect_root_reachable(
        &self,
        root_id: &ObjectId,
        reachable: &mut HashSet<String>,
        errors: &mut Vec<String>,
    ) {
        reachable.insert(root_id.0.clone());
        match self.load_root_files(root_id) {
            Ok(files) => {
                for entry in files.values() {
                    match &entry.content {
                        FileContentRef::Text(text_id) => {
                            reachable.insert(text_id.0.clone());
                        }
                        FileContentRef::Opaque(blob_id) | FileContentRef::Binary(blob_id) => {
                            reachable.insert(blob_id.0.clone());
                        }
                    }
                }
            }
            Err(err) => errors.push(format!("failed to walk root {}: {err}", root_id.0)),
        }
    }
}
