use super::*;

impl CrabDb {
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
                lane_id TEXT,
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
            CREATE TABLE IF NOT EXISTS lanes (
                lane_id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                kind TEXT,
                provider TEXT,
                model TEXT,
                created_at INTEGER NOT NULL,
                metadata_json TEXT
            );
            CREATE TABLE IF NOT EXISTS lane_branches (
                lane_id TEXT PRIMARY KEY,
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
            CREATE TABLE IF NOT EXISTS lane_sessions (
                session_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
                title TEXT,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                metadata_json TEXT
            );
            CREATE TABLE IF NOT EXISTS lane_acp_sessions (
                acp_session_id TEXT PRIMARY KEY,
                upstream_session_id TEXT,
                lane_id TEXT NOT NULL,
                crabdb_session_id TEXT NOT NULL,
                cwd TEXT NOT NULL,
                provider TEXT,
                model TEXT,
                upstream_command_json TEXT,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS lane_acp_sessions_lane_idx ON lane_acp_sessions(lane_id, updated_at);
            CREATE INDEX IF NOT EXISTS lane_acp_sessions_crabdb_session_idx ON lane_acp_sessions(crabdb_session_id);
            CREATE TABLE IF NOT EXISTS lane_turns (
                turn_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
                session_id TEXT,
                base_change TEXT NOT NULL,
                before_change TEXT NOT NULL,
                after_change TEXT,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                metadata_json TEXT
            );
            CREATE INDEX IF NOT EXISTS lane_turns_session_started_idx ON lane_turns(session_id, started_at);
            CREATE INDEX IF NOT EXISTS lane_turns_lane_started_idx ON lane_turns(lane_id, started_at);
            CREATE TABLE IF NOT EXISTS lane_events (
                event_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
                turn_id TEXT,
                session_id TEXT,
                event_type TEXT NOT NULL,
                change_id TEXT,
                message_id TEXT,
                payload_json TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS lane_events_lane_created_idx ON lane_events(lane_id, created_at);
            CREATE INDEX IF NOT EXISTS lane_events_session_created_idx ON lane_events(session_id, created_at);
            CREATE INDEX IF NOT EXISTS lane_events_turn_created_idx ON lane_events(turn_id, created_at);
            CREATE INDEX IF NOT EXISTS lane_events_type_created_idx ON lane_events(event_type, created_at);
            CREATE INDEX IF NOT EXISTS lane_events_lane_type_created_idx ON lane_events(lane_id, event_type, created_at);
            CREATE INDEX IF NOT EXISTS lane_events_session_type_created_idx ON lane_events(session_id, event_type, created_at);
            CREATE INDEX IF NOT EXISTS lane_events_turn_type_created_idx ON lane_events(turn_id, event_type, created_at);
            CREATE TABLE IF NOT EXISTS lane_trace_span_events (
                span_id TEXT NOT NULL,
                event_id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                trace_id TEXT,
                lane_id TEXT NOT NULL,
                session_id TEXT,
                turn_id TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS lane_trace_span_events_span_created_idx ON lane_trace_span_events(span_id, created_at);
            CREATE INDEX IF NOT EXISTS lane_trace_span_events_trace_created_idx ON lane_trace_span_events(trace_id, created_at);
            CREATE TABLE IF NOT EXISTS lane_approvals (
                approval_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
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
            CREATE INDEX IF NOT EXISTS lane_approvals_status_idx ON lane_approvals(status, requested_at);
            CREATE INDEX IF NOT EXISTS lane_approvals_lane_idx ON lane_approvals(lane_id, requested_at);
            CREATE TABLE IF NOT EXISTS lane_run_states (
                run_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
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
            CREATE INDEX IF NOT EXISTS lane_run_states_lane_idx ON lane_run_states(lane_id, updated_at);
            CREATE INDEX IF NOT EXISTS lane_run_states_status_idx ON lane_run_states(status, updated_at);
            CREATE INDEX IF NOT EXISTS lane_run_states_approval_idx ON lane_run_states(approval_id);
            CREATE TABLE IF NOT EXISTS leases (
                lease_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
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
                base_root TEXT,
                left_root TEXT,
                right_root TEXT,
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
            CREATE TABLE IF NOT EXISTS conflict_resolution_suggestions (
                suggestion_id TEXT PRIMARY KEY,
                signature TEXT NOT NULL,
                path TEXT NOT NULL,
                conflict_class TEXT NOT NULL,
                resolution TEXT NOT NULL,
                conflict_set_id TEXT NOT NULL,
                operation TEXT NOT NULL,
                source_ref TEXT,
                target_ref TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS conflict_resolution_suggestions_signature_idx ON conflict_resolution_suggestions(signature, created_at);
            CREATE TABLE IF NOT EXISTS external_mutation_audit (
                audit_id TEXT PRIMARY KEY,
                actor TEXT NOT NULL DEFAULT 'unknown',
                surface TEXT NOT NULL,
                command TEXT NOT NULL,
                target_ref TEXT,
                lane_id TEXT,
                status TEXT NOT NULL,
                status_code INTEGER,
                change_id TEXT,
                summary_json TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS external_mutation_audit_created_idx ON external_mutation_audit(created_at);
            CREATE INDEX IF NOT EXISTS external_mutation_audit_surface_created_idx ON external_mutation_audit(surface, created_at);
            CREATE INDEX IF NOT EXISTS external_mutation_audit_lane_created_idx ON external_mutation_audit(lane_id, created_at);
            CREATE TABLE IF NOT EXISTS http_idempotency_keys (
                key TEXT PRIMARY KEY,
                method TEXT NOT NULL,
                path TEXT NOT NULL,
                request_hash TEXT NOT NULL,
                status INTEGER NOT NULL,
                body BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS http_idempotency_keys_updated_idx ON http_idempotency_keys(updated_at);
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
            CREATE TABLE IF NOT EXISTS worktree_file_index (
                path TEXT PRIMARY KEY,
                size_bytes INTEGER NOT NULL,
                modified_ns INTEGER NOT NULL,
                changed_ns INTEGER NOT NULL,
                device_id INTEGER NOT NULL DEFAULT 0,
                inode INTEGER NOT NULL DEFAULT 0,
                executable INTEGER NOT NULL,
                kind TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                last_seen_scan INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memory_items (
                memory_ord INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id TEXT NOT NULL UNIQUE,
                scope_type TEXT NOT NULL,
                scope_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                path TEXT,
                title TEXT,
                body TEXT NOT NULL,
                status TEXT NOT NULL,
                source_ref TEXT,
                source_change TEXT,
                source_root TEXT,
                metadata_json TEXT NOT NULL,
                created_by TEXT NOT NULL,
                updated_by TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                archived_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS memory_items_scope_idx ON memory_items(scope_type, scope_id, status, updated_at);
            CREATE INDEX IF NOT EXISTS memory_items_kind_idx ON memory_items(kind, status, updated_at);
            CREATE INDEX IF NOT EXISTS memory_items_path_idx ON memory_items(path, status, updated_at);
            CREATE INDEX IF NOT EXISTS memory_items_source_change_idx ON memory_items(source_change, updated_at);
            CREATE TABLE IF NOT EXISTS memory_embeddings (
                memory_id TEXT PRIMARY KEY REFERENCES memory_items(memory_id) ON DELETE CASCADE,
                memory_ord INTEGER NOT NULL UNIQUE,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                dims INTEGER NOT NULL,
                embedding BLOB NOT NULL,
                embedding_hash TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS memory_embeddings_model_idx ON memory_embeddings(provider, model, dims);
            CREATE TABLE IF NOT EXISTS memory_embedding_indexes (
                index_id TEXT PRIMARY KEY,
                backend TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                dims INTEGER NOT NULL,
                table_name TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(backend, provider, model, dims)
            );
            CREATE TABLE IF NOT EXISTS memory_revisions (
                revision_id TEXT PRIMARY KEY,
                memory_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                operation TEXT NOT NULL,
                scope_type TEXT NOT NULL,
                scope_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                path TEXT,
                title TEXT,
                body TEXT NOT NULL,
                status TEXT NOT NULL,
                source_ref TEXT,
                source_change TEXT,
                source_root TEXT,
                metadata_json TEXT NOT NULL,
                embedding_hash TEXT,
                actor_id TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(memory_id, version)
            );
            CREATE INDEX IF NOT EXISTS memory_revisions_memory_idx ON memory_revisions(memory_id, version);
            CREATE INDEX IF NOT EXISTS memory_revisions_source_change_idx ON memory_revisions(source_change, created_at);
            ",
        )?;
        ensure_column(&self.conn, "conflict_sets", "details_json", "TEXT")?;
        ensure_column(&self.conn, "merge_results", "base_root", "TEXT")?;
        ensure_column(&self.conn, "merge_results", "left_root", "TEXT")?;
        ensure_column(&self.conn, "merge_results", "right_root", "TEXT")?;
        ensure_column(
            &self.conn,
            "external_mutation_audit",
            "actor",
            "TEXT NOT NULL DEFAULT 'unknown'",
        )?;
        ensure_column(&self.conn, "lane_events", "session_id", "TEXT")?;
        ensure_column(
            &self.conn,
            "worktree_file_index",
            "last_seen_scan",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(
            &self.conn,
            "worktree_file_index",
            "device_id",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(
            &self.conn,
            "worktree_file_index",
            "inode",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        self.record_schema_version()?;
        Ok(())
    }
}
