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
            CREATE INDEX IF NOT EXISTS agent_turns_session_started_idx ON agent_turns(session_id, started_at);
            CREATE INDEX IF NOT EXISTS agent_turns_agent_started_idx ON agent_turns(agent_id, started_at);
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
            CREATE INDEX IF NOT EXISTS agent_events_agent_created_idx ON agent_events(agent_id, created_at);
            CREATE INDEX IF NOT EXISTS agent_events_session_created_idx ON agent_events(session_id, created_at);
            CREATE INDEX IF NOT EXISTS agent_events_turn_created_idx ON agent_events(turn_id, created_at);
            CREATE INDEX IF NOT EXISTS agent_events_type_created_idx ON agent_events(event_type, created_at);
            CREATE INDEX IF NOT EXISTS agent_events_agent_type_created_idx ON agent_events(agent_id, event_type, created_at);
            CREATE INDEX IF NOT EXISTS agent_events_session_type_created_idx ON agent_events(session_id, event_type, created_at);
            CREATE INDEX IF NOT EXISTS agent_events_turn_type_created_idx ON agent_events(turn_id, event_type, created_at);
            CREATE TABLE IF NOT EXISTS agent_trace_span_events (
                span_id TEXT NOT NULL,
                event_id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                trace_id TEXT,
                agent_id TEXT NOT NULL,
                session_id TEXT,
                turn_id TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS agent_trace_span_events_span_created_idx ON agent_trace_span_events(span_id, created_at);
            CREATE INDEX IF NOT EXISTS agent_trace_span_events_trace_created_idx ON agent_trace_span_events(trace_id, created_at);
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
            ",
        )?;
        ensure_column(&self.conn, "conflict_sets", "details_json", "TEXT")?;
        ensure_column(&self.conn, "agent_events", "session_id", "TEXT")?;
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
