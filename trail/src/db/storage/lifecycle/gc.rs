use super::*;
use crate::db::change_ledger::{ledger_gc_roots, IntentGcRoot};

const GC_DELETE_BATCH_SIZE: usize = 256;

impl Trail {
    pub fn gc(&mut self, dry_run: bool) -> Result<GcReport> {
        let _lock = self.acquire_write_lock()?;
        // Capture roots before recovery terminalizes an intent. This makes the
        // recovery boundary itself conservative: the next GC, not this one,
        // may collect a target that recovery proved terminal.
        let intent_roots = ledger_gc_roots(&self.conn)?;
        if !dry_run {
            self.changed_path_ledger().recover()?;
        }
        let reachable = self.reachable_object_ids_with_intent_roots(&intent_roots)?;
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
                    prunable.push((object_id, kind));
                }
            } else {
                preserved_unknown += 1;
            }
        }
        // Envelope rows hold SQL foreign keys to tree-root mappings. Delete
        // unreachable envelopes before all other artifact nodes, then retain
        // object-id order within each class. This is deterministic and avoids
        // depending on unrelated content hashes for referential ordering.
        prunable.sort_by(|left, right| {
            (left.1 != ARTIFACT_ENVELOPE_KIND)
                .cmp(&(right.1 != ARTIFACT_ENVELOPE_KIND))
                .then_with(|| left.0.cmp(&right.0))
        });
        let prunable = prunable
            .into_iter()
            .map(|(object_id, _)| object_id)
            .collect::<Vec<_>>();
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
            for batch in prunable.chunks(GC_DELETE_BATCH_SIZE) {
                report.pruned_objects += self.delete_gc_object_batch(batch)?;
            }
            self.object_cache
                .lock()
                .expect("object cache poisoned")
                .clear();
            let rebuild = self.rebuild_indexes_unlocked()?;
            report.errors.extend(rebuild.errors);
        }
        Ok(report)
    }

    fn delete_gc_object_batch(&self, object_ids: &[String]) -> Result<u64> {
        self.conn.execute_batch("SAVEPOINT trail_gc_object_batch")?;
        let deletion = (|| -> Result<u64> {
            let mut deleted = 0_u64;
            for object_id in object_ids {
                // An envelope row is a lookup/index record, not an independent
                // retention root. Durable consumers (generation bindings,
                // attempts, attestations, quarantines, shadows, and holds) are
                // traced before this point and their foreign keys fail closed
                // if a new root type is ever omitted.
                self.conn.execute(
                    "DELETE FROM artifact_envelopes WHERE object_id=?1",
                    params![object_id],
                )?;
                self.conn.execute(
                    "DELETE FROM artifact_objects WHERE object_id=?1",
                    params![object_id],
                )?;
                let removed = self
                    .conn
                    .execute("DELETE FROM objects WHERE object_id=?1", params![object_id])?;
                if removed != 1 {
                    return Err(Error::Corrupt(format!(
                        "garbage-collection candidate `{object_id}` disappeared during its fenced batch"
                    )));
                }
                deleted = deleted.saturating_add(1);
            }
            Ok(deleted)
        })();
        match deletion {
            Ok(deleted) => {
                self.conn
                    .execute_batch("RELEASE SAVEPOINT trail_gc_object_batch")?;
                Ok(deleted)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_gc_object_batch;
                     RELEASE SAVEPOINT trail_gc_object_batch",
                );
                Err(error)
            }
        }
    }

    fn reachable_object_ids_with_intent_roots(
        &self,
        intent_roots: &[IntentGcRoot],
    ) -> Result<HashSet<String>> {
        let (operation_objects, mut errors) = self.operation_objects()?;
        let reachable_changes =
            self.reachable_operation_changes(&operation_objects, &mut errors)?;
        let by_change = operation_objects
            .iter()
            .map(|object| (object.operation.change_id.0.clone(), object))
            .collect::<HashMap<_, _>>();
        let mut reachable = HashSet::new();

        let by_object = operation_objects
            .iter()
            .map(|object| (object.object_id.0.as_str(), object))
            .collect::<HashMap<_, _>>();
        for intent in intent_roots {
            self.collect_root_reachable(&intent.root_id, &mut reachable, &mut errors);
            let Some(operation_id) = &intent.operation_id else {
                continue;
            };
            let Some(target_operation) = by_object.get(operation_id.0.as_str()) else {
                errors.push(format!(
                    "intent operation {} is missing or is not a valid operation object",
                    operation_id.0
                ));
                continue;
            };
            if target_operation.operation.change_id != intent.change_id
                || target_operation.operation.after_root != intent.root_id
            {
                errors.push(format!(
                    "intent operation {} does not match target change/root",
                    operation_id.0
                ));
                continue;
            }
            let mut pending = vec![intent.change_id.0.clone()];
            while let Some(change_id) = pending.pop() {
                let Some(object) = by_change.get(&change_id) else {
                    errors.push(format!(
                        "intent operation ancestry is missing change {change_id}"
                    ));
                    continue;
                };
                if !reachable.insert(object.object_id.0.clone()) {
                    continue;
                }
                if let Some(before) = &object.operation.before_root {
                    self.collect_root_reachable(before, &mut reachable, &mut errors);
                }
                self.collect_root_reachable(
                    &object.operation.after_root,
                    &mut reachable,
                    &mut errors,
                );
                pending.extend(
                    object
                        .operation
                        .parents
                        .iter()
                        .map(|parent| parent.0.clone()),
                );
            }
        }

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

        for object_id in self.workspace_layer_object_roots()? {
            reachable.insert(object_id);
        }

        self.collect_artifact_gc_reachable(&mut reachable, &mut errors)?;

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

    fn collect_artifact_gc_reachable(
        &self,
        reachable: &mut HashSet<String>,
        errors: &mut Vec<String>,
    ) -> Result<()> {
        let integrity_errors = self.validate_artifact_cas_integrity()?;
        if !integrity_errors.is_empty() {
            errors.extend(integrity_errors);
            return Ok(());
        }

        let mut pending_artifacts = BTreeSet::<(String, String)>::new();
        let mut pending_objects = BTreeSet::<String>::new();

        {
            let mut statement = self.conn.prepare(
                "SELECT envelope_id,tree_root_id FROM artifact_generation_bindings
                 ORDER BY binding_id",
            )?;
            for row in statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (envelope_id, tree_root_id) = row?;
                pending_artifacts.insert((envelope_id, ARTIFACT_ENVELOPE_KIND.into()));
                pending_artifacts.insert((tree_root_id, ARTIFACT_TREE_ROOT_KIND.into()));
            }
        }
        {
            let mut statement = self.conn.prepare(
                "SELECT envelope_id,tree_root_id FROM workspace_layer_artifact_shadows
                 ORDER BY layer_id",
            )?;
            for row in statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (envelope_id, tree_root_id) = row?;
                pending_artifacts.insert((envelope_id, ARTIFACT_ENVELOPE_KIND.into()));
                pending_artifacts.insert((tree_root_id, ARTIFACT_TREE_ROOT_KIND.into()));
            }
        }
        {
            // A verified real-directory cache is a conservative local lease.
            // Cache eviction removes this row independently; the next object
            // GC can then reclaim the authoritative graph if no durable root
            // remains.
            let mut statement = self.conn.prepare(
                "SELECT tree_root_id FROM artifact_materializations
                 ORDER BY materialization_id",
            )?;
            for row in statement.query_map([], |row| row.get::<_, String>(0))? {
                pending_artifacts.insert((row?, ARTIFACT_TREE_ROOT_KIND.into()));
            }
        }
        {
            let mut statement = self.conn.prepare(
                "SELECT source_root,candidate_journal_object_id,envelope_id
                 FROM artifact_construction_attempts ORDER BY attempt_id",
            )?;
            for row in statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })? {
                let (source_root, journal, envelope) = row?;
                pending_objects.insert(source_root);
                pending_objects.extend(journal);
                if let Some(envelope) = envelope {
                    pending_artifacts.insert((envelope, ARTIFACT_ENVELOPE_KIND.into()));
                }
            }
        }
        {
            let mut statement = self.conn.prepare(
                "SELECT source_root,plan_object_id,stdout_object_id,stderr_object_id,
                        snapshot_id,failure_receipt_object_id
                 FROM artifact_resolution_attempts ORDER BY attempt_id",
            )?;
            for row in statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })? {
                let (source, plan, stdout, stderr, snapshot, failure) = row?;
                pending_objects.insert(source);
                pending_objects.insert(plan);
                pending_objects.extend(stdout);
                pending_objects.extend(stderr);
                pending_objects.extend(snapshot);
                pending_objects.extend(failure);
            }
        }
        {
            let mut statement = self.conn.prepare(
                "SELECT snapshot_id,source_root,content_object_id,predecessor_snapshot_id
                 FROM artifact_resolution_snapshots ORDER BY snapshot_id",
            )?;
            for row in statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })? {
                let (snapshot, source, content, predecessor) = row?;
                pending_objects.insert(snapshot);
                pending_objects.insert(source);
                pending_objects.insert(content);
                pending_objects.extend(predecessor);
            }
        }
        {
            let mut statement = self.conn.prepare(
                "SELECT envelope_id,object_id FROM artifact_attestations
                 ORDER BY attestation_id",
            )?;
            for row in statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (envelope, object) = row?;
                pending_artifacts.insert((envelope, ARTIFACT_ENVELOPE_KIND.into()));
                pending_objects.insert(object);
            }
        }
        {
            let mut statement = self.conn.prepare(
                "SELECT incumbent_envelope_id,candidate_envelope_id,evidence_object_id
                 FROM artifact_quarantines ORDER BY quarantine_id",
            )?;
            for row in statement.query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })? {
                let (incumbent, candidate, evidence) = row?;
                if let Some(incumbent) = incumbent {
                    pending_artifacts.insert((incumbent, ARTIFACT_ENVELOPE_KIND.into()));
                }
                pending_artifacts.insert((candidate, ARTIFACT_ENVELOPE_KIND.into()));
                pending_objects.insert(evidence);
            }
        }
        {
            let now = now_ts();
            let mut statement = self.conn.prepare(
                "SELECT hold_id,target_kind,target_id FROM artifact_holds
                 WHERE expires_at IS NULL OR expires_at>?1 ORDER BY hold_id",
            )?;
            for row in statement.query_map(params![now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })? {
                let (hold_id, target_kind, target_id) = row?;
                match target_kind.as_str() {
                    "artifact_envelope" => {
                        pending_artifacts.insert((target_id, ARTIFACT_ENVELOPE_KIND.into()));
                    }
                    "artifact_tree" => {
                        pending_artifacts.insert((target_id, ARTIFACT_TREE_ROOT_KIND.into()));
                    }
                    "artifact_object" => {
                        let kind = self
                            .conn
                            .query_row(
                                "SELECT kind FROM artifact_objects WHERE artifact_id=?1",
                                params![target_id],
                                |row| row.get::<_, String>(0),
                            )
                            .optional()?;
                        match kind {
                            Some(kind) => {
                                pending_artifacts.insert((target_id, kind));
                            }
                            None => errors.push(format!(
                                "artifact hold {hold_id} references missing artifact object {target_id}"
                            )),
                        }
                    }
                    "object" | "resolution_snapshot" => {
                        pending_objects.insert(target_id);
                    }
                    _ => errors.push(format!(
                        "artifact hold {hold_id} has unsupported target kind `{target_kind}`"
                    )),
                }
            }
        }
        {
            // Publication rows and external backups are different durability
            // boundaries: an in-progress publication remains in this database
            // and roots its pins, while a completed backup is a self-contained
            // archive created under the same workspace write lock.
            let mut statement = self.conn.prepare(
                "SELECT source_root,manifest_object_id FROM workspace_layer_publications
                 ORDER BY publication_id",
            )?;
            for row in statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })? {
                let (source_root, manifest) = row?;
                pending_objects.insert(source_root);
                pending_objects.extend(manifest);
            }
        }

        let mut visited_artifacts = HashSet::<String>::new();
        let mut visited_objects = HashSet::<String>::new();
        while !pending_artifacts.is_empty() || !pending_objects.is_empty() {
            while let Some((artifact_id, expected_kind)) = pending_artifacts.pop_first() {
                let mapping = self
                    .conn
                    .query_row(
                        "SELECT object_id,kind FROM artifact_objects WHERE artifact_id=?1",
                        params![artifact_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?;
                let Some((object_id, actual_kind)) = mapping else {
                    errors.push(format!(
                        "reachable artifact object {artifact_id} is missing its CAS mapping"
                    ));
                    continue;
                };
                if actual_kind != expected_kind {
                    errors.push(format!(
                        "reachable artifact object {artifact_id} has kind {actual_kind}, expected {expected_kind}"
                    ));
                    continue;
                }
                if visited_artifacts.insert(artifact_id) {
                    pending_objects.insert(object_id);
                }
            }

            let Some(object_id) = pending_objects.pop_first() else {
                continue;
            };
            if !visited_objects.insert(object_id.clone()) {
                continue;
            }
            let Some((kind, version, bytes)) = self.validated_gc_object(&object_id, errors)? else {
                continue;
            };
            reachable.insert(object_id.clone());

            let edge_result = (|| -> Result<()> {
                match kind.as_str() {
                    WORKTREE_ROOT_KIND => {
                        self.collect_root_reachable(
                            &ObjectId(object_id.clone()),
                            reachable,
                            errors,
                        );
                    }
                    ARTIFACT_DIRECTORY_NODE_KIND => {
                        require_gc_object_version(&kind, version, ARTIFACT_DIRECTORY_NODE_VERSION)?;
                        let node: ArtifactDirectoryNodeV1 = from_cbor(&bytes)?;
                        for entry in node.entries {
                            match entry.target {
                                ArtifactDirectoryEntryTargetV1::Directory { node_id } => {
                                    pending_artifacts
                                        .insert((node_id.0, ARTIFACT_DIRECTORY_NODE_KIND.into()));
                                }
                                ArtifactDirectoryEntryTargetV1::File { node_id } => {
                                    pending_artifacts
                                        .insert((node_id.0, ARTIFACT_FILE_NODE_KIND.into()));
                                }
                                ArtifactDirectoryEntryTargetV1::Symlink { .. } => {}
                            }
                        }
                    }
                    ARTIFACT_FILE_NODE_KIND => {
                        require_gc_object_version(&kind, version, ARTIFACT_FILE_NODE_VERSION)?;
                        let node: ArtifactFileNodeV1 = from_cbor(&bytes)?;
                        match node.content {
                            ArtifactFileContentV1::Blob { blob_id } => {
                                pending_artifacts.insert((blob_id.0, ARTIFACT_BLOB_KIND.into()));
                            }
                            ArtifactFileContentV1::Chunks { chunk_list_id } => {
                                pending_artifacts
                                    .insert((chunk_list_id.0, ARTIFACT_CHUNK_LIST_KIND.into()));
                            }
                        }
                    }
                    ARTIFACT_CHUNK_LIST_KIND => {
                        require_gc_object_version(&kind, version, ARTIFACT_CHUNK_LIST_VERSION)?;
                        let list: ArtifactChunkListV1 = from_cbor(&bytes)?;
                        for chunk in list.chunks {
                            pending_artifacts
                                .insert((chunk.chunk_id.0, ARTIFACT_CHUNK_KIND.into()));
                        }
                    }
                    ARTIFACT_TREE_ROOT_KIND => {
                        require_gc_object_version(&kind, version, ARTIFACT_TREE_ROOT_VERSION)?;
                        let tree: ArtifactTreeRootV1 = from_cbor(&bytes)?;
                        pending_artifacts.insert((
                            tree.root_directory_id.0,
                            ARTIFACT_DIRECTORY_NODE_KIND.into(),
                        ));
                    }
                    ARTIFACT_ENVELOPE_KIND => {
                        require_gc_object_version(&kind, version, ARTIFACT_ENVELOPE_VERSION)?;
                        let envelope: ArtifactEnvelopeV1 = from_cbor(&bytes)?;
                        pending_artifacts
                            .insert((envelope.tree_root_id.0, ARTIFACT_TREE_ROOT_KIND.into()));
                        pending_objects
                            .extend(envelope.resolution_snapshot_id.into_iter().map(|id| id.0));
                        pending_objects
                            .extend(envelope.validation_receipt_ids.into_iter().map(|id| id.0));
                    }
                    ARTIFACT_RESOLUTION_PLAN_KIND => {
                        require_gc_object_version(
                            &kind,
                            version,
                            ARTIFACT_RESOLUTION_PLAN_VERSION,
                        )?;
                        let plan: ArtifactResolutionPlanV1 = from_cbor(&bytes)?;
                        pending_objects.insert(plan.source_root.0);
                    }
                    ARTIFACT_RESOLUTION_SNAPSHOT_KIND => {
                        require_gc_object_version(
                            &kind,
                            version,
                            ARTIFACT_RESOLUTION_SNAPSHOT_VERSION,
                        )?;
                        let snapshot: ArtifactResolutionSnapshotV1 = from_cbor(&bytes)?;
                        pending_objects.insert(snapshot.source_root.0);
                        pending_objects.insert(snapshot.content_object_id.0);
                        pending_objects
                            .extend(snapshot.predecessor_snapshot_id.into_iter().map(|id| id.0));
                    }
                    ARTIFACT_RESOLUTION_FAILURE_KIND => {
                        require_gc_object_version(
                            &kind,
                            version,
                            ARTIFACT_RESOLUTION_PLAN_VERSION,
                        )?;
                        let receipt: ArtifactResolutionFailureReceiptV1 = from_cbor(&bytes)?;
                        pending_objects.insert(receipt.source_root.0);
                        pending_objects.extend(receipt.stdout_object_id.into_iter().map(|id| id.0));
                        pending_objects.extend(receipt.stderr_object_id.into_iter().map(|id| id.0));
                    }
                    ARTIFACT_DIVERGENCE_EVIDENCE_KIND => {
                        require_gc_object_version(
                            &kind,
                            version,
                            ARTIFACT_DIVERGENCE_EVIDENCE_VERSION,
                        )?;
                        let evidence: ArtifactDivergenceEvidenceV1 = from_cbor(&bytes)?;
                        pending_artifacts.insert((
                            evidence.incumbent_envelope_id.0,
                            ARTIFACT_ENVELOPE_KIND.into(),
                        ));
                        pending_artifacts.insert((
                            evidence.incumbent_tree_root_id.0,
                            ARTIFACT_TREE_ROOT_KIND.into(),
                        ));
                        pending_artifacts.insert((
                            evidence.candidate_envelope_id.0,
                            ARTIFACT_ENVELOPE_KIND.into(),
                        ));
                        pending_artifacts.insert((
                            evidence.candidate_tree_root_id.0,
                            ARTIFACT_TREE_ROOT_KIND.into(),
                        ));
                    }
                    ARTIFACT_BLOB_KIND => {
                        require_gc_object_version(&kind, version, ARTIFACT_BLOB_VERSION)?;
                        let _: ArtifactBlobV1 = from_cbor(&bytes)?;
                    }
                    ARTIFACT_CHUNK_KIND => {
                        require_gc_object_version(&kind, version, ARTIFACT_CHUNK_VERSION)?;
                        let _: ArtifactChunkV1 = from_cbor(&bytes)?;
                    }
                    ARTIFACT_RESOLUTION_CONTENT_KIND => {
                        require_gc_object_version(
                            &kind,
                            version,
                            ARTIFACT_RESOLUTION_SNAPSHOT_VERSION,
                        )?;
                        let _: ArtifactResolutionContentV1 = from_cbor(&bytes)?;
                    }
                    ARTIFACT_RESOLUTION_CAPTURE_KIND => {
                        require_gc_object_version(
                            &kind,
                            version,
                            ARTIFACT_RESOLUTION_PLAN_VERSION,
                        )?;
                        let _: ArtifactResolutionCaptureV1 = from_cbor(&bytes)?;
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(error) = edge_result {
                errors.push(format!(
                    "failed to traverse reachable object {object_id} ({kind}): {error}"
                ));
            }
        }
        Ok(())
    }

    fn validated_gc_object(
        &self,
        object_id: &str,
        errors: &mut Vec<String>,
    ) -> Result<Option<(String, u16, Vec<u8>)>> {
        let row = self
            .conn
            .query_row(
                "SELECT kind,version,codec,hash_alg,size_bytes,bytes
                 FROM objects WHERE object_id=?1",
                params![object_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, version, codec, hash_alg, size_bytes, bytes)) = row else {
            errors.push(format!("reachable object {object_id} is missing"));
            return Ok(None);
        };
        let Ok(version) = u16::try_from(version) else {
            errors.push(format!(
                "reachable object {object_id} has an invalid version"
            ));
            return Ok(None);
        };
        if codec != "cbor"
            || hash_alg != "sha256"
            || i64::try_from(bytes.len()).ok() != Some(size_bytes)
            || ObjectId::for_bytes(&kind, version, &bytes).0 != object_id
        {
            errors.push(format!(
                "reachable object {object_id} has invalid metadata or content identity"
            ));
            return Ok(None);
        }
        Ok(Some((kind, version, bytes)))
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

fn require_gc_object_version(kind: &str, actual: u16, expected: u16) -> Result<()> {
    if actual != expected {
        return Err(Error::Corrupt(format!(
            "object kind {kind} has version {actual}, expected {expected}"
        )));
    }
    Ok(())
}
