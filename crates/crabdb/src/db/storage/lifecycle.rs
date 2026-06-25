use super::*;

impl CrabDb {
    pub fn rebuild_indexes(&mut self) -> Result<IndexRebuildReport> {
        let _lock = self.acquire_write_lock()?;
        self.rebuild_indexes_unlocked()
    }

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

        self.collect_agent_event_object_refs(&mut reachable, &mut errors)?;

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

    pub(crate) fn collect_agent_event_object_refs(
        &self,
        reachable: &mut HashSet<String>,
        errors: &mut Vec<String>,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, payload_json FROM agent_events ORDER BY created_at, event_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (event_id, payload_json) = row?;
            let payload = match serde_json::from_str::<serde_json::Value>(&payload_json) {
                Ok(payload) => payload,
                Err(err) => {
                    errors.push(format!("failed to decode agent event {event_id}: {err}"));
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

    pub(crate) fn init_schema(&self) -> Result<()> {
        let user_version = self.schema_user_version()?;
        if user_version > CRABDB_SCHEMA_VERSION {
            return Err(Error::InvalidInput(format!(
                "CrabDB schema version {user_version} is newer than supported version {CRABDB_SCHEMA_VERSION}; upgrade this binary before opening the workspace"
            )));
        }
        self.conn.execute_batch(
            "\
            CREATE TABLE IF NOT EXISTS schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS objects (
                object_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                version INTEGER NOT NULL,
                codec TEXT NOT NULL,
                hash_alg TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                bytes BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS refs (
                name TEXT PRIMARY KEY,
                change_id TEXT NOT NULL,
                root_id TEXT NOT NULL,
                operation_id TEXT NOT NULL,
                generation INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS operations (
                change_id TEXT PRIMARY KEY,
                operation_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                branch TEXT NOT NULL,
                before_root TEXT,
                after_root TEXT NOT NULL,
                actor_kind TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                session_id TEXT,
                message TEXT,
                path_count INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS operations_branch_created_idx ON operations(branch, created_at);
            CREATE INDEX IF NOT EXISTS operations_session_created_idx ON operations(session_id, created_at);
            CREATE TABLE IF NOT EXISTS operation_parents (
                change_id TEXT NOT NULL,
                parent_change_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                PRIMARY KEY (change_id, position)
            );
            CREATE TABLE IF NOT EXISTS file_history (
                file_id TEXT NOT NULL,
                change_id TEXT NOT NULL,
                path TEXT NOT NULL,
                old_path TEXT,
                kind TEXT NOT NULL,
                before_hash TEXT,
                after_hash TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS file_history_file_idx ON file_history(file_id, created_at);
            CREATE INDEX IF NOT EXISTS file_history_path_idx ON file_history(path, created_at);
            CREATE TABLE IF NOT EXISTS line_history (
                line_id TEXT NOT NULL,
                file_id TEXT NOT NULL,
                change_id TEXT NOT NULL,
                path TEXT NOT NULL,
                line_number INTEGER,
                kind TEXT NOT NULL,
                text_hash TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS line_history_line_idx ON line_history(line_id, created_at);
            CREATE TABLE IF NOT EXISTS messages (
                message_id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                body TEXT NOT NULL,
                agent_id TEXT,
                session_id TEXT,
                change_id TEXT,
                object_id TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS anchors (
                anchor_id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                file_id TEXT NOT NULL,
                line_id TEXT NOT NULL,
                object_id TEXT NOT NULL,
                created_path TEXT NOT NULL,
                created_line INTEGER NOT NULL,
                created_change TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS anchors_file_idx ON anchors(file_id, created_at);
            CREATE INDEX IF NOT EXISTS anchors_line_idx ON anchors(line_id, created_at);
            CREATE TABLE IF NOT EXISTS agents (
                agent_id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                kind TEXT,
                provider TEXT,
                model TEXT,
                created_at INTEGER NOT NULL,
                metadata_json TEXT
            );
            CREATE TABLE IF NOT EXISTS agent_branches (
                agent_id TEXT PRIMARY KEY,
                ref_name TEXT NOT NULL UNIQUE,
                base_change TEXT NOT NULL,
                head_change TEXT NOT NULL,
                base_root TEXT NOT NULL,
                head_root TEXT NOT NULL,
                session_id TEXT,
                workdir TEXT,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agent_sessions (
                session_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                title TEXT,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                metadata_json TEXT
            );
            CREATE TABLE IF NOT EXISTS agent_turns (
                turn_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT,
                base_change TEXT NOT NULL,
                before_change TEXT NOT NULL,
                after_change TEXT,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                metadata_json TEXT
            );
            CREATE TABLE IF NOT EXISTS agent_events (
                event_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                turn_id TEXT,
                session_id TEXT,
                event_type TEXT NOT NULL,
                change_id TEXT,
                message_id TEXT,
                payload_json TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agent_approvals (
                approval_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT,
                turn_id TEXT,
                action TEXT NOT NULL,
                summary TEXT NOT NULL,
                payload_json TEXT,
                status TEXT NOT NULL,
                requested_at INTEGER NOT NULL,
                decided_at INTEGER,
                reviewer TEXT,
                note TEXT
            );
            CREATE INDEX IF NOT EXISTS agent_approvals_status_idx ON agent_approvals(status, requested_at);
            CREATE INDEX IF NOT EXISTS agent_approvals_agent_idx ON agent_approvals(agent_id, requested_at);
            CREATE TABLE IF NOT EXISTS agent_run_states (
                run_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT,
                turn_id TEXT,
                approval_id TEXT,
                status TEXT NOT NULL,
                reason TEXT NOT NULL,
                summary TEXT NOT NULL,
                state_json TEXT NOT NULL,
                interruption_json TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                resumed_at INTEGER,
                reviewer TEXT,
                note TEXT
            );
            CREATE INDEX IF NOT EXISTS agent_run_states_agent_idx ON agent_run_states(agent_id, updated_at);
            CREATE INDEX IF NOT EXISTS agent_run_states_status_idx ON agent_run_states(status, updated_at);
            CREATE INDEX IF NOT EXISTS agent_run_states_approval_idx ON agent_run_states(approval_id);
            CREATE TABLE IF NOT EXISTS leases (
                lease_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                ref_name TEXT NOT NULL,
                path TEXT,
                file_id TEXT,
                mode TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS merge_queue (
                queue_id TEXT PRIMARY KEY,
                source_ref TEXT NOT NULL,
                target_ref TEXT NOT NULL,
                status TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS merge_results (
                merge_id TEXT PRIMARY KEY,
                queue_id TEXT,
                source_ref TEXT NOT NULL,
                target_ref TEXT NOT NULL,
                base_change TEXT NOT NULL,
                left_change TEXT NOT NULL,
                right_change TEXT NOT NULL,
                result_change TEXT,
                status TEXT NOT NULL,
                conflict_set TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS conflict_sets (
                conflict_set_id TEXT PRIMARY KEY,
                merge_id TEXT,
                source_ref TEXT,
                target_ref TEXT,
                status TEXT NOT NULL,
                details_json TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS git_mappings (
                mapping_id TEXT PRIMARY KEY,
                direction TEXT NOT NULL,
                branch TEXT NOT NULL,
                git_head TEXT,
                git_dirty INTEGER NOT NULL,
                crab_change TEXT NOT NULL,
                crab_root TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS git_mappings_change_idx ON git_mappings(crab_change);
            CREATE INDEX IF NOT EXISTS git_mappings_head_idx ON git_mappings(git_head);
            ",
        )?;
        ensure_column(&self.conn, "conflict_sets", "details_json", "TEXT")?;
        ensure_column(&self.conn, "agent_events", "session_id", "TEXT")?;
        self.record_schema_version()?;
        Ok(())
    }

    pub(crate) fn schema_user_version(&self) -> Result<i64> {
        self.conn
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(Error::from)
    }

    pub(crate) fn set_schema_user_version(&self, version: i64) -> Result<()> {
        self.conn
            .execute_batch(&format!("PRAGMA user_version = {version};"))?;
        Ok(())
    }

    pub(crate) fn record_schema_version(&self) -> Result<()> {
        self.set_schema_user_version(CRABDB_SCHEMA_VERSION)?;
        let now = now_ts();
        for (key, value) in [
            (SCHEMA_META_VERSION_KEY, CRABDB_SCHEMA_VERSION.to_string()),
            (
                SCHEMA_META_APP_VERSION_KEY,
                env!("CARGO_PKG_VERSION").to_string(),
            ),
        ] {
            self.conn.execute(
                "INSERT OR REPLACE INTO schema_meta (key, value, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, now],
            )?;
        }
        Ok(())
    }

    pub(crate) fn schema_meta_value(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn allocate_change_id(&self, actor_id: &str, hint: &str) -> Result<ChangeId> {
        let lamport = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(generation), 0) + 1 FROM refs",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1);
        Ok(ChangeId::allocate(
            &self.config.workspace.id,
            actor_id,
            lamport,
            hint,
        ))
    }
}
