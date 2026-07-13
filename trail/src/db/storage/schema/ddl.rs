use super::*;

impl Trail {
    pub(crate) fn init_schema(&self) -> Result<()> {
        let user_version = self.schema_user_version()?;
        if user_version > TRAIL_SCHEMA_VERSION {
            return Err(Error::InvalidInput(format!(
                "Trail schema version {user_version} is newer than supported version {TRAIL_SCHEMA_VERSION}; upgrade this binary before opening the workspace"
            )));
        }
        if user_version == TRAIL_SCHEMA_VERSION
            && current_environment_schema_complete(&self.conn)?
            && super::agent_capture::agent_capture_schema_complete(&self.conn)?
        {
            return Ok(());
        }
        self.conn
            .execute_batch("SAVEPOINT trail_schema_migration")?;
        let migration = (|| -> Result<()> {
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
                trail_session_id TEXT NOT NULL,
                cwd TEXT NOT NULL,
                path_mappings_json TEXT NOT NULL DEFAULT '[]',
                provider TEXT,
                model TEXT,
                upstream_command_json TEXT,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS lane_acp_sessions_lane_idx ON lane_acp_sessions(lane_id, updated_at);
            CREATE INDEX IF NOT EXISTS lane_acp_sessions_trail_session_idx ON lane_acp_sessions(trail_session_id);
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
            CREATE TABLE IF NOT EXISTS lane_merge_queue (
                queue_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
                target_ref TEXT NOT NULL,
                status TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS lane_merge_queue_active_idx
                ON lane_merge_queue(lane_id, target_ref, status);
            CREATE INDEX IF NOT EXISTS lane_merge_queue_run_idx
                ON lane_merge_queue(status, priority DESC, created_at ASC);
            CREATE TABLE IF NOT EXISTS merge_results (
                merge_id TEXT PRIMARY KEY,
                lane_queue_id TEXT,
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
            CREATE TABLE IF NOT EXISTS workspace_views (
                view_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL UNIQUE,
                base_change TEXT NOT NULL,
                base_root TEXT NOT NULL,
                backend TEXT NOT NULL,
                mountpoint TEXT NOT NULL,
                source_upper TEXT NOT NULL,
                generated_upper TEXT NOT NULL,
                scratch_upper TEXT NOT NULL,
                meta_dir TEXT NOT NULL,
                journal_path TEXT NOT NULL,
                generation INTEGER NOT NULL,
                checkpoint_seq INTEGER NOT NULL,
                checkpoint_root TEXT,
                status TEXT NOT NULL,
                owner_pid INTEGER,
                owner_start_token TEXT,
                heartbeat_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS workspace_views_status_idx ON workspace_views(status, updated_at);
            CREATE TABLE IF NOT EXISTS workspace_layers (
                layer_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                cache_key TEXT NOT NULL UNIQUE,
                adapter TEXT NOT NULL,
                adapter_version INTEGER NOT NULL,
                manifest_object_id TEXT,
                storage_path TEXT NOT NULL,
                state TEXT NOT NULL,
                logical_bytes INTEGER NOT NULL,
                physical_bytes INTEGER,
                entry_count INTEGER NOT NULL,
                portability_scope TEXT NOT NULL,
                builder_id TEXT,
                lease_expires_at INTEGER,
                last_used_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS workspace_layers_state_used_idx ON workspace_layers(state, last_used_at);
            CREATE TABLE IF NOT EXISTS workspace_view_layers (
                view_id TEXT NOT NULL,
                layer_id TEXT NOT NULL,
                mount_path TEXT NOT NULL,
                priority INTEGER NOT NULL,
                read_only INTEGER NOT NULL,
                source_path TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (view_id, mount_path, priority)
            );
            CREATE INDEX IF NOT EXISTS workspace_view_layers_layer_idx ON workspace_view_layers(layer_id);
            CREATE TABLE IF NOT EXISTS workspace_environment_states (
                view_id TEXT NOT NULL,
                adapter TEXT NOT NULL,
                expected_key TEXT NOT NULL,
                attached_key TEXT,
                status TEXT NOT NULL,
                reason TEXT,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, adapter)
            );
            CREATE TABLE IF NOT EXISTS environment_component_states (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                adapter_identity TEXT NOT NULL,
                adapter_version INTEGER NOT NULL,
                implementation_version TEXT NOT NULL,
                distribution_digest TEXT,
                kind TEXT NOT NULL,
                expected_key TEXT NOT NULL,
                attached_key TEXT,
                status TEXT NOT NULL,
                reason TEXT,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id)
            );
            CREATE INDEX IF NOT EXISTS environment_component_states_adapter_idx
                ON environment_component_states(adapter_identity, status, updated_at);
            CREATE TABLE IF NOT EXISTS environment_component_key_provenance (
                component_key TEXT PRIMARY KEY,
                canonical_key_json BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS environment_component_bindings (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                mount_path TEXT NOT NULL,
                kind TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id),
                UNIQUE (view_id, mount_path)
            );
            CREATE TABLE IF NOT EXISTS environment_component_output_bindings (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                output_name TEXT NOT NULL,
                mount_path TEXT NOT NULL,
                layer_subpath TEXT NOT NULL DEFAULT '',
                policy TEXT NOT NULL DEFAULT 'immutable_seed_private',
                binding_identity TEXT NOT NULL DEFAULT '',
                kind TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id, output_name),
                UNIQUE (view_id, mount_path)
            );
            CREATE TABLE IF NOT EXISTS environment_component_dependencies (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                dependency_component_id TEXT NOT NULL,
                dependency_component_key TEXT NOT NULL DEFAULT '',
                edge_type TEXT NOT NULL DEFAULT 'build_requires',
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id, dependency_component_id)
            );
            CREATE INDEX IF NOT EXISTS environment_component_dependencies_dependency_idx
                ON environment_component_dependencies(view_id, dependency_component_id, component_id);
            CREATE TABLE IF NOT EXISTS environment_cache_namespaces (
                namespace_id TEXT PRIMARY KEY,
                adapter_identity TEXT NOT NULL,
                cache_name TEXT NOT NULL,
                protocol TEXT NOT NULL,
                access TEXT NOT NULL,
                authority TEXT NOT NULL DEFAULT 'performance_only',
                scope TEXT NOT NULL DEFAULT 'workspace',
                compatibility_json BLOB NOT NULL,
                storage_path TEXT NOT NULL,
                last_used_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS environment_cache_namespaces_lru_idx
                ON environment_cache_namespaces(last_used_at, namespace_id);
            CREATE TABLE IF NOT EXISTS environment_component_caches (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                cache_name TEXT NOT NULL,
                namespace_id TEXT NOT NULL,
                protocol TEXT NOT NULL,
                access TEXT NOT NULL,
                compatibility_json BLOB NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id, cache_name)
            );
            CREATE TABLE IF NOT EXISTS environment_component_external_artifacts (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                artifact_name TEXT NOT NULL,
                artifact_type TEXT NOT NULL,
                provider TEXT NOT NULL,
                reference TEXT NOT NULL,
                digest TEXT NOT NULL,
                platform TEXT NOT NULL,
                cleanup_owner TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id, artifact_name)
            );
            CREATE TABLE IF NOT EXISTS environment_component_runtime_resources (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                resource_name TEXT NOT NULL,
                runtime_type TEXT NOT NULL,
                provider TEXT NOT NULL,
                artifact_name TEXT NOT NULL,
                container_port INTEGER NOT NULL,
                protocol TEXT NOT NULL,
                health_type TEXT NOT NULL,
                health_timeout_ms INTEGER NOT NULL,
                restart_policy TEXT NOT NULL,
                cleanup_owner TEXT NOT NULL,
                volume_target TEXT,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id, resource_name)
            );
            CREATE TABLE IF NOT EXISTS environment_component_runtime_secrets (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                resource_name TEXT NOT NULL,
                secret_name TEXT NOT NULL,
                provider TEXT NOT NULL,
                reference TEXT NOT NULL,
                version TEXT,
                purpose TEXT NOT NULL,
                injection TEXT NOT NULL,
                target TEXT NOT NULL,
                environment TEXT,
                required INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id, resource_name, secret_name),
                UNIQUE (view_id, component_id, resource_name, target)
            );
            CREATE TABLE IF NOT EXISTS environment_generations (
                generation_id TEXT PRIMARY KEY,
                view_id TEXT NOT NULL,
                generation_sequence INTEGER NOT NULL,
                source_root TEXT NOT NULL,
                specification_digest TEXT NOT NULL,
                predecessor_generation_id TEXT,
                state TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                activated_at INTEGER,
                retired_at INTEGER,
                UNIQUE (view_id, generation_sequence)
            );
            CREATE INDEX IF NOT EXISTS environment_generations_view_state_idx
                ON environment_generations(view_id, state, generation_sequence);
            CREATE TABLE IF NOT EXISTS environment_generation_components (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                adapter_identity TEXT NOT NULL,
                kind TEXT NOT NULL,
                component_key TEXT NOT NULL,
                layer_id TEXT,
                mount_path TEXT,
                PRIMARY KEY (generation_id, component_id)
            );
            CREATE INDEX IF NOT EXISTS environment_generation_components_layer_idx
                ON environment_generation_components(layer_id);
            CREATE TABLE IF NOT EXISTS environment_generation_outputs (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                output_name TEXT NOT NULL,
                policy TEXT NOT NULL DEFAULT 'immutable_seed_private',
                storage_identity TEXT NOT NULL,
                layer_id TEXT,
                mount_path TEXT NOT NULL,
                layer_subpath TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (generation_id, component_id, output_name),
                UNIQUE (generation_id, mount_path)
            );
            CREATE INDEX IF NOT EXISTS environment_generation_outputs_layer_idx
                ON environment_generation_outputs(layer_id);
            CREATE TABLE IF NOT EXISTS environment_generation_edges (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                dependency_component_id TEXT NOT NULL,
                dependency_component_key TEXT NOT NULL,
                edge_type TEXT NOT NULL DEFAULT 'build_requires',
                PRIMARY KEY (generation_id, component_id, dependency_component_id)
            );
            CREATE INDEX IF NOT EXISTS environment_generation_edges_dependency_idx
                ON environment_generation_edges(generation_id, dependency_component_id, component_id);
            CREATE TABLE IF NOT EXISTS environment_generation_caches (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                cache_name TEXT NOT NULL,
                namespace_id TEXT NOT NULL,
                protocol TEXT NOT NULL,
                access TEXT NOT NULL,
                compatibility_json BLOB NOT NULL,
                PRIMARY KEY (generation_id, component_id, cache_name)
            );
            CREATE INDEX IF NOT EXISTS environment_generation_caches_namespace_idx
                ON environment_generation_caches(namespace_id, generation_id);
            CREATE TABLE IF NOT EXISTS environment_generation_external_artifacts (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                artifact_name TEXT NOT NULL,
                artifact_type TEXT NOT NULL,
                provider TEXT NOT NULL,
                reference TEXT NOT NULL,
                digest TEXT NOT NULL,
                platform TEXT NOT NULL,
                cleanup_owner TEXT NOT NULL,
                PRIMARY KEY (generation_id, component_id, artifact_name)
            );
            CREATE INDEX IF NOT EXISTS environment_generation_external_artifacts_digest_idx
                ON environment_generation_external_artifacts(provider, digest, generation_id);
            CREATE TABLE IF NOT EXISTS environment_generation_runtime_resources (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                resource_name TEXT NOT NULL,
                runtime_type TEXT NOT NULL,
                provider TEXT NOT NULL,
                artifact_name TEXT NOT NULL,
                image_reference TEXT NOT NULL,
                image_digest TEXT NOT NULL,
                image_platform TEXT NOT NULL,
                container_port INTEGER NOT NULL,
                protocol TEXT NOT NULL,
                health_type TEXT NOT NULL,
                health_timeout_ms INTEGER NOT NULL,
                restart_policy TEXT NOT NULL,
                cleanup_owner TEXT NOT NULL,
                volume_target TEXT,
                allocation_id TEXT NOT NULL,
                provider_resource_id TEXT,
                container_name TEXT NOT NULL,
                network_name TEXT NOT NULL,
                volume_name TEXT,
                host_port INTEGER,
                status TEXT NOT NULL,
                health_status TEXT NOT NULL,
                reason TEXT,
                cleanup_token TEXT NOT NULL,
                owner_pid INTEGER,
                owner_start_token TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                started_at INTEGER,
                stopped_at INTEGER,
                PRIMARY KEY (generation_id, component_id, resource_name),
                UNIQUE (allocation_id),
                UNIQUE (container_name)
            );
            CREATE INDEX IF NOT EXISTS environment_generation_runtime_resources_status_idx
                ON environment_generation_runtime_resources(status, updated_at, generation_id);
            CREATE TABLE IF NOT EXISTS environment_generation_runtime_secrets (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                resource_name TEXT NOT NULL,
                secret_name TEXT NOT NULL,
                provider TEXT NOT NULL,
                reference TEXT NOT NULL,
                version TEXT,
                purpose TEXT NOT NULL,
                injection TEXT NOT NULL,
                target TEXT NOT NULL,
                environment TEXT,
                required INTEGER NOT NULL,
                status TEXT NOT NULL,
                reason TEXT,
                resolved_at INTEGER,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (generation_id, component_id, resource_name, secret_name),
                UNIQUE (generation_id, component_id, resource_name, target)
            );
            CREATE INDEX IF NOT EXISTS environment_generation_runtime_secrets_status_idx
                ON environment_generation_runtime_secrets(status, updated_at, generation_id);
            CREATE TABLE IF NOT EXISTS environment_secret_access_audit (
                access_id TEXT PRIMARY KEY,
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                resource_name TEXT NOT NULL,
                secret_name TEXT NOT NULL,
                provider TEXT NOT NULL,
                purpose TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS environment_secret_access_audit_generation_idx
                ON environment_secret_access_audit(generation_id, created_at);
            CREATE TABLE IF NOT EXISTS environment_view_generations (
                view_id TEXT PRIMARY KEY,
                generation_id TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS environment_sync_attempts (
                attempt_id TEXT PRIMARY KEY,
                view_id TEXT NOT NULL,
                source_root TEXT NOT NULL,
                mode TEXT NOT NULL,
                owner_pid INTEGER NOT NULL,
                owner_start_token TEXT NOT NULL,
                status TEXT NOT NULL,
                reason TEXT,
                started_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                finished_at INTEGER
            );
            CREATE UNIQUE INDEX IF NOT EXISTS environment_sync_attempts_running_view_idx
                ON environment_sync_attempts(view_id) WHERE status = 'running';
            CREATE INDEX IF NOT EXISTS environment_sync_attempts_status_idx
                ON environment_sync_attempts(status, updated_at);
            CREATE TABLE IF NOT EXISTS workspace_git_shadows (
                view_id TEXT PRIMARY KEY,
                git_dir TEXT NOT NULL,
                policy TEXT NOT NULL,
                pinned_head TEXT NOT NULL,
                current_head TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
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
            if user_version < 16 {
                migrate_lane_merge_queue_v16(&self.conn)?;
            }
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
                "lane_acp_sessions",
                "path_mappings_json",
                "TEXT NOT NULL DEFAULT '[]'",
            )?;
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
            ensure_column(
                &self.conn,
                "workspace_view_layers",
                "source_path",
                "TEXT NOT NULL DEFAULT ''",
            )?;
            ensure_column(
                &self.conn,
                "environment_component_output_bindings",
                "policy",
                "TEXT NOT NULL DEFAULT 'immutable_seed_private'",
            )?;
            ensure_column(
                &self.conn,
                "environment_component_output_bindings",
                "binding_identity",
                "TEXT NOT NULL DEFAULT ''",
            )?;
            ensure_environment_generation_outputs_v7(&self.conn)?;
            ensure_column(
                &self.conn,
                "environment_component_dependencies",
                "dependency_component_key",
                "TEXT NOT NULL DEFAULT ''",
            )?;
            ensure_column(
                &self.conn,
                "environment_component_dependencies",
                "edge_type",
                "TEXT NOT NULL DEFAULT 'build_requires'",
            )?;
            ensure_column(
                &self.conn,
                "environment_generation_edges",
                "edge_type",
                "TEXT NOT NULL DEFAULT 'build_requires'",
            )?;
            ensure_column(
                &self.conn,
                "environment_component_runtime_secrets",
                "environment",
                "TEXT",
            )?;
            ensure_column(
                &self.conn,
                "environment_generation_runtime_secrets",
                "environment",
                "TEXT",
            )?;
            self.ensure_agent_capture_schema()?;
            self.conn.execute(
                "UPDATE environment_component_dependencies
                 SET dependency_component_key = COALESCE(
                     (SELECT dependency.attached_key
                      FROM environment_component_states dependency
                      WHERE dependency.view_id = environment_component_dependencies.view_id
                        AND dependency.component_id = environment_component_dependencies.dependency_component_id),
                     '')
                 WHERE dependency_component_key = ''",
                [],
            )?;
            self.conn.execute(
            "INSERT OR IGNORE INTO environment_component_states
             (view_id, component_id, adapter_identity, adapter_version, implementation_version, distribution_digest, kind, expected_key, attached_key, status, reason, updated_at)
             SELECT view_id,
                    adapter,
                    CASE WHEN adapter = 'node' OR adapter LIKE 'node:%' THEN 'trail/node@1' ELSE 'legacy/' || adapter END,
                    CASE WHEN adapter = 'node' OR adapter LIKE 'node:%' THEN 1 ELSE 0 END,
                    'legacy',
                    NULL,
                    CASE WHEN adapter = 'node' OR adapter LIKE 'node:%' THEN 'dependency' ELSE 'legacy' END,
                    expected_key,
                    attached_key,
                    status,
                    reason,
                    updated_at
             FROM workspace_environment_states",
            [],
        )?;
            self.conn.execute(
            "INSERT OR IGNORE INTO environment_component_bindings
             (view_id, component_id, mount_path, kind, updated_at)
             SELECT view_id,
                    adapter,
                    CASE WHEN adapter = 'node' THEN 'node_modules' ELSE substr(adapter, 6) || '/node_modules' END,
                    'dependency',
                    updated_at
             FROM workspace_environment_states
             WHERE adapter = 'node' OR adapter LIKE 'node:%'",
            [],
        )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO environment_component_output_bindings
                 (view_id, component_id, output_name, mount_path, layer_subpath, policy, binding_identity, kind, updated_at)
                 SELECT b.view_id, b.component_id, 'primary', b.mount_path, '',
                        'immutable_seed_private', COALESCE(l.layer_id, 'legacy-unbound:' || b.component_id),
                        b.kind, b.updated_at
                 FROM environment_component_bindings b
                 LEFT JOIN workspace_view_layers l
                   ON l.view_id = b.view_id AND l.mount_path = b.mount_path",
                [],
            )?;
            self.conn.execute(
                "UPDATE environment_component_output_bindings
                 SET binding_identity = COALESCE(
                     (SELECT l.layer_id FROM workspace_view_layers l
                      WHERE l.view_id = environment_component_output_bindings.view_id
                        AND l.mount_path = environment_component_output_bindings.mount_path),
                     'legacy-unbound:' || component_id || ':' || output_name)
                 WHERE binding_identity = ''",
                [],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO environment_generations
                 (generation_id, view_id, generation_sequence, source_root, specification_digest, predecessor_generation_id, state, created_at, activated_at, retired_at)
                 SELECT 'envgen_legacy_' || e.view_id,
                        e.view_id,
                        1,
                        COALESCE(v.base_root, 'unknown'),
                        'legacy-projection',
                        NULL,
                        'active',
                        MIN(e.updated_at),
                        MIN(e.updated_at),
                        NULL
                 FROM workspace_environment_states e
                 LEFT JOIN workspace_views v ON v.view_id = e.view_id
                 GROUP BY e.view_id",
                [],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO environment_generation_components
                 (generation_id, component_id, adapter_identity, kind, component_key, layer_id, mount_path)
                 SELECT 'envgen_legacy_' || s.view_id,
                        s.component_id,
                        s.adapter_identity,
                        s.kind,
                        COALESCE(s.attached_key, s.expected_key),
                        l.layer_id,
                        b.mount_path
                 FROM environment_component_states s
                 LEFT JOIN environment_component_bindings b
                   ON b.view_id = s.view_id AND b.component_id = s.component_id
                 LEFT JOIN workspace_view_layers l
                   ON l.view_id = b.view_id AND l.mount_path = b.mount_path",
                [],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO environment_generation_outputs
                 (generation_id, component_id, output_name, policy, storage_identity, layer_id, mount_path, layer_subpath)
                 SELECT generation_id, component_id, 'primary', 'immutable_seed_private', layer_id,
                        layer_id, mount_path, ''
                 FROM environment_generation_components
                 WHERE layer_id IS NOT NULL AND mount_path IS NOT NULL",
                [],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO environment_view_generations (view_id, generation_id, updated_at)
                 SELECT view_id, generation_id, COALESCE(activated_at, created_at)
                 FROM environment_generations
                 WHERE state = 'active'",
                [],
            )?;
            self.record_schema_version()?;
            Ok(())
        })();
        match migration {
            Ok(()) => {
                self.conn
                    .execute_batch("RELEASE SAVEPOINT trail_schema_migration")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_schema_migration; RELEASE SAVEPOINT trail_schema_migration",
                );
                Err(err)
            }
        }
    }
}

fn migrate_lane_merge_queue_v16(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS merge_queue;")?;
    let mut stmt = conn.prepare("PRAGMA table_info(merge_results)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    drop(stmt);
    if columns.contains("queue_id") && !columns.contains("lane_queue_id") {
        conn.execute_batch(
            "ALTER TABLE merge_results RENAME COLUMN queue_id TO lane_queue_id;
             UPDATE merge_results SET lane_queue_id = NULL WHERE lane_queue_id IS NOT NULL;",
        )?;
    }
    Ok(())
}

fn ensure_environment_generation_outputs_v7(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(environment_generation_outputs)")?;
    let columns = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, bool>(3)?))
        })?
        .collect::<std::result::Result<BTreeMap<_, _>, _>>()?;
    let already_v7 = columns.contains_key("policy")
        && columns.contains_key("storage_identity")
        && columns.get("layer_id") == Some(&false);
    if already_v7 {
        return Ok(());
    }
    if !columns.contains_key("layer_id") {
        return Err(Error::Corrupt(
            "environment_generation_outputs is missing layer_id during schema migration"
                .to_string(),
        ));
    }
    conn.execute_batch(
        "DROP TABLE IF EXISTS environment_generation_outputs_v7;
         CREATE TABLE environment_generation_outputs_v7 (
             generation_id TEXT NOT NULL,
             component_id TEXT NOT NULL,
             output_name TEXT NOT NULL,
             policy TEXT NOT NULL DEFAULT 'immutable_seed_private',
             storage_identity TEXT NOT NULL,
             layer_id TEXT,
             mount_path TEXT NOT NULL,
             layer_subpath TEXT NOT NULL DEFAULT '',
             PRIMARY KEY (generation_id, component_id, output_name),
             UNIQUE (generation_id, mount_path)
         );
         INSERT INTO environment_generation_outputs_v7
             (generation_id, component_id, output_name, policy, storage_identity, layer_id, mount_path, layer_subpath)
         SELECT generation_id, component_id, output_name, 'immutable_seed_private', layer_id,
                layer_id, mount_path, layer_subpath
         FROM environment_generation_outputs;
         DROP TABLE environment_generation_outputs;
         ALTER TABLE environment_generation_outputs_v7 RENAME TO environment_generation_outputs;
         CREATE INDEX environment_generation_outputs_layer_idx
             ON environment_generation_outputs(layer_id);",
    )?;
    Ok(())
}

fn current_environment_schema_complete(conn: &Connection) -> Result<bool> {
    let required_tables = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('schema_meta', 'environment_component_states', 'environment_component_key_provenance', 'environment_component_bindings', 'environment_component_output_bindings', 'environment_component_dependencies', 'environment_cache_namespaces', 'environment_component_caches', 'environment_component_external_artifacts', 'environment_component_runtime_resources', 'environment_component_runtime_secrets', 'environment_generations', 'environment_generation_components', 'environment_generation_outputs', 'environment_generation_edges', 'environment_generation_caches', 'environment_generation_external_artifacts', 'environment_generation_runtime_resources', 'environment_generation_runtime_secrets', 'environment_secret_access_audit', 'environment_view_generations', 'environment_sync_attempts')",
            [],
            |row| row.get::<_, i64>(0),
        )? == 22;
    if !required_tables {
        return Ok(false);
    }
    let meta_version = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            params![SCHEMA_META_VERSION_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let expected_version = TRAIL_SCHEMA_VERSION.to_string();
    if meta_version.as_deref() != Some(expected_version.as_str()) {
        return Ok(false);
    }
    let mut stmt = conn.prepare("PRAGMA table_info(environment_component_states)")?;
    let state_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(workspace_view_layers)")?;
    let layer_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_component_output_bindings)")?;
    let output_binding_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_generation_outputs)")?;
    let generation_output_columns = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, bool>(3)?))
        })?
        .collect::<std::result::Result<BTreeMap<_, _>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_component_dependencies)")?;
    let dependency_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_generation_edges)")?;
    let generation_edge_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_component_external_artifacts)")?;
    let external_artifact_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_generation_external_artifacts)")?;
    let generation_external_artifact_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_component_runtime_resources)")?;
    let runtime_resource_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_generation_runtime_resources)")?;
    let generation_runtime_resource_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_component_runtime_secrets)")?;
    let runtime_secret_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut stmt = conn.prepare("PRAGMA table_info(environment_generation_runtime_secrets)")?;
    let generation_runtime_secret_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    Ok(state_columns.contains("implementation_version")
        && state_columns.contains("distribution_digest")
        && layer_columns.contains("source_path")
        && output_binding_columns.contains("policy")
        && output_binding_columns.contains("binding_identity")
        && generation_output_columns.contains_key("policy")
        && generation_output_columns.contains_key("storage_identity")
        && generation_output_columns.get("layer_id") == Some(&false)
        && dependency_columns.contains("dependency_component_key")
        && dependency_columns.contains("edge_type")
        && generation_edge_columns.contains("edge_type")
        && external_artifact_columns.contains("reference")
        && external_artifact_columns.contains("digest")
        && external_artifact_columns.contains("cleanup_owner")
        && generation_external_artifact_columns.contains("reference")
        && generation_external_artifact_columns.contains("digest")
        && generation_external_artifact_columns.contains("cleanup_owner")
        && runtime_resource_columns.contains("artifact_name")
        && runtime_resource_columns.contains("health_timeout_ms")
        && runtime_resource_columns.contains("volume_target")
        && generation_runtime_resource_columns.contains("allocation_id")
        && generation_runtime_resource_columns.contains("host_port")
        && generation_runtime_resource_columns.contains("cleanup_token")
        && generation_runtime_resource_columns.contains("health_status")
        && runtime_secret_columns.contains("reference")
        && runtime_secret_columns.contains("injection")
        && runtime_secret_columns.contains("required")
        && runtime_secret_columns.contains("environment")
        && generation_runtime_secret_columns.contains("status")
        && generation_runtime_secret_columns.contains("environment")
        && generation_runtime_secret_columns.contains("resolved_at"))
}
