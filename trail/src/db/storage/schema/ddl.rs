use super::*;

pub(super) const BASE_SCHEMA_V18: &str = r#"
CREATE TABLE agent_attestation_key_revocations (
                 key_id TEXT PRIMARY KEY,
                 public_key_hex TEXT NOT NULL,
                 reason TEXT NOT NULL,
                 revoked_at INTEGER NOT NULL,
                 metadata_json TEXT
             );
CREATE TABLE agent_capture_runs (
                 capture_run_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 lane_id TEXT,
                 workdir TEXT NOT NULL,
                 canonical_workdir TEXT NOT NULL,
                 owner_agent TEXT NOT NULL,
                 owner_session_id TEXT NOT NULL,
                 executor_agent TEXT,
                 work_item_id TEXT,
                 status TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 expires_at INTEGER NOT NULL,
                 ended_at INTEGER,
                 metadata_json TEXT
             );
CREATE TABLE agent_hook_installations (
                 installation_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 scope TEXT NOT NULL,
                 config_path TEXT NOT NULL,
                 lane_id TEXT,
                 manifest_digest TEXT NOT NULL,
                 manifest_signature_json TEXT,
                 ownership_inventory_json TEXT NOT NULL,
                 config_before_digest TEXT,
                 config_after_digest TEXT NOT NULL,
                 adapter_version TEXT NOT NULL,
                 provider_version_range TEXT,
                 detected_provider_version TEXT,
                 capability_status TEXT NOT NULL,
                 status TEXT NOT NULL,
                 installed_at INTEGER NOT NULL,
                 verified_at INTEGER,
                 last_receipt_at INTEGER,
                 metadata_json TEXT
             );
CREATE TABLE agent_hook_receipts (
                 receipt_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 installation_id TEXT REFERENCES agent_hook_installations(installation_id),
                 mapping_id TEXT REFERENCES lane_agent_sessions(mapping_id),
                 provider TEXT NOT NULL,
                 native_event TEXT NOT NULL,
                 native_session_id TEXT,
                 native_turn_id TEXT,
                 transport TEXT NOT NULL,
                 dedupe_key TEXT NOT NULL,
                 payload_digest TEXT NOT NULL,
                 raw_object_id TEXT NOT NULL REFERENCES objects(object_id),
                 raw_artifact_id TEXT REFERENCES lane_artifacts(artifact_id),
                 receive_sequence INTEGER,
                 connection_id TEXT,
                 direction TEXT,
                 connection_sequence INTEGER,
                 status TEXT NOT NULL,
                 attempt_count INTEGER NOT NULL DEFAULT 0,
                 next_attempt_at INTEGER,
                 diagnostic TEXT,
                 occurred_at INTEGER,
                 received_at INTEGER NOT NULL,
                 processed_at INTEGER,
                 updated_at INTEGER NOT NULL,
                 UNIQUE(workspace_id, provider, dedupe_key),
                 UNIQUE(mapping_id, receive_sequence)
             );
CREATE TABLE anchors (
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
CREATE TABLE conflict_resolution_suggestions (
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
CREATE TABLE conflict_sets (
                conflict_set_id TEXT PRIMARY KEY,
                merge_id TEXT,
                source_ref TEXT,
                target_ref TEXT,
                status TEXT NOT NULL,
                details_json TEXT,
                created_at INTEGER NOT NULL
            );
CREATE TABLE environment_cache_namespaces (
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
CREATE TABLE environment_component_bindings (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                mount_path TEXT NOT NULL,
                kind TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id),
                UNIQUE (view_id, mount_path)
            );
CREATE TABLE environment_component_caches (
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
CREATE TABLE environment_component_dependencies (
                view_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                dependency_component_id TEXT NOT NULL,
                dependency_component_key TEXT NOT NULL DEFAULT '',
                edge_type TEXT NOT NULL DEFAULT 'build_requires',
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, component_id, dependency_component_id)
            );
CREATE TABLE environment_component_external_artifacts (
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
CREATE TABLE environment_component_key_provenance (
                component_key TEXT PRIMARY KEY,
                canonical_key_json BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );
CREATE TABLE environment_component_output_bindings (
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
CREATE TABLE environment_component_runtime_resources (
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
CREATE TABLE environment_component_runtime_secrets (
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
CREATE TABLE environment_component_states (
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
CREATE TABLE environment_generation_caches (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                cache_name TEXT NOT NULL,
                namespace_id TEXT NOT NULL,
                protocol TEXT NOT NULL,
                access TEXT NOT NULL,
                compatibility_json BLOB NOT NULL,
                PRIMARY KEY (generation_id, component_id, cache_name)
            );
CREATE TABLE environment_generation_components (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                adapter_identity TEXT NOT NULL,
                kind TEXT NOT NULL,
                component_key TEXT NOT NULL,
                layer_id TEXT,
                mount_path TEXT,
                PRIMARY KEY (generation_id, component_id)
            );
CREATE TABLE environment_generation_edges (
                generation_id TEXT NOT NULL,
                component_id TEXT NOT NULL,
                dependency_component_id TEXT NOT NULL,
                dependency_component_key TEXT NOT NULL,
                edge_type TEXT NOT NULL DEFAULT 'build_requires',
                PRIMARY KEY (generation_id, component_id, dependency_component_id)
            );
CREATE TABLE environment_generation_external_artifacts (
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
CREATE TABLE environment_generation_outputs (
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
CREATE TABLE environment_generation_runtime_resources (
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
CREATE TABLE environment_generation_runtime_secrets (
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
CREATE TABLE environment_generations (
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
CREATE TABLE environment_secret_access_audit (
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
CREATE TABLE environment_sync_attempts (
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
CREATE TABLE environment_view_generations (
                view_id TEXT PRIMARY KEY,
                generation_id TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
CREATE TABLE external_mutation_audit (
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
CREATE TABLE file_history (
                file_id TEXT NOT NULL,
                change_id TEXT NOT NULL,
                path TEXT NOT NULL,
                old_path TEXT,
                kind TEXT NOT NULL,
                before_hash TEXT,
                after_hash TEXT,
                created_at INTEGER NOT NULL
            );
CREATE TABLE git_agent_links (
                 git_agent_link_id TEXT PRIMARY KEY,
                 git_commit TEXT NOT NULL,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT REFERENCES lane_turns(turn_id),
                 from_change TEXT,
                 through_change TEXT,
                 confidence TEXT NOT NULL,
                 source TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 metadata_json TEXT
             );
CREATE TABLE git_mappings (
                mapping_id TEXT PRIMARY KEY,
                direction TEXT NOT NULL,
                branch TEXT NOT NULL,
                git_head TEXT,
                git_dirty INTEGER NOT NULL,
                crab_change TEXT NOT NULL,
                crab_root TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
CREATE TABLE http_idempotency_keys (
                key TEXT PRIMARY KEY,
                method TEXT NOT NULL,
                path TEXT NOT NULL,
                request_hash TEXT NOT NULL,
                status INTEGER NOT NULL,
                body BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
CREATE TABLE lane_acp_sessions (
                acp_session_id TEXT PRIMARY KEY,
                upstream_session_id TEXT,
                lane_id TEXT NOT NULL,
                trail_session_id TEXT NOT NULL,
                cwd TEXT NOT NULL,
                provider TEXT,
                model TEXT,
                upstream_command_json TEXT,
                path_mappings_json TEXT NOT NULL DEFAULT '[]',
                current_mode_id TEXT,
                config_options_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
CREATE TABLE lane_agent_session_aliases (
                 workspace_id TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 native_session_alias TEXT NOT NULL,
                 mapping_id TEXT NOT NULL REFERENCES lane_agent_sessions(mapping_id),
                 reason TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 PRIMARY KEY(workspace_id, provider, native_session_alias)
             );
CREATE TABLE lane_agent_sessions (
                 mapping_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 native_session_id TEXT NOT NULL,
                 parent_native_session_id TEXT,
                 trail_session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 capture_run_id TEXT REFERENCES agent_capture_runs(capture_run_id),
                 primary_transport TEXT NOT NULL,
                 transcript_identity TEXT,
                 transcript_offset INTEGER,
                 resume_json TEXT,
                 last_attestation_id TEXT,
                 status TEXT NOT NULL,
                 pending_turn_outcome TEXT,
                 session_close_requested INTEGER NOT NULL DEFAULT 0,
                 capture_epoch INTEGER NOT NULL DEFAULT 1,
                 finalization_owner TEXT,
                 finalization_lease_expires_at INTEGER,
                 next_receive_sequence INTEGER NOT NULL DEFAULT 1,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 UNIQUE(workspace_id, provider, native_session_id)
             );
CREATE TABLE lane_approvals (
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
CREATE TABLE lane_artifacts (
                 artifact_id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT REFERENCES lane_turns(turn_id),
                 provider TEXT NOT NULL,
                 artifact_kind TEXT NOT NULL,
                 format TEXT NOT NULL,
                 source TEXT NOT NULL,
                 source_locator_redacted TEXT,
                 content_object_id TEXT,
                 content_digest TEXT NOT NULL,
                 size_bytes INTEGER NOT NULL,
                 start_offset INTEGER,
                 end_offset INTEGER,
                 redaction_profile TEXT,
                 retention_status TEXT NOT NULL,
                 trust TEXT NOT NULL,
                 supersedes_artifact_id TEXT REFERENCES lane_artifacts(artifact_id),
                 created_at INTEGER NOT NULL,
                 metadata_json TEXT
             );
CREATE TABLE lane_branches (
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
CREATE TABLE lane_events (
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
CREATE TABLE lane_learnings (
                 learning_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT REFERENCES lane_turns(turn_id),
                 scope TEXT NOT NULL,
                 body TEXT NOT NULL,
                 status TEXT NOT NULL,
                 confidence REAL,
                 source_artifact_id TEXT REFERENCES lane_artifacts(artifact_id),
                 anchor_json TEXT,
                 created_at INTEGER NOT NULL,
                 reviewed_at INTEGER,
                 reviewer TEXT,
                 expires_at INTEGER,
                 superseded_by TEXT REFERENCES lane_learnings(learning_id),
                 metadata_json TEXT
             );
CREATE TABLE lane_merge_queue (
                queue_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
                target_ref TEXT NOT NULL,
                status TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
CREATE TABLE lane_provenance_edges (
                 provenance_edge_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 from_node_id TEXT NOT NULL REFERENCES lane_provenance_nodes(provenance_node_id),
                 to_node_id TEXT NOT NULL REFERENCES lane_provenance_nodes(provenance_node_id),
                 relation TEXT NOT NULL,
                 source_confidence TEXT NOT NULL,
                 receipt_id TEXT REFERENCES agent_hook_receipts(receipt_id),
                 created_at INTEGER NOT NULL,
                 attributes_json TEXT
             );
CREATE TABLE lane_provenance_nodes (
                 provenance_node_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT REFERENCES lane_turns(turn_id),
                 node_kind TEXT NOT NULL,
                 summary TEXT NOT NULL,
                 event_id TEXT,
                 span_id TEXT,
                 message_id TEXT,
                 change_id TEXT,
                 artifact_id TEXT REFERENCES lane_artifacts(artifact_id),
                 source_confidence TEXT NOT NULL,
                 classifier_version TEXT,
                 created_at INTEGER NOT NULL,
                 attributes_json TEXT
             );
CREATE TABLE lane_run_states (
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
CREATE TABLE lane_session_attestation_turns (
                 attestation_id TEXT NOT NULL REFERENCES lane_session_attestations(attestation_id),
                 turn_id TEXT NOT NULL REFERENCES lane_turns(turn_id),
                 change_id TEXT,
                 evidence_manifest_id TEXT NOT NULL REFERENCES lane_turn_evidence_manifests(manifest_id),
                 PRIMARY KEY(attestation_id, turn_id)
             );
CREATE TABLE lane_session_attestations (
                 attestation_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 capture_run_id TEXT REFERENCES agent_capture_runs(capture_run_id),
                 previous_attestation_id TEXT REFERENCES lane_session_attestations(attestation_id),
                 statement_object_id TEXT NOT NULL,
                 statement_digest TEXT NOT NULL UNIQUE,
                 signature_json TEXT,
                 status TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 superseded_by TEXT REFERENCES lane_session_attestations(attestation_id),
                 metadata_json TEXT
             );
CREATE TABLE lane_sessions (
                session_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
                title TEXT,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                metadata_json TEXT
            );
CREATE TABLE lane_trace_span_events (
                span_id TEXT NOT NULL,
                event_id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                trace_id TEXT,
                lane_id TEXT NOT NULL,
                session_id TEXT,
                turn_id TEXT,
                created_at INTEGER NOT NULL
            );
CREATE TABLE lane_turn_evidence_manifests (
                 manifest_id TEXT PRIMARY KEY,
                 lane_id TEXT NOT NULL REFERENCES lanes(lane_id),
                 session_id TEXT NOT NULL REFERENCES lane_sessions(session_id),
                 turn_id TEXT NOT NULL UNIQUE REFERENCES lane_turns(turn_id),
                 schema_version INTEGER NOT NULL,
                 object_id TEXT NOT NULL,
                 digest TEXT NOT NULL UNIQUE,
                 created_at INTEGER NOT NULL
             );
CREATE TABLE lane_turns (
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
CREATE TABLE lanes (
                lane_id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                kind TEXT,
                provider TEXT,
                model TEXT,
                created_at INTEGER NOT NULL,
                metadata_json TEXT
            );
CREATE TABLE leases (
                lease_id TEXT PRIMARY KEY,
                lane_id TEXT NOT NULL,
                ref_name TEXT NOT NULL,
                path TEXT,
                file_id TEXT,
                mode TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
CREATE TABLE line_history (
                line_id TEXT NOT NULL,
                file_id TEXT NOT NULL,
                change_id TEXT NOT NULL,
                path TEXT NOT NULL,
                line_number INTEGER,
                kind TEXT NOT NULL,
                text_hash TEXT,
                created_at INTEGER NOT NULL
            );
CREATE TABLE memory_embedding_indexes (
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
CREATE TABLE memory_embeddings (
                memory_id TEXT PRIMARY KEY REFERENCES memory_items(memory_id) ON DELETE CASCADE,
                memory_ord INTEGER NOT NULL UNIQUE,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                dims INTEGER NOT NULL,
                embedding BLOB NOT NULL,
                embedding_hash TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
CREATE TABLE memory_items (
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
CREATE TABLE memory_revisions (
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
CREATE TABLE merge_results (
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
CREATE TABLE messages (
                message_id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                body TEXT NOT NULL,
                lane_id TEXT,
                session_id TEXT,
                change_id TEXT,
                object_id TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
CREATE TABLE objects (
                object_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                version INTEGER NOT NULL,
                codec TEXT NOT NULL,
                hash_alg TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                bytes BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );
CREATE TABLE operation_parents (
                change_id TEXT NOT NULL,
                parent_change_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                PRIMARY KEY (change_id, position)
            );
CREATE TABLE operations (
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
CREATE TABLE pending_path_index_derived_repairs (
                ref_name TEXT NOT NULL,
                repair_kind TEXT NOT NULL CHECK (repair_kind IN ('lane_manifest', 'workspace_checkpoint')),
                old_root TEXT NOT NULL,
                new_root TEXT NOT NULL,
                new_change TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (ref_name, repair_kind)
            );
CREATE TABLE refs (
                name TEXT PRIMARY KEY,
                change_id TEXT NOT NULL,
                root_id TEXT NOT NULL,
                operation_id TEXT NOT NULL,
                generation INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
CREATE TABLE schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
CREATE TABLE workspace_environment_states (
                view_id TEXT NOT NULL,
                adapter TEXT NOT NULL,
                expected_key TEXT NOT NULL,
                attached_key TEXT,
                status TEXT NOT NULL,
                reason TEXT,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (view_id, adapter)
            );
CREATE TABLE workspace_git_shadows (
                view_id TEXT PRIMARY KEY,
                git_dir TEXT NOT NULL,
                policy TEXT NOT NULL,
                pinned_head TEXT NOT NULL,
                current_head TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
CREATE TABLE workspace_layers (
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
CREATE TABLE workspace_view_layers (
                view_id TEXT NOT NULL,
                layer_id TEXT NOT NULL,
                mount_path TEXT NOT NULL,
                priority INTEGER NOT NULL,
                read_only INTEGER NOT NULL,
                source_path TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (view_id, mount_path, priority)
            );
CREATE TABLE workspace_views (
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
CREATE TABLE worktree_file_index (
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
CREATE INDEX agent_attestation_key_revocations_time_idx
                 ON agent_attestation_key_revocations(revoked_at, key_id);
CREATE INDEX agent_capture_runs_active_workdir_idx
                 ON agent_capture_runs(workspace_id, canonical_workdir, expires_at)
                 WHERE status = 'active';
CREATE INDEX agent_capture_runs_owner_idx
                 ON agent_capture_runs(owner_agent, owner_session_id, updated_at);
CREATE INDEX agent_hook_installations_status_idx
                 ON agent_hook_installations(provider, status, verified_at);
CREATE UNIQUE INDEX agent_hook_installations_target_idx
                 ON agent_hook_installations(workspace_id, provider, scope, config_path);
CREATE INDEX agent_hook_receipts_replay_idx
                 ON agent_hook_receipts(status, next_attempt_at, received_at);
CREATE INDEX agent_hook_receipts_session_idx
                 ON agent_hook_receipts(workspace_id, provider, native_session_id, received_at);
CREATE INDEX agent_hook_receipts_turn_idx
                 ON agent_hook_receipts(native_turn_id, received_at);
CREATE UNIQUE INDEX agent_hook_receipts_connection_sequence_idx
                 ON agent_hook_receipts(connection_id, direction, connection_sequence)
                 WHERE connection_id IS NOT NULL
                   AND direction IS NOT NULL
                   AND connection_sequence IS NOT NULL;
CREATE INDEX anchors_file_idx ON anchors(file_id, created_at);
CREATE INDEX anchors_line_idx ON anchors(line_id, created_at);
CREATE INDEX conflict_resolution_suggestions_signature_idx ON conflict_resolution_suggestions(signature, created_at);
CREATE INDEX environment_cache_namespaces_lru_idx
                ON environment_cache_namespaces(last_used_at, namespace_id);
CREATE INDEX environment_component_dependencies_dependency_idx
                ON environment_component_dependencies(view_id, dependency_component_id, component_id);
CREATE INDEX environment_component_states_adapter_idx
                ON environment_component_states(adapter_identity, status, updated_at);
CREATE INDEX environment_generation_caches_namespace_idx
                ON environment_generation_caches(namespace_id, generation_id);
CREATE INDEX environment_generation_components_layer_idx
                ON environment_generation_components(layer_id);
CREATE INDEX environment_generation_edges_dependency_idx
                ON environment_generation_edges(generation_id, dependency_component_id, component_id);
CREATE INDEX environment_generation_external_artifacts_digest_idx
                ON environment_generation_external_artifacts(provider, digest, generation_id);
CREATE INDEX environment_generation_outputs_layer_idx
                ON environment_generation_outputs(layer_id);
CREATE INDEX environment_generation_runtime_resources_status_idx
                ON environment_generation_runtime_resources(status, updated_at, generation_id);
CREATE INDEX environment_generation_runtime_secrets_status_idx
                ON environment_generation_runtime_secrets(status, updated_at, generation_id);
CREATE INDEX environment_generations_view_state_idx
                ON environment_generations(view_id, state, generation_sequence);
CREATE INDEX environment_secret_access_audit_generation_idx
                ON environment_secret_access_audit(generation_id, created_at);
CREATE UNIQUE INDEX environment_sync_attempts_running_view_idx
                ON environment_sync_attempts(view_id) WHERE status = 'running';
CREATE INDEX environment_sync_attempts_status_idx
                ON environment_sync_attempts(status, updated_at);
CREATE INDEX external_mutation_audit_created_idx ON external_mutation_audit(created_at);
CREATE INDEX external_mutation_audit_lane_created_idx ON external_mutation_audit(lane_id, created_at);
CREATE INDEX external_mutation_audit_surface_created_idx ON external_mutation_audit(surface, created_at);
CREATE INDEX file_history_file_idx ON file_history(file_id, created_at);
CREATE INDEX file_history_path_idx ON file_history(path, created_at);
CREATE UNIQUE INDEX git_agent_links_identity_idx
                 ON git_agent_links(git_commit, session_id, COALESCE(turn_id, ''), source);
CREATE INDEX git_agent_links_session_idx
                 ON git_agent_links(session_id, created_at);
CREATE INDEX git_agent_links_turn_idx
                 ON git_agent_links(turn_id, created_at);
CREATE INDEX git_mappings_change_idx ON git_mappings(crab_change);
CREATE INDEX git_mappings_head_idx ON git_mappings(git_head);
CREATE INDEX http_idempotency_keys_updated_idx ON http_idempotency_keys(updated_at);
CREATE INDEX lane_acp_sessions_lane_idx ON lane_acp_sessions(lane_id, updated_at);
CREATE INDEX lane_acp_sessions_trail_session_idx ON lane_acp_sessions(trail_session_id);
CREATE INDEX lane_agent_session_aliases_mapping_idx
                 ON lane_agent_session_aliases(mapping_id);
CREATE INDEX lane_agent_sessions_finalization_idx
                 ON lane_agent_sessions(status, finalization_lease_expires_at)
                 WHERE status = 'finalizing';
CREATE INDEX lane_agent_sessions_lane_idx
                 ON lane_agent_sessions(lane_id, updated_at);
CREATE INDEX lane_agent_sessions_run_idx
                 ON lane_agent_sessions(capture_run_id, updated_at);
CREATE INDEX lane_agent_sessions_trail_session_idx
                 ON lane_agent_sessions(trail_session_id, updated_at);
CREATE INDEX lane_approvals_lane_idx ON lane_approvals(lane_id, requested_at);
CREATE INDEX lane_approvals_status_idx ON lane_approvals(status, requested_at);
CREATE INDEX lane_artifacts_digest_idx
                 ON lane_artifacts(content_digest, artifact_kind);
CREATE INDEX lane_artifacts_session_idx
                 ON lane_artifacts(session_id, created_at, artifact_id);
CREATE INDEX lane_artifacts_turn_idx
                 ON lane_artifacts(turn_id, created_at, artifact_id);
CREATE INDEX lane_events_lane_created_idx ON lane_events(lane_id, created_at);
CREATE INDEX lane_events_lane_type_created_idx ON lane_events(lane_id, event_type, created_at);
CREATE INDEX lane_events_session_created_idx ON lane_events(session_id, created_at);
CREATE INDEX lane_events_session_type_created_idx ON lane_events(session_id, event_type, created_at);
CREATE INDEX lane_events_turn_created_idx ON lane_events(turn_id, created_at);
CREATE INDEX lane_events_turn_type_created_idx ON lane_events(turn_id, event_type, created_at);
CREATE INDEX lane_events_type_created_idx ON lane_events(event_type, created_at);
CREATE INDEX lane_learnings_scope_idx
                 ON lane_learnings(lane_id, scope, status, created_at);
CREATE INDEX lane_learnings_session_idx
                 ON lane_learnings(session_id, turn_id, created_at);
CREATE INDEX lane_merge_queue_active_idx
                ON lane_merge_queue(lane_id, target_ref, status);
CREATE INDEX lane_merge_queue_run_idx
                ON lane_merge_queue(status, priority DESC, created_at ASC);
CREATE INDEX lane_provenance_edges_from_idx
                 ON lane_provenance_edges(from_node_id, relation);
CREATE UNIQUE INDEX lane_provenance_edges_identity_idx
                 ON lane_provenance_edges(
                     from_node_id, to_node_id, relation, COALESCE(receipt_id, '')
                 );
CREATE INDEX lane_provenance_edges_to_idx
                 ON lane_provenance_edges(to_node_id, relation);
CREATE INDEX lane_provenance_nodes_change_idx
                 ON lane_provenance_nodes(change_id, created_at);
CREATE INDEX lane_provenance_nodes_session_idx
                 ON lane_provenance_nodes(session_id, turn_id, node_kind, created_at);
CREATE INDEX lane_run_states_approval_idx ON lane_run_states(approval_id);
CREATE INDEX lane_run_states_lane_idx ON lane_run_states(lane_id, updated_at);
CREATE INDEX lane_run_states_status_idx ON lane_run_states(status, updated_at);
CREATE INDEX lane_session_attestations_session_idx
                 ON lane_session_attestations(session_id, created_at);
CREATE INDEX lane_trace_span_events_span_created_idx ON lane_trace_span_events(span_id, created_at);
CREATE INDEX lane_trace_span_events_trace_created_idx ON lane_trace_span_events(trace_id, created_at);
CREATE INDEX lane_turn_evidence_manifests_session_idx
                 ON lane_turn_evidence_manifests(session_id, created_at);
CREATE INDEX lane_turns_lane_started_idx ON lane_turns(lane_id, started_at);
CREATE INDEX lane_turns_session_started_idx ON lane_turns(session_id, started_at);
CREATE INDEX line_history_line_idx ON line_history(line_id, created_at);
CREATE INDEX memory_embeddings_model_idx ON memory_embeddings(provider, model, dims);
CREATE INDEX memory_items_kind_idx ON memory_items(kind, status, updated_at);
CREATE INDEX memory_items_path_idx ON memory_items(path, status, updated_at);
CREATE INDEX memory_items_scope_idx ON memory_items(scope_type, scope_id, status, updated_at);
CREATE INDEX memory_items_source_change_idx ON memory_items(source_change, updated_at);
CREATE INDEX memory_revisions_memory_idx ON memory_revisions(memory_id, version);
CREATE INDEX memory_revisions_source_change_idx ON memory_revisions(source_change, created_at);
CREATE INDEX operations_branch_created_idx ON operations(branch, created_at);
CREATE INDEX operations_session_created_idx ON operations(session_id, created_at);
CREATE INDEX pending_path_index_derived_repairs_root_idx
                ON pending_path_index_derived_repairs(new_root);
CREATE INDEX workspace_layers_state_used_idx ON workspace_layers(state, last_used_at);
CREATE INDEX workspace_view_layers_layer_idx ON workspace_view_layers(layer_id);
CREATE INDEX workspace_views_status_idx ON workspace_views(status, updated_at);
"#;

pub(super) const LANE_INITIALIZATIONS_V19: &str = r#"
CREATE TABLE lane_initializations (
    initialization_id TEXT PRIMARY KEY,
    lane_name TEXT NOT NULL UNIQUE,
    lane_id TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN
        ('reserved','materialized','associated','observer_ready','repair_required')),
    workdir TEXT,
    materialization_json TEXT,
    last_error_code TEXT,
    last_error_message TEXT,
    repair_command TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX lane_initializations_phase_updated_idx
    ON lane_initializations(phase, updated_at);
"#;

impl Trail {
    pub(crate) fn create_schema_v19(&self) -> Result<()> {
        create_schema_v19(&self.conn)
    }
}

pub(crate) fn create_schema_v19(conn: &Connection) -> Result<()> {
    create_schema(conn, TRAIL_SCHEMA_VERSION, true)
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn create_schema_v18_for_test(conn: &Connection) -> Result<()> {
    create_schema(conn, SCHEMA_V18_VERSION, false)
}

fn create_schema(conn: &Connection, version: i64, lane_initializations: bool) -> Result<()> {
    if conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))? != 0 {
        return Err(Error::Corrupt(
            "fresh schema connection is not empty".into(),
        ));
    }
    conn.execute_batch("SAVEPOINT create_schema;")?;
    let result = (|| {
        conn.execute_batch(BASE_SCHEMA_V18)?;
        super::changed_path_ledger::create_changed_path_ledger_schema(conn)?;
        if lane_initializations {
            conn.execute_batch(LANE_INITIALIZATIONS_V19)?;
        }
        validate_schema_v18_shape(conn)?;
        let now = now_ts();
        for (key, value) in [
            (SCHEMA_META_VERSION_KEY, version.to_string()),
            (
                SCHEMA_META_APP_VERSION_KEY,
                env!("CARGO_PKG_VERSION").to_string(),
            ),
            ("changed_path.observer_log_format_min", "1".to_string()),
            ("changed_path.observer_log_format_max", "1".to_string()),
        ] {
            conn.execute(
                "INSERT INTO schema_meta(key, value, updated_at) VALUES(?1, ?2, ?3)",
                params![key, value, now],
            )?;
        }
        conn.pragma_update(None, "user_version", version)?;
        if lane_initializations {
            validate_schema_v19(conn)
        } else {
            validate_schema_v18_for_migration(conn)
        }
    })();
    match result {
        Ok(()) => conn
            .execute_batch("RELEASE create_schema;")
            .map_err(Into::into),
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK TO create_schema; RELEASE create_schema;");
            Err(err)
        }
    }
}

pub(super) fn validate_lane_initializations_v19_shape(conn: &Connection) -> Result<()> {
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(LANE_INITIALIZATIONS_V19)?;
    let actual = lane_initialization_objects(conn)?;
    let wanted = lane_initialization_objects(&expected)?;
    if actual != wanted {
        return Err(Error::Corrupt(
            "lane initialization schema v19 sqlite_master shape does not match".into(),
        ));
    }
    Ok(())
}

fn lane_initialization_objects(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut statement = conn.prepare(
        "SELECT type,name,COALESCE(sql,'') FROM sqlite_master
         WHERE name LIKE 'lane_initializations%'
         ORDER BY type,name",
    )?;
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                normalize_sql(&row.get::<_, String>(2)?),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(super) fn validate_schema_v18_shape(conn: &Connection) -> Result<()> {
    let expected = Connection::open_in_memory()?;
    expected.pragma_update(None, "foreign_keys", true)?;
    expected.execute_batch(BASE_SCHEMA_V18)?;
    if schema_objects(conn)? != schema_objects(&expected)? {
        return Err(Error::Corrupt(
            "base schema v18 sqlite_master shape does not match".into(),
        ));
    }
    Ok(())
}

pub(super) fn base_schema_complete_for_version(
    conn: &Connection,
    expected_version: i64,
) -> Result<bool> {
    if validate_schema_v18_shape(conn).is_err() {
        return Ok(false);
    }
    let mut statement = conn.prepare(
        "SELECT key, value FROM schema_meta
         WHERE key IN (
             'schema.version',
             'changed_path.observer_log_format_min',
             'changed_path.observer_log_format_max'
         ) ORDER BY key",
    )?;
    let metadata = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(metadata
        == vec![
            (
                "changed_path.observer_log_format_max".to_string(),
                "1".to_string(),
            ),
            (
                "changed_path.observer_log_format_min".to_string(),
                "1".to_string(),
            ),
            (
                SCHEMA_META_VERSION_KEY.to_string(),
                expected_version.to_string(),
            ),
        ]
        && conn
            .query_row(
                "SELECT length(value) > 0 FROM schema_meta WHERE key = 'app.version'",
                [],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            == Some(true))
}

pub(super) fn lane_initialization_objects_absent(conn: &Connection) -> Result<bool> {
    Ok(lane_initialization_objects(conn)?.is_empty())
}

fn schema_objects(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut statement = conn.prepare(
        "SELECT type, name, COALESCE(sql, '') FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%'
           AND name NOT LIKE 'prolly_%'
           AND name NOT LIKE 'changed_path_%'
           AND name NOT LIKE 'lane_initializations%'
         ORDER BY type, name",
    )?;
    let objects = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                normalize_sql(&row.get::<_, String>(2)?),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Error::from)?;
    Ok(objects)
}

fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn master_objects(conn: &Connection) -> Vec<(String, String, Option<String>)> {
        conn.prepare(
            "SELECT type, name, sql FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%'
             ORDER BY type, name",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
    }

    #[test]
    fn late_ledger_ddl_conflict_rolls_back_entire_fresh_creation() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE changed_path_observer_owners (
                sentinel TEXT NOT NULL
             );",
        )
        .unwrap();
        let before = master_objects(&conn);
        let before_user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();

        assert!(create_schema_v19(&conn).is_err());

        assert_eq!(master_objects(&conn), before);
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            before_user_version
        );
    }
}
