use super::*;

const PENDING_REPAIR_LANE_MANIFEST: &str = "lane_manifest";
const PENDING_REPAIR_WORKSPACE_CHECKPOINT: &str = "workspace_checkpoint";

struct PreparedPathIndexRepair {
    old_root: ObjectId,
    new_root: ObjectId,
    case_fold_map_root: String,
}

#[derive(Clone)]
struct CleanGitMappingRepairSource {
    direction: String,
    branch: String,
    git_head: Option<String>,
}

struct PreparedLaneRepair {
    branch: LaneBranch,
    retarget_clean_manifest: bool,
    checkpoint_view_id: Option<String>,
    retarget_checkpoint_marker: bool,
}

struct PendingPathIndexDerivedRepair {
    ref_name: String,
    repair_kind: String,
    old_root: ObjectId,
    new_root: ObjectId,
    new_change: ChangeId,
}

fn remove_path_index_derived_mirror(path: &Path) -> bool {
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

struct PreparedRefRepairPublication {
    reference: RefRecord,
    new_root: ObjectId,
    change_id: ChangeId,
    operation_id: ObjectId,
    operation: Operation,
    git_mappings: Vec<CleanGitMappingRepairSource>,
    lane: Option<PreparedLaneRepair>,
    retarget_current_worktree_baseline: bool,
}

struct PreflightRefRepair {
    reference: RefRecord,
    new_root: ObjectId,
    git_mappings: Vec<CleanGitMappingRepairSource>,
    lane: Option<PreparedLaneRepair>,
    retarget_current_worktree_baseline: bool,
}

#[derive(Default)]
struct PathIndexRepairOutcome {
    roots: Vec<PathIndexRootRepair>,
    refs: Vec<PathIndexRefRepair>,
}

#[cfg(test)]
#[derive(Clone, Copy, Default)]
struct PathIndexRebuildMemoryMetrics {
    live_paths: usize,
    peak_live_paths: usize,
    observed_paths: usize,
}

#[cfg(test)]
thread_local! {
    static PATH_INDEX_REBUILD_MEMORY_METRICS: Cell<PathIndexRebuildMemoryMetrics> =
        Cell::new(PathIndexRebuildMemoryMetrics::default());
}

#[cfg(test)]
fn reset_path_index_rebuild_memory_metrics() {
    PATH_INDEX_REBUILD_MEMORY_METRICS
        .with(|metrics| metrics.set(PathIndexRebuildMemoryMetrics::default()));
}

#[cfg(test)]
fn retain_path_index_rebuild_paths(count: usize) {
    PATH_INDEX_REBUILD_MEMORY_METRICS.with(|metrics| {
        let mut next = metrics.get();
        next.live_paths += count;
        next.observed_paths += count;
        next.peak_live_paths = next.peak_live_paths.max(next.live_paths);
        metrics.set(next);
    });
}

#[cfg(test)]
fn release_path_index_rebuild_paths(count: usize) {
    PATH_INDEX_REBUILD_MEMORY_METRICS.with(|metrics| {
        let mut next = metrics.get();
        next.live_paths = next.live_paths.saturating_sub(count);
        metrics.set(next);
    });
}

#[cfg(test)]
fn path_index_rebuild_memory_metrics() -> PathIndexRebuildMemoryMetrics {
    PATH_INDEX_REBUILD_MEMORY_METRICS.with(Cell::get)
}

impl Trail {
    pub fn rebuild_indexes(&mut self) -> Result<IndexRebuildReport> {
        let _lock = self.acquire_write_lock()?;
        self.rebuild_indexes_unlocked()
    }

    pub fn rebuild_indexes_with_rich_text(&mut self) -> Result<IndexRebuildReport> {
        let _lock = self.acquire_write_lock()?;
        let hydrated = self.hydrate_current_branch_rich_text_unlocked()?;
        let mut report = self.rebuild_indexes_unlocked()?;
        report.rich_text_hydrated = hydrated;
        Ok(report)
    }

    fn hydrate_current_branch_rich_text_unlocked(&self) -> Result<u64> {
        let branch = self.current_branch()?;
        let head = self.resolve_branch_ref(&branch)?;
        let mut files = self.load_root_files(&head.root_id)?;
        let mut lazy_texts = Vec::new();

        for (path, entry) in &files {
            let FileContentRef::Text(text_id) = &entry.content else {
                continue;
            };
            let content: TextContent = self.get_object(TEXT_CONTENT_KIND, text_id)?;
            if matches!(content.representation, TextRepresentation::LazyText { .. }) {
                lazy_texts.push((path.clone(), text_id.clone()));
            }
        }

        if lazy_texts.is_empty() {
            return Ok(0);
        }

        for (path, text_id) in &lazy_texts {
            let lines = self.load_text_lines(text_id)?;
            let rich_text_id = self.put_text_content_from_lines(&lines)?;
            if let Some(entry) = files.get_mut(path) {
                entry.content = FileContentRef::Text(rich_text_id);
            }
        }

        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "hydrate-rich-text")?;
        let built = self.build_root_from_file_entries(files, &change_id)?;
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::ManualCheckpoint,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch,
            actor,
            session_id: None,
            message: Some("Hydrate lazy text indexes".to_string()),
            changes: Vec::new(),
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        self.set_worktree_index_baseline(&built.root_id)?;
        Ok(lazy_texts.len() as u64)
    }

    pub(crate) fn rebuild_indexes_unlocked(&self) -> Result<IndexRebuildReport> {
        self.drain_pending_path_index_derived_repairs()?;
        // Repair immutable root state before deleting/rebuilding the derived
        // operation indexes. The maintenance operations published below must
        // participate in reachability and be indexed by this same command.
        let path_index_repairs = self.rebuild_live_path_invariant_indexes_unlocked()?;
        // Ref files are derived mirrors of the authoritative SQLite refs. Run
        // this on every rebuild so an interrupted/permission-blocked mirror
        // write after a prior committed repair remains retryable.
        self.reconcile_live_ref_files_best_effort();
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
            path_index_repaired_roots: path_index_repairs.roots,
            path_index_repaired_refs: path_index_repairs.refs,
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

        self.rebuild_lane_trace_span_event_index()?;

        Ok(report)
    }

    fn rebuild_live_path_invariant_indexes_unlocked(&self) -> Result<PathIndexRepairOutcome> {
        let live_refs = self
            .all_refs()?
            .into_iter()
            .filter(|reference| {
                reference.name.starts_with(MAIN_REF_PREFIX)
                    || reference.name.starts_with(LANE_REF_PREFIX)
            })
            .collect::<Vec<_>>();

        // First validate every distinct legacy root. No operation, ref, lane,
        // baseline, or Git-mapping metadata is published until all roots pass.
        let mut examined_roots = BTreeSet::new();
        let mut legacy_roots = BTreeMap::<ObjectId, u64>::new();
        for reference in &live_refs {
            if !examined_roots.insert(reference.root_id.clone()) {
                continue;
            }
            let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &reference.root_id)?;
            if root.case_fold_map_root.is_some() || root.file_count == 0 {
                continue;
            }
            self.with_validated_legacy_root_paths(&reference.root_id, root.file_count, |_| Ok(()))?;
            legacy_roots.insert(reference.root_id.clone(), root.file_count);
        }

        // Building may write content-addressed Prolly nodes and root objects,
        // but only after every legacy root has passed path validation.
        let mut prepared = BTreeMap::<ObjectId, PreparedPathIndexRepair>::new();
        for (old_root_id, file_count) in legacy_roots {
            let mut root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &old_root_id)?;
            let case_fold_map_root =
                self.with_validated_legacy_root_paths(&old_root_id, file_count, |paths| {
                    let case_fold_tree = self.build_case_fold_map_tree(paths.iter())?;
                    tree_root_hex(&case_fold_tree).ok_or_else(|| {
                        Error::Corrupt(format!(
                            "non-empty legacy root {} produced an empty path-invariant index",
                            old_root_id.0
                        ))
                    })
                })?;
            root.case_fold_map_root = Some(case_fold_map_root.clone());
            let new_root = self.put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &root)?;
            prepared.insert(
                old_root_id.clone(),
                PreparedPathIndexRepair {
                    old_root: old_root_id,
                    new_root,
                    case_fold_map_root,
                },
            );
        }

        if prepared.is_empty() {
            return Ok(PathIndexRepairOutcome::default());
        }

        let current_branch_ref = branch_ref(&self.current_branch()?);
        let current_worktree_baseline = self.worktree_index_baseline_root()?;
        let mut preflight_refs = Vec::new();

        // Preflight every ref's derived metadata before creating maintenance
        // operations or advancing the first ref. This keeps a corrupt later
        // lane from partially publishing repairs for earlier refs.
        for reference in live_refs {
            let Some(repair) = prepared.get(&reference.root_id) else {
                continue;
            };
            let lane = if let Some(lane_name) = reference.name.strip_prefix(LANE_REF_PREFIX) {
                Some(self.preflight_lane_path_index_repair(&reference, lane_name)?)
            } else {
                None
            };
            let git_mappings = self.clean_git_mapping_sources_for_path_index_repair(&reference)?;
            preflight_refs.push(PreflightRefRepair {
                retarget_current_worktree_baseline: reference.name == current_branch_ref
                    && current_worktree_baseline.as_ref() == Some(&reference.root_id),
                reference,
                new_root: repair.new_root.clone(),
                git_mappings,
                lane,
            });
        }

        let mut publications = Vec::new();
        for preflight in preflight_refs {
            let actor = Actor::system();
            let change_id = self.allocate_change_id(&actor.id, "path-index-rebuild")?;
            let operation = Operation {
                version: OP_OBJECT_VERSION,
                change_id: change_id.clone(),
                kind: OperationKind::ManualCheckpoint,
                parents: vec![preflight.reference.change_id.clone()],
                before_root: Some(preflight.reference.root_id.clone()),
                after_root: preflight.new_root.clone(),
                branch: preflight
                    .reference
                    .name
                    .strip_prefix(MAIN_REF_PREFIX)
                    .unwrap_or(&preflight.reference.name)
                    .to_string(),
                actor,
                session_id: None,
                message: Some("Rebuild path invariant index".to_string()),
                changes: Vec::new(),
                created_at: now_ts(),
            };
            // The object is immutable and may safely become orphaned if the
            // later atomic SQLite publication fails.
            let operation_id = self.put_object(OPERATION_KIND, OP_OBJECT_VERSION, &operation)?;
            publications.push(PreparedRefRepairPublication {
                retarget_current_worktree_baseline: preflight.retarget_current_worktree_baseline,
                reference: preflight.reference,
                new_root: preflight.new_root,
                change_id,
                operation_id,
                operation,
                git_mappings: preflight.git_mappings,
                lane: preflight.lane,
            });
        }

        // Publish all authoritative SQLite metadata as one unit. Ref files and
        // clean manifest/checkpoint files are derived mirrors and are refreshed
        // after commit using their existing recovery semantics.
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let publication_result = (|| -> Result<()> {
            for publication in &publications {
                self.index_operation_in_transaction(
                    &publication.operation,
                    &publication.operation_id,
                )?;
                if let Some(lane) = &publication.lane {
                    if lane.retarget_clean_manifest {
                        self.insert_pending_path_index_derived_repair(
                            &publication.reference.name,
                            PENDING_REPAIR_LANE_MANIFEST,
                            &publication.reference.root_id,
                            &publication.new_root,
                            &publication.change_id,
                        )?;
                    }
                    if lane.retarget_checkpoint_marker {
                        self.insert_pending_path_index_derived_repair(
                            &publication.reference.name,
                            PENDING_REPAIR_WORKSPACE_CHECKPOINT,
                            &publication.reference.root_id,
                            &publication.new_root,
                            &publication.change_id,
                        )?;
                    }
                }
                let generation = publication.reference.generation + 1;
                let updated = self.conn.execute(
                    "UPDATE refs SET change_id = ?1, root_id = ?2, operation_id = ?3, generation = ?4, updated_at = ?5 \
                     WHERE name = ?6 AND generation = ?7 AND change_id = ?8 AND root_id = ?9",
                    params![
                        publication.change_id.0,
                        publication.new_root.0,
                        publication.operation_id.0,
                        generation,
                        now_ts(),
                        publication.reference.name,
                        publication.reference.generation,
                        publication.reference.change_id.0,
                        publication.reference.root_id.0
                    ],
                )?;
                if updated != 1 {
                    return Err(Error::StaleBranch(publication.reference.name.clone()));
                }

                if let Some(lane) = &publication.lane {
                    let updated = self.conn.execute(
                        "UPDATE lane_branches SET head_change = ?1, head_root = ?2, updated_at = ?3 \
                         WHERE lane_id = ?4 AND head_change = ?5 AND head_root = ?6",
                        params![
                            publication.change_id.0,
                            publication.new_root.0,
                            now_ts(),
                            lane.branch.lane_id,
                            publication.reference.change_id.0,
                            publication.reference.root_id.0
                        ],
                    )?;
                    if updated != 1 {
                        return Err(Error::Corrupt(format!(
                            "lane branch {} changed during path-index repair",
                            lane.branch.ref_name
                        )));
                    }
                    if let Some(view_id) = &lane.checkpoint_view_id {
                        let updated = self.conn.execute(
                            "UPDATE workspace_views SET checkpoint_root = ?1, updated_at = ?2 WHERE view_id = ?3 AND checkpoint_root = ?4",
                            params![
                                publication.new_root.0,
                                now_ts(),
                                view_id,
                                publication.reference.root_id.0
                            ],
                        )?;
                        if updated != 1 {
                            return Err(Error::Corrupt(format!(
                                "workspace view {view_id} changed during path-index repair"
                            )));
                        }
                    }
                }

                if publication.retarget_current_worktree_baseline {
                    self.conn.execute(
                        "INSERT OR REPLACE INTO schema_meta (key, value, updated_at) VALUES (?1, ?2, ?3)",
                        params![
                            "worktree.index.baseline_root",
                            publication.new_root.0,
                            now_ts()
                        ],
                    )?;
                }
                for mapping in &publication.git_mappings {
                    self.insert_git_mapping_for_state(
                        &mapping.direction,
                        &mapping.branch,
                        &publication.change_id,
                        &publication.new_root,
                        mapping.git_head.clone(),
                        false,
                    )?;
                }
            }
            Ok(())
        })();
        match publication_result {
            Ok(()) => {
                if let Err(err) = self.conn.execute_batch("COMMIT;") {
                    let _ = self.conn.execute_batch("ROLLBACK;");
                    return Err(Error::from(err));
                }
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(err);
            }
        }
        test_crash_point("path_index_rebuild_after_publication_commit");
        self.drain_pending_path_index_derived_repairs()?;

        let mut outcome = PathIndexRepairOutcome {
            roots: prepared
                .values()
                .map(|repair| PathIndexRootRepair {
                    old_root: repair.old_root.clone(),
                    new_root: repair.new_root.clone(),
                    case_fold_map_root: repair.case_fold_map_root.clone(),
                })
                .collect(),
            refs: Vec::new(),
        };
        for publication in publications {
            outcome.refs.push(PathIndexRefRepair {
                name: publication.reference.name,
                old_change: publication.reference.change_id,
                new_change: publication.change_id,
                old_root: publication.reference.root_id,
                new_root: publication.new_root,
            });
        }
        Ok(outcome)
    }

    fn with_validated_legacy_root_paths<T>(
        &self,
        root_id: &ObjectId,
        expected_file_count: u64,
        consume: impl FnOnce(&[String]) -> Result<T>,
    ) -> Result<T> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let path_tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let mut paths = Vec::new();
        for item in self.root_prolly.range(&path_tree, &[], None)? {
            let (key, _) = item?;
            let path = String::from_utf8(key).map_err(|err| {
                Error::Corrupt(format!(
                    "legacy root {} has a non UTF-8 path-map key: {err}",
                    root_id.0
                ))
            })?;
            let normalized = normalize_relative_path(&path).map_err(|err| {
                Error::Corrupt(format!(
                    "legacy root {} has invalid path-map key {path:?}: {err}",
                    root_id.0
                ))
            })?;
            if normalized != path {
                return Err(Error::Corrupt(format!(
                    "legacy root {} has noncanonical path-map key {path:?}; path must be normalized as {normalized:?}",
                    root_id.0
                )));
            }
            paths.push(path);
        }
        if paths.len() as u64 != expected_file_count {
            return Err(Error::Corrupt(format!(
                "legacy root {} declares {} files but its path map contains {} entries",
                root_id.0,
                expected_file_count,
                paths.len()
            )));
        }
        validate_no_case_fold_collisions(paths.iter()).map_err(|err| match err {
            Error::InvalidPath { path, reason } => Error::InvalidPath {
                path,
                reason: format!("legacy root {}: {reason}", root_id.0),
            },
            other => other,
        })?;

        #[cfg(test)]
        retain_path_index_rebuild_paths(paths.len());
        let result = consume(&paths);
        #[cfg(test)]
        release_path_index_rebuild_paths(paths.len());
        result
    }

    fn insert_pending_path_index_derived_repair(
        &self,
        ref_name: &str,
        repair_kind: &str,
        old_root: &ObjectId,
        new_root: &ObjectId,
        new_change: &ChangeId,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO pending_path_index_derived_repairs \
             (ref_name, repair_kind, old_root, new_root, new_change, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(ref_name, repair_kind) DO UPDATE SET \
             old_root = excluded.old_root, new_root = excluded.new_root, \
             new_change = excluded.new_change, created_at = excluded.created_at",
            params![
                ref_name,
                repair_kind,
                old_root.0,
                new_root.0,
                new_change.0,
                now_ts()
            ],
        )?;
        Ok(())
    }

    pub(crate) fn has_pending_path_index_derived_repairs(&self) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pending_path_index_derived_repairs LIMIT 1)",
                [],
                |row| row.get(0),
            )
            .map_err(Error::from)
    }

    pub(crate) fn drain_pending_path_index_derived_repairs(&self) -> Result<()> {
        self.drain_pending_path_index_derived_repairs_with_db_mapping(None)
    }

    pub(crate) fn drain_pending_path_index_derived_repairs_from_restore_stage(
        &self,
        published_db_dir: &Path,
    ) -> Result<()> {
        self.drain_pending_path_index_derived_repairs_with_db_mapping(Some(published_db_dir))
    }

    fn drain_pending_path_index_derived_repairs_with_db_mapping(
        &self,
        published_db_dir: Option<&Path>,
    ) -> Result<()> {
        let pending = {
            let mut stmt = self.conn.prepare(
                "SELECT ref_name, repair_kind, old_root, new_root, new_change \
                 FROM pending_path_index_derived_repairs \
                 ORDER BY ref_name, repair_kind",
            )?;
            let repairs = stmt
                .query_map([], |row| {
                    Ok(PendingPathIndexDerivedRepair {
                        ref_name: row.get(0)?,
                        repair_kind: row.get(1)?,
                        old_root: ObjectId(row.get(2)?),
                        new_root: ObjectId(row.get(3)?),
                        new_change: ChangeId(row.get(4)?),
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            repairs
        };

        for repair in pending {
            if !self.repair_pending_path_index_derived_mirror(&repair, published_db_dir)? {
                continue;
            }
            self.conn.execute(
                "DELETE FROM pending_path_index_derived_repairs \
                 WHERE ref_name = ?1 AND repair_kind = ?2 AND old_root = ?3 \
                   AND new_root = ?4 AND new_change = ?5",
                params![
                    repair.ref_name,
                    repair.repair_kind,
                    repair.old_root.0,
                    repair.new_root.0,
                    repair.new_change.0
                ],
            )?;
        }
        Ok(())
    }

    fn repair_pending_path_index_derived_mirror(
        &self,
        repair: &PendingPathIndexDerivedRepair,
        published_db_dir: Option<&Path>,
    ) -> Result<bool> {
        let Some(reference) = self.try_get_ref(&repair.ref_name)? else {
            return Ok(true);
        };
        if reference.root_id != repair.new_root || reference.change_id != repair.new_change {
            return Ok(true);
        }
        let Some(lane_name) = repair.ref_name.strip_prefix(LANE_REF_PREFIX) else {
            return Ok(true);
        };
        let branch = match self.lane_branch(lane_name) {
            Ok(branch) => branch,
            Err(Error::RefNotFound(_)) => return Ok(true),
            Err(err) => return Err(err),
        };
        if branch.status == "removed"
            || branch.ref_name != repair.ref_name
            || branch.head_root != repair.new_root
            || branch.head_change != repair.new_change
        {
            return Ok(true);
        }
        let lane = match self.lane_record(&branch.lane_id) {
            Ok(lane) => lane,
            Err(Error::RefNotFound(_)) => return Ok(true),
            Err(err) => return Err(err),
        };

        match repair.repair_kind.as_str() {
            PENDING_REPAIR_LANE_MANIFEST => {
                let Some(workdir) = branch.workdir.as_deref() else {
                    return Ok(true);
                };
                let path = self
                    .lane_layered_clean_manifest_path(&branch)?
                    .unwrap_or_else(|| {
                        Path::new(workdir)
                            .join(".trail")
                            .join("workdir-manifest.json")
                    });
                let path = published_db_dir
                    .and_then(|published| path.strip_prefix(published).ok())
                    .map_or(path.clone(), |relative| self.db_dir.join(relative));
                Ok(self.repair_clean_workdir_manifest_root_mirror(
                    &path,
                    &repair.old_root,
                    &repair.new_root,
                ))
            }
            PENDING_REPAIR_WORKSPACE_CHECKPOINT => {
                let Some(view) = self.lane_workspace_view(&lane.name)? else {
                    return Ok(true);
                };
                if view.checkpoint_root.as_ref() != Some(&repair.new_root) {
                    return Ok(true);
                }
                let current_meta = self.workspace_view_paths_for_lane_name(&lane.name).meta_dir;
                if Path::new(&view.meta_dir) != current_meta {
                    return Ok(false);
                }
                Ok(self.repair_workspace_checkpoint_marker_mirror(
                    &current_meta.join("clean-checkpoint.json"),
                    &view,
                    repair,
                ))
            }
            unknown => Err(Error::Corrupt(format!(
                "pending path-index derived repair has unknown kind `{unknown}`"
            ))),
        }
    }

    fn repair_workspace_checkpoint_marker_mirror(
        &self,
        path: &Path,
        view: &LaneWorkspaceViewReport,
        repair: &PendingPathIndexDerivedRepair,
    ) -> bool {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        };
        let mut marker: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(marker) => marker,
            Err(_) => return remove_path_index_derived_mirror(path),
        };
        if marker["view_id"].as_str() != Some(view.view_id.as_str())
            || marker["journal_sequence"].as_u64() != Some(view.checkpoint_seq)
        {
            return remove_path_index_derived_mirror(path);
        }
        let marker_root = marker["root_id"].as_str();
        if marker_root == Some(repair.new_root.0.as_str())
            && marker["operation"].as_str() == Some(repair.new_change.0.as_str())
        {
            return true;
        }
        if marker_root != Some(repair.old_root.0.as_str()) {
            return remove_path_index_derived_mirror(path);
        }
        marker["root_id"] = serde_json::Value::String(repair.new_root.0.clone());
        marker["operation"] = serde_json::Value::String(repair.new_change.0.clone());
        match serde_json::to_vec_pretty(&marker) {
            Ok(bytes) if write_file_atomic(path, &bytes, false).is_ok() => true,
            _ => remove_path_index_derived_mirror(path),
        }
    }

    fn reconcile_live_ref_files_best_effort(&self) {
        let Ok(references) = self.all_refs() else {
            return;
        };
        for reference in references {
            let _ = write_ref_file(
                &self.db_dir,
                &reference.name,
                &reference.change_id,
                &reference.root_id,
                &reference.operation_id,
                reference.generation,
            );
        }
    }

    fn clean_git_mapping_sources_for_path_index_repair(
        &self,
        reference: &RefRecord,
    ) -> Result<Vec<CleanGitMappingRepairSource>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT direction, branch, git_head FROM git_mappings \
             WHERE crab_root = ?1 AND git_dirty = 0 \
             ORDER BY direction, branch, git_head",
        )?;
        let rows = stmt.query_map(params![reference.root_id.0], |row| {
            Ok(CleanGitMappingRepairSource {
                direction: row.get(0)?,
                branch: row.get(1)?,
                git_head: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn preflight_lane_path_index_repair(
        &self,
        reference: &RefRecord,
        lane_name: &str,
    ) -> Result<PreparedLaneRepair> {
        let branch = self.lane_branch(lane_name)?;
        if branch.ref_name != reference.name
            || branch.head_change != reference.change_id
            || branch.head_root != reference.root_id
        {
            return Err(Error::Corrupt(format!(
                "lane branch {} does not match its mutable ref head before path-index repair",
                reference.name
            )));
        }
        let layered_manifest_path = self.lane_layered_clean_manifest_path(&branch)?;
        let retarget_clean_manifest = if let Some(workdir) = &branch.workdir {
            self.preflight_clean_workdir_manifest_root_retarget(
                Path::new(workdir),
                layered_manifest_path.as_deref(),
                &reference.root_id,
            )?
        } else {
            false
        };

        let lane = self.lane_record(&branch.lane_id)?;
        let mut checkpoint_view_id = None;
        let mut retarget_checkpoint_marker = false;
        if let Some(view) = self.lane_workspace_view(&lane.name)? {
            if view.checkpoint_root.as_ref() == Some(&reference.root_id) {
                checkpoint_view_id = Some(view.view_id.clone());
                let path = Path::new(&view.meta_dir).join("clean-checkpoint.json");
                match fs::read(&path) {
                    Ok(bytes) => {
                        let marker: serde_json::Value =
                            serde_json::from_slice(&bytes).map_err(|err| {
                                Error::Corrupt(format!(
                                    "workspace checkpoint marker `{}` cannot be retargeted: {err}",
                                    path.display()
                                ))
                            })?;
                        if marker["view_id"].as_str() != Some(view.view_id.as_str())
                            || marker["root_id"].as_str() != Some(reference.root_id.0.as_str())
                            || marker["journal_sequence"].as_u64() != Some(view.checkpoint_seq)
                        {
                            return Err(Error::Corrupt(format!(
                                "workspace checkpoint marker `{}` does not match its clean lane baseline",
                                path.display()
                            )));
                        }
                        retarget_checkpoint_marker = true;
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(Error::Io(err)),
                }
            }
        }
        Ok(PreparedLaneRepair {
            branch,
            retarget_clean_manifest,
            checkpoint_view_id,
            retarget_checkpoint_marker,
        })
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

#[cfg(test)]
mod path_index_rebuild_tests {
    use super::*;
    use std::process::Stdio;

    fn publish_legacy_root(db: &Trail, head: &RefRecord) -> (ObjectId, ChangeId) {
        let mut legacy: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        assert!(legacy.case_fold_map_root.take().is_some());
        let legacy_root_id = db
            .put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &legacy)
            .unwrap();
        let change_id = db
            .allocate_change_id("trail-test", "legacy-path-index")
            .unwrap();
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::ManualCheckpoint,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: legacy_root_id.clone(),
            branch: head.name.clone(),
            actor: Actor::system(),
            session_id: None,
            message: Some("Simulate legacy root".to_string()),
            changes: Vec::new(),
            created_at: now_ts(),
        };
        let operation_id = db.store_operation(&operation).unwrap();
        db.advance_ref_cas(head, &change_id, &legacy_root_id, &operation_id)
            .unwrap();
        (legacy_root_id, change_id)
    }

    fn write_patch(path: &str, content: &str, base_change: &ChangeId) -> PatchDocument {
        serde_json::from_value(serde_json::json!({
            "base_change": base_change.0,
            "edits": [{"op": "write", "path": path, "content": content}]
        }))
        .unwrap()
    }

    #[test]
    fn rebuild_repairs_shared_branch_and_lane_legacy_heads_once() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let modern = db.resolve_branch_ref("main").unwrap();
        let modern_files = db.load_root_files(&modern.root_id).unwrap();
        let (legacy_root_id, legacy_change_id) = publish_legacy_root(&db, &modern);
        db.set_worktree_index_baseline(&legacy_root_id).unwrap();
        db.spawn_lane("legacy-lane", Some("main"), true, None, None)
            .unwrap();

        // Compatibility reads and materialization stay available before repair.
        assert_eq!(db.load_root_files(&legacy_root_id).unwrap(), modern_files);
        assert!(db
            .diff_root_file_summaries(&modern.root_id, &legacy_root_id)
            .unwrap()
            .is_empty());
        let materialized = tempfile::tempdir().unwrap();
        db.materialize_files_at(materialized.path(), &BTreeMap::new(), &modern_files)
            .unwrap();
        assert_eq!(
            fs::read(materialized.path().join("README.md")).unwrap(),
            b"hello\n"
        );

        let object_count_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
            .unwrap();
        let operation_count_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
            .unwrap();
        let prolly_count_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row.get(0))
            .unwrap();
        let err = db
            .apply_lane_patch(
                "legacy-lane",
                write_patch("new.txt", "new\n", &legacy_change_id),
            )
            .unwrap_err();
        assert!(matches!(err, Error::PathIndexRequired(_)));
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM objects", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            object_count_before
        );
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM operations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            operation_count_before
        );
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            prolly_count_before
        );

        let report = db.rebuild_indexes().unwrap();
        assert_eq!(report.path_index_repaired_roots.len(), 1);
        assert_eq!(report.path_index_repaired_refs.len(), 2);
        let repaired_branch = db.resolve_branch_ref("main").unwrap();
        let repaired_lane = db.get_ref(&lane_ref("legacy-lane")).unwrap();
        assert_eq!(repaired_branch.root_id, repaired_lane.root_id);
        assert_ne!(repaired_branch.root_id, legacy_root_id);
        let repaired_root: WorktreeRoot = db
            .get_object(WORKTREE_ROOT_KIND, &repaired_branch.root_id)
            .unwrap();
        let legacy_root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &legacy_root_id).unwrap();
        assert!(repaired_root.case_fold_map_root.is_some());
        assert_eq!(repaired_root.path_map_root, legacy_root.path_map_root);
        assert_eq!(
            repaired_root.file_index_map_root,
            legacy_root.file_index_map_root
        );
        assert_eq!(repaired_root.file_count, legacy_root.file_count);
        assert_eq!(repaired_root.total_text_bytes, legacy_root.total_text_bytes);
        assert_eq!(repaired_root.created_by, legacy_root.created_by);
        assert_eq!(
            db.load_root_files(&repaired_branch.root_id).unwrap(),
            modern_files
        );
        assert_eq!(
            db.worktree_index_baseline_root().unwrap(),
            Some(repaired_branch.root_id.clone())
        );
        let lane_row = db.lane_branch("legacy-lane").unwrap();
        assert_eq!(lane_row.head_change, repaired_lane.change_id);
        assert_eq!(lane_row.head_root, repaired_lane.root_id);
        assert!(db
            .preview_lane_workdir_record("legacy-lane")
            .unwrap()
            .changed_paths
            .is_empty());
        for repaired_ref in &report.path_index_repaired_refs {
            let operation = db.operation(&repaired_ref.new_change).unwrap();
            assert!(operation.changes.is_empty());
            assert_eq!(operation.before_root, Some(repaired_ref.old_root.clone()));
            assert_eq!(operation.after_root, repaired_ref.new_root);
            assert_eq!(
                operation.message.as_deref(),
                Some("Rebuild path invariant index")
            );
        }

        let applied = db
            .apply_lane_patch(
                "legacy-lane",
                write_patch("new.txt", "new\n", &repaired_lane.change_id),
            )
            .unwrap();
        assert_eq!(applied.changed_paths.len(), 1);

        let branch_before_second = db.resolve_branch_ref("main").unwrap();
        let lane_before_second = db.get_ref(&lane_ref("legacy-lane")).unwrap();
        let second = db.rebuild_indexes().unwrap();
        assert!(second.path_index_repaired_roots.is_empty());
        assert!(second.path_index_repaired_refs.is_empty());
        let branch_after_second = db.resolve_branch_ref("main").unwrap();
        assert_eq!(
            branch_after_second.change_id,
            branch_before_second.change_id
        );
        assert_eq!(branch_after_second.root_id, branch_before_second.root_id);
        assert_eq!(
            branch_after_second.generation,
            branch_before_second.generation
        );
        let lane_after_second = db.get_ref(&lane_ref("legacy-lane")).unwrap();
        assert_eq!(lane_after_second.change_id, lane_before_second.change_id);
        assert_eq!(lane_after_second.root_id, lane_before_second.root_id);
        assert_eq!(lane_after_second.generation, lane_before_second.generation);
    }

    #[test]
    fn rebuild_preserves_clean_git_mapping_for_repaired_branch() {
        let workspace = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(workspace.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        git(&["init"]);
        git(&["config", "user.email", "trail@example.test"]);
        git(&["config", "user.name", "Trail Test"]);
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        git(&["add", "README.md"]);
        git(&["commit", "-m", "initial"]);
        let git_head = git(&["rev-parse", "HEAD"]);
        Trail::init(workspace.path(), "main", InitImportMode::GitTracked, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let modern = db.resolve_branch_ref("main").unwrap();
        let (legacy_root_id, _) = publish_legacy_root(&db, &modern);
        let historical_change = modern.change_id.clone();
        db.insert_git_mapping_for_state(
            "import",
            "main",
            &historical_change,
            &legacy_root_id,
            Some(git_head.clone()),
            false,
        )
        .unwrap();
        // Historical operations can share a root, and repeated verification
        // can produce duplicate clean mapping facts for that visible state.
        db.insert_git_mapping_for_state(
            "import",
            "main",
            &ChangeId("change_historical_duplicate".to_string()),
            &legacy_root_id,
            Some(git_head.clone()),
            false,
        )
        .unwrap();
        db.insert_git_mapping_for_state(
            "export",
            "release",
            &ChangeId("change_historical_release".to_string()),
            &legacy_root_id,
            Some(git_head.clone()),
            false,
        )
        .unwrap();
        db.spawn_lane("mapped-lane", Some("main"), false, None, None)
            .unwrap();

        let report = db.rebuild_indexes().unwrap();
        let repaired = db.resolve_branch_ref("main").unwrap();
        assert_eq!(report.path_index_repaired_refs.len(), 2);
        assert!(db
            .git_clean_head_matches_root_mapping(&git_head, &repaired.root_id)
            .unwrap());
        let mapping = db
            .git_mappings(20)
            .unwrap()
            .into_iter()
            .find(|mapping| {
                mapping.crab_root == repaired.root_id
                    && mapping.crab_change == repaired.change_id
                    && mapping.git_head.as_deref() == Some(git_head.as_str())
            })
            .unwrap();
        assert_eq!(mapping.direction, "import");
        assert_eq!(mapping.branch, "main");
        assert!(!mapping.git_dirty);
        let copied = db
            .git_mappings(50)
            .unwrap()
            .into_iter()
            .filter(|mapping| {
                mapping.crab_root == repaired.root_id && mapping.crab_change == repaired.change_id
            })
            .collect::<Vec<_>>();
        assert_eq!(
            copied
                .iter()
                .filter(|mapping| mapping.direction == "import" && mapping.branch == "main")
                .count(),
            1
        );
        assert_eq!(
            copied
                .iter()
                .filter(|mapping| mapping.direction == "export" && mapping.branch == "release")
                .count(),
            1
        );

        let repaired_lane = db.get_ref(&lane_ref("mapped-lane")).unwrap();
        let applied = db
            .apply_lane_patch(
                "mapped-lane",
                write_patch("agent.txt", "agent\n", &repaired_lane.change_id),
            )
            .unwrap();
        db.agent_mark_reviewed("mapped-lane", None).unwrap();
        let range = format!("{}..{}", repaired_lane.change_id.0, applied.operation.0);
        db.reset_git_handoff_metrics();
        let exported = db
            .git_export_commit_mapped(
                &range,
                "mapped delta after index repair",
                Some(GitState {
                    head: Some(git_head),
                    dirty: false,
                }),
            )
            .unwrap();
        assert_eq!(exported.performance.export_mode, "mapped_delta");
        assert_eq!(exported.performance.full_root_file_count, 0);
    }

    #[test]
    fn equivalent_repaired_root_reuses_cross_process_clean_daemon_snapshot_but_not_dirty_state() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let mut daemon_db = Trail::open(workspace.path()).unwrap();
        let modern = daemon_db.resolve_branch_ref("main").unwrap();
        let (legacy_root, _) = publish_legacy_root(&daemon_db, &modern);
        daemon_db.enable_daemon_worktree_cache().unwrap();
        // This regression exercises persisted snapshot reuse across handles,
        // not asynchronous OS event delivery. Stop the watcher after warmup
        // so delayed backend events cannot turn the fixed baseline fixture
        // dirty while the index-repair assertions are running.
        drop(
            daemon_db
                .daemon_worktree_cache
                .as_mut()
                .unwrap()
                .watcher
                .take(),
        );
        let persisted_reader = Trail::open(workspace.path()).unwrap();
        let persisted_snapshot = persisted_reader.daemon_worktree_snapshot();
        assert!(
            matches!(
                persisted_snapshot,
                Some(DaemonWorktreeSnapshot::Clean { root_id: Some(ref root), .. }) if root == &legacy_root
            ),
            "unexpected persisted daemon snapshot: {persisted_snapshot:?}"
        );
        drop(persisted_reader);

        let mut repair_db = Trail::open(workspace.path()).unwrap();
        repair_db.rebuild_indexes().unwrap();
        let repaired = repair_db.resolve_branch_ref("main").unwrap();
        assert_ne!(repaired.root_id, legacy_root);
        let repaired_snapshot = repair_db.daemon_worktree_snapshot();
        assert!(
            matches!(
                repaired_snapshot,
                Some(DaemonWorktreeSnapshot::Clean { root_id: Some(ref root), .. }) if root == &legacy_root
            ),
            "unexpected repaired daemon snapshot: {repaired_snapshot:?}"
        );
        repair_db
            .conn
            .execute("DELETE FROM worktree_file_index", [])
            .unwrap();

        let clean = repair_db.status(None).unwrap();

        assert_eq!(clean.worktree_state, WorktreeState::Clean);
        assert_eq!(
            repair_db
                .conn
                .query_row("SELECT COUNT(*) FROM worktree_file_index", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0,
            "clean equivalent daemon state must avoid the O(N) worktree refresh"
        );

        fs::write(workspace.path().join("README.md"), "dirty\n").unwrap();
        {
            let cache = daemon_db.daemon_worktree_cache.as_ref().unwrap();
            let mut state = cache.state.lock().unwrap();
            state.dirty_paths.insert("README.md".to_string());
            state.generation = state.generation.saturating_add(1);
        }
        let dirty = daemon_db.status(None).unwrap();
        assert_eq!(dirty.worktree_state, WorktreeState::DirtyTracked);
        assert_eq!(dirty.changed_paths.len(), 1);
    }

    #[test]
    fn empty_and_modern_roots_do_not_publish_path_index_repairs() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let head = db.resolve_branch_ref("main").unwrap();

        let report = db.rebuild_indexes().unwrap();

        assert!(report.path_index_repaired_roots.is_empty());
        assert!(report.path_index_repaired_refs.is_empty());
        let after = db.resolve_branch_ref("main").unwrap();
        assert_eq!(after.change_id, head.change_id);
        assert_eq!(after.root_id, head.root_id);
        assert_eq!(after.generation, head.generation);
    }

    #[test]
    fn rebuild_repairs_distinct_noncurrent_branch_and_lane_roots() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.create_branch("other", Some("main")).unwrap();
        fs::write(workspace.path().join("other.txt"), "other\n").unwrap();
        db.record(
            Some("other"),
            Some("different root".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
        let main = db.resolve_branch_ref("main").unwrap();
        let other = db.resolve_branch_ref("other").unwrap();
        assert_ne!(main.root_id, other.root_id);
        let (main_legacy, _) = publish_legacy_root(&db, &main);
        let (other_legacy, _) = publish_legacy_root(&db, &other);
        assert_ne!(main_legacy, other_legacy);
        db.spawn_lane("other-lane", Some("other"), false, None, None)
            .unwrap();

        let report = db.rebuild_indexes().unwrap();

        assert_eq!(report.path_index_repaired_roots.len(), 2);
        assert_eq!(report.path_index_repaired_refs.len(), 3);
        let roots = report
            .path_index_repaired_roots
            .iter()
            .map(|repair| repair.old_root.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(roots, BTreeSet::from([main_legacy, other_legacy]));
        assert_eq!(
            db.resolve_branch_ref("other").unwrap().root_id,
            db.get_ref(&lane_ref("other-lane")).unwrap().root_id
        );
    }

    fn legacy_root_with_path_keys(
        db: &Trail,
        source_root_id: &ObjectId,
        keys: Vec<Vec<u8>>,
    ) -> ObjectId {
        let source: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, source_root_id).unwrap();
        let source_files = db.load_root_files(source_root_id).unwrap();
        let entry = source_files.values().next().unwrap();
        let file_count = keys.len() as u64;
        let mut builder = SortedBatchBuilder::new(db.store.clone(), root_map_prolly_config());
        for key in keys {
            builder.add(key, cbor(entry).unwrap()).unwrap();
        }
        let path_tree = builder.build().unwrap();
        let legacy = WorktreeRoot {
            path_map_root: tree_root_hex(&path_tree),
            case_fold_map_root: None,
            file_count,
            ..source
        };
        db.put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &legacy)
            .unwrap()
    }

    fn assert_rebuild_preflight_does_not_publish(db: &mut Trail, protected_ref: &RefRecord) {
        fn count(db: &Trail, table: &str) -> i64 {
            db.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        }
        let before_ref = db.get_ref(&protected_ref.name).unwrap();
        let objects = count(db, "objects");
        let prolly_nodes = count(db, "prolly_nodes");
        let operations = count(db, "operations");
        let git_mappings = count(db, "git_mappings");
        let lane_branches = count(db, "lane_branches");

        assert!(db.rebuild_indexes().is_err());

        let after_ref = db.get_ref(&protected_ref.name).unwrap();
        assert_eq!(after_ref.change_id, before_ref.change_id);
        assert_eq!(after_ref.root_id, before_ref.root_id);
        assert_eq!(after_ref.generation, before_ref.generation);
        assert_eq!(count(db, "objects"), objects);
        assert_eq!(count(db, "prolly_nodes"), prolly_nodes);
        assert_eq!(count(db, "operations"), operations);
        assert_eq!(count(db, "git_mappings"), git_mappings);
        assert_eq!(count(db, "lane_branches"), lane_branches);
    }

    #[test]
    fn corrupt_later_collision_root_does_not_advance_earlier_valid_ref() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.create_branch("a-good", Some("main")).unwrap();
        let good = db.resolve_branch_ref("a-good").unwrap();
        publish_legacy_root(&db, &good);
        let good_legacy = db.resolve_branch_ref("a-good").unwrap();
        let main = db.resolve_branch_ref("main").unwrap();
        let bad_root = legacy_root_with_path_keys(
            &db,
            &main.root_id,
            vec![b"README.md".to_vec(), b"readme.md".to_vec()],
        );
        db.set_ref(
            &branch_ref("z-bad"),
            &main.change_id,
            &bad_root,
            &main.operation_id,
        )
        .unwrap();

        assert_rebuild_preflight_does_not_publish(&mut db, &good_legacy);
    }

    #[test]
    fn malformed_legacy_path_key_does_not_publish_any_ref_metadata() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.create_branch("a-good", Some("main")).unwrap();
        let good = db.resolve_branch_ref("a-good").unwrap();
        publish_legacy_root(&db, &good);
        let good_legacy = db.resolve_branch_ref("a-good").unwrap();
        let main = db.resolve_branch_ref("main").unwrap();
        let bad_root = legacy_root_with_path_keys(&db, &main.root_id, vec![vec![0xff, 0xfe]]);
        db.set_ref(
            &branch_ref("z-malformed"),
            &main.change_id,
            &bad_root,
            &main.operation_id,
        )
        .unwrap();

        assert_rebuild_preflight_does_not_publish(&mut db, &good_legacy);
    }

    #[test]
    fn corrupt_later_lane_manifest_does_not_create_maintenance_operation_or_advance_refs() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let modern = db.resolve_branch_ref("main").unwrap();
        publish_legacy_root(&db, &modern);
        db.spawn_lane("z-corrupt", Some("main"), true, None, None)
            .unwrap();

        let main_before = db.resolve_branch_ref("main").unwrap();
        let lane_before = db.get_ref(&lane_ref("z-corrupt")).unwrap();
        let lane_row_before = db.lane_branch("z-corrupt").unwrap();
        let manifest_path = Path::new(lane_row_before.workdir.as_ref().unwrap())
            .join(".trail")
            .join("workdir-manifest.json");
        assert!(manifest_path.is_file());
        fs::write(&manifest_path, b"{not-json").unwrap();
        let operation_objects_before: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM objects WHERE kind = ?1",
                params![OPERATION_KIND],
                |row| row.get(0),
            )
            .unwrap();

        let err = db.rebuild_indexes().unwrap_err();

        assert!(matches!(err, Error::Corrupt(message) if message.contains("cannot be retargeted")));
        let main_after = db.resolve_branch_ref("main").unwrap();
        let lane_after = db.get_ref(&lane_ref("z-corrupt")).unwrap();
        let lane_row_after = db.lane_branch("z-corrupt").unwrap();
        assert_eq!(main_after.change_id, main_before.change_id);
        assert_eq!(main_after.root_id, main_before.root_id);
        assert_eq!(main_after.generation, main_before.generation);
        assert_eq!(lane_after.change_id, lane_before.change_id);
        assert_eq!(lane_after.root_id, lane_before.root_id);
        assert_eq!(lane_after.generation, lane_before.generation);
        assert_eq!(lane_row_after.head_change, lane_row_before.head_change);
        assert_eq!(lane_row_after.head_root, lane_row_before.head_root);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM objects WHERE kind = ?1",
                    params![OPERATION_KIND],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            operation_objects_before
        );
    }

    #[test]
    fn path_index_rebuild_publication_crash_helper() {
        let Some(workspace) = std::env::var_os("TRAIL_TEST_PATH_INDEX_CRASH_WORKSPACE") else {
            return;
        };
        let mut db = Trail::open(PathBuf::from(workspace)).unwrap();
        let _ = db.rebuild_indexes();
        panic!("path-index rebuild crash helper passed its requested crash point");
    }

    fn wait_for_path_index_crash_handshake(
        child: &mut std::process::Child,
        ready: &Path,
        phase: &str,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if ready.is_file() {
                return;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("path-index crash helper exited at {phase} before handshake: {status}");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("timed out waiting for path-index crash handshake at {phase}");
    }

    #[test]
    fn publication_commit_survives_crash_with_operation_graph_and_lane_mirror_recovery() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let modern = db.resolve_branch_ref("main").unwrap();
        publish_legacy_root(&db, &modern);
        let spawned = db
            .spawn_lane("crash-lane", Some("main"), true, None, None)
            .unwrap();
        let lane_before = db.get_ref(&lane_ref("crash-lane")).unwrap();
        let lane_row = db.lane_branch("crash-lane").unwrap();
        let workdir = PathBuf::from(spawned.workdir.unwrap());
        let manifest_path = workdir.join(".trail/workdir-manifest.json");
        let lane = db.lane_record(&lane_row.lane_id).unwrap();
        let view = db
            .create_workspace_view(
                &lane.lane_id,
                &lane_before.change_id,
                &lane_before.root_id,
                "test-cow",
                &workdir,
            )
            .unwrap();
        let marker_path = Path::new(&view.meta_dir).join("clean-checkpoint.json");
        write_file_atomic(
            &marker_path,
            &serde_json::to_vec_pretty(&serde_json::json!({
                "view_id": view.view_id,
                "operation": lane_before.change_id.0,
                "root_id": lane_before.root_id.0,
                "journal_sequence": view.checkpoint_seq,
            }))
            .unwrap(),
            false,
        )
        .unwrap();
        drop(db);

        let phase = "path_index_rebuild_after_publication_commit";
        let ready = workspace.path().join(".trail/tmp/path-index-commit.ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "db::storage::lifecycle::rebuild::path_index_rebuild_tests::path_index_rebuild_publication_crash_helper",
                "--nocapture",
            ])
            .env("RUST_TEST_THREADS", "1")
            .env("TRAIL_TEST_CRASH_AT", phase)
            .env("TRAIL_TEST_CRASH_READY", &ready)
            .env("TRAIL_TEST_PATH_INDEX_CRASH_WORKSPACE", workspace.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_for_path_index_crash_handshake(&mut child, &ready, phase);
        child.kill().unwrap();
        let _ = child.wait().unwrap();

        let reopened = Trail::open(workspace.path()).unwrap();
        let lane_after = reopened.get_ref(&lane_ref("crash-lane")).unwrap();
        assert_ne!(lane_after.change_id, lane_before.change_id);
        let operation = reopened.operation(&lane_after.change_id).unwrap();
        assert_eq!(operation.parents, vec![lane_before.change_id.clone()]);
        assert_eq!(
            reopened.parents(&lane_after.change_id).unwrap(),
            vec![lane_before.change_id]
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["root_id"], lane_after.root_id.0);
        let marker: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        assert_eq!(marker["root_id"], lane_after.root_id.0);
        assert_eq!(marker["operation"], lane_after.change_id.0);
        let pending: i64 = reopened
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pending_path_index_derived_repairs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
    }

    #[test]
    fn failed_pending_manifest_repair_stays_durable_and_retries_after_invalidation() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let spawned = db
            .spawn_lane("pending-lane", Some("main"), true, None, None)
            .unwrap();
        let head = db.get_ref(&lane_ref("pending-lane")).unwrap();
        let manifest = PathBuf::from(spawned.workdir.unwrap())
            .join(".trail")
            .join("workdir-manifest.json");
        fs::remove_file(&manifest).unwrap();
        fs::create_dir(&manifest).unwrap();
        db.conn
            .execute(
                "INSERT INTO pending_path_index_derived_repairs \
                 (ref_name, repair_kind, old_root, new_root, new_change, created_at) \
                 VALUES (?1, 'lane_manifest', ?2, ?2, ?3, ?4)",
                params![head.name, head.root_id.0, head.change_id.0, now_ts()],
            )
            .unwrap();

        db.rebuild_indexes().unwrap();
        let pending = |db: &Trail| {
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM pending_path_index_derived_repairs",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        };
        assert_eq!(pending(&db), 1);

        fs::remove_dir(&manifest).unwrap();
        db.rebuild_indexes().unwrap();
        assert_eq!(pending(&db), 0);
    }

    #[test]
    fn open_with_empty_pending_queue_does_not_contend_with_writer_lock() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let _lock = db.acquire_write_lock().unwrap();

        Trail::open(workspace.path()).unwrap();
    }

    #[test]
    fn pending_repair_for_removed_lane_scope_clears_without_using_stale_paths() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane("removed-lane", Some("main"), false, None, None)
            .unwrap();
        let head = db.get_ref(&lane_ref("removed-lane")).unwrap();
        db.conn
            .execute(
                "INSERT INTO pending_path_index_derived_repairs \
                 (ref_name, repair_kind, old_root, new_root, new_change, created_at) \
                 VALUES (?1, 'workspace_checkpoint', ?2, ?2, ?3, ?4)",
                params![head.name, head.root_id.0, head.change_id.0, now_ts()],
            )
            .unwrap();
        db.conn
            .execute("DELETE FROM refs WHERE name = ?1", params![head.name])
            .unwrap();
        db.conn
            .execute(
                "DELETE FROM lane_branches WHERE ref_name = ?1",
                params![head.name],
            )
            .unwrap();

        db.rebuild_indexes().unwrap();

        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM pending_path_index_derived_repairs",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn rebuild_retains_paths_for_at_most_one_distinct_legacy_root() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let modern = db.resolve_branch_ref("main").unwrap();
        let one = legacy_root_with_path_keys(&db, &modern.root_id, vec![b"one.txt".to_vec()]);
        let two = legacy_root_with_path_keys(
            &db,
            &modern.root_id,
            vec![b"one.txt".to_vec(), b"two.txt".to_vec()],
        );
        let three = legacy_root_with_path_keys(
            &db,
            &modern.root_id,
            vec![
                b"one.txt".to_vec(),
                b"three.txt".to_vec(),
                b"two.txt".to_vec(),
            ],
        );
        for (name, root) in [("main", one), ("two", two), ("three", three)] {
            db.set_ref(
                &branch_ref(name),
                &modern.change_id,
                &root,
                &modern.operation_id,
            )
            .unwrap();
        }

        reset_path_index_rebuild_memory_metrics();
        let report = db.rebuild_indexes().unwrap();
        let metrics = path_index_rebuild_memory_metrics();

        assert_eq!(report.path_index_repaired_roots.len(), 3);
        assert!(metrics.observed_paths >= 6);
        assert_eq!(metrics.live_paths, 0);
        assert!(
            metrics.peak_live_paths <= 3,
            "peak retained paths {} exceeded largest root",
            metrics.peak_live_paths
        );
    }

    #[test]
    fn rebuild_reconciles_stale_ref_file_even_without_root_repairs() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let head = db.resolve_branch_ref("main").unwrap();
        let ref_path = db.db_dir.join(&head.name);
        fs::write(&ref_path, br#"{"root_id":"stale"}"#).unwrap();

        let report = db.rebuild_indexes().unwrap();

        assert!(report.path_index_repaired_refs.is_empty());
        let mirrored: serde_json::Value =
            serde_json::from_slice(&fs::read(ref_path).unwrap()).unwrap();
        assert_eq!(mirrored["root_id"], head.root_id.0);
        assert_eq!(mirrored["change_id"], head.change_id.0);
        assert_eq!(mirrored["generation"], head.generation);
    }
}
