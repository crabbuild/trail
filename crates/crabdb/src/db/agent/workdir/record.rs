use super::*;

impl CrabDb {
    pub fn record_agent_workdir(
        &mut self,
        agent: &str,
        message: Option<String>,
    ) -> Result<AgentRecordReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        };
        let workdir_path = PathBuf::from(&workdir);
        if !workdir_path.is_dir() {
            return Err(Error::WorkspaceNotFound(workdir_path));
        }
        let head = self.get_ref(&branch.ref_name)?;
        let sparse_paths = self.sparse_workdir_paths(&workdir_path)?;
        let is_sparse = sparse_paths.is_some();
        let cached_status = self.cached_workdir_manifest_status(&workdir_path, &head.root_id)?;
        if matches!(cached_status, CachedWorkdirManifestStatus::Clean) {
            return Ok(AgentRecordReport {
                agent_id: branch.agent_id,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }

        let actor = Actor::agent(agent);
        let change_id = self.allocate_change_id(&actor.id, "agent_record")?;
        let (built, materialized_paths, previous_files, clean_disk_manifest) = match cached_status {
            CachedWorkdirManifestStatus::Clean => unreachable!(),
            CachedWorkdirManifestStatus::Dirty {
                disk_manifest,
                candidate_paths,
            } => {
                let (summaries, previous_files, use_disk_manifest_for_clean) =
                    if let Some(mut selected_paths) = sparse_paths.clone() {
                        selected_paths.extend(disk_manifest.keys().cloned());
                        selected_paths.sort();
                        selected_paths.dedup();
                        let previous_files =
                            self.load_root_files_for_selections(&head.root_id, &selected_paths)?;
                        let summaries = self.diff_file_maps_to_manifest_for_paths(
                            &previous_files,
                            &disk_manifest,
                            &selected_paths,
                        );
                        (summaries, previous_files, false)
                    } else if let Some(candidate_paths) = candidate_paths {
                        let previous_files =
                            self.load_root_files_for_paths(&head.root_id, &candidate_paths)?;
                        let summaries = self.diff_file_maps_to_manifest_for_paths(
                            &previous_files,
                            &disk_manifest,
                            &candidate_paths,
                        );
                        (summaries, previous_files, true)
                    } else {
                        let previous_files = self.load_root_files(&head.root_id)?;
                        let summaries =
                            self.diff_file_maps_to_manifest(&previous_files, &disk_manifest);
                        (summaries, previous_files, false)
                    };
                let materialized_paths = disk_manifest.keys().cloned().collect::<Vec<_>>();
                if summaries.is_empty() {
                    if is_sparse {
                        self.write_sparse_workdir_manifest(
                            &workdir_path,
                            materialized_paths.iter(),
                        )?;
                    }
                    if use_disk_manifest_for_clean {
                        self.write_clean_workdir_manifest_from_disk_manifest(
                            &workdir_path,
                            &head.root_id,
                            &disk_manifest,
                            materialized_paths.iter(),
                        )?;
                    } else {
                        self.write_clean_workdir_manifest(
                            &workdir_path,
                            &head.root_id,
                            &previous_files,
                            materialized_paths.iter(),
                        )?;
                    }
                    return Ok(AgentRecordReport {
                        agent_id: branch.agent_id,
                        operation: None,
                        root_id: head.root_id,
                        changed_paths: Vec::new(),
                    });
                }
                let selected_paths = summaries
                    .iter()
                    .map(|summary| summary.path.clone())
                    .collect::<Vec<_>>();
                let disk_files = self.scan_files_under_for_paths(&workdir_path, &selected_paths)?;
                let built = self.build_root_for_selected_disk_files_incremental(
                    &head.root_id,
                    &previous_files,
                    &disk_files,
                    &selected_paths,
                    &change_id,
                )?;
                let clean_disk_manifest = use_disk_manifest_for_clean.then_some(disk_manifest);
                (
                    built,
                    materialized_paths,
                    previous_files,
                    clean_disk_manifest,
                )
            }
            CachedWorkdirManifestStatus::Missing => {
                let disk_files = self.scan_files_under(&workdir_path)?;
                let (built, previous_files) = if let Some(mut selected_paths) = sparse_paths.clone()
                {
                    selected_paths.extend(disk_files.iter().map(|file| file.path.clone()));
                    selected_paths.sort();
                    selected_paths.dedup();
                    let previous_files =
                        self.load_root_files_for_selections(&head.root_id, &selected_paths)?;
                    let built = self.build_root_for_selected_disk_files_incremental(
                        &head.root_id,
                        &previous_files,
                        &disk_files,
                        &selected_paths,
                        &change_id,
                    )?;
                    (built, previous_files)
                } else {
                    let previous_files = self.load_root_files(&head.root_id)?;
                    let built = self.build_root_from_disk_files(
                        &disk_files,
                        &change_id,
                        Some(&previous_files),
                    )?;
                    (built, previous_files)
                };
                let materialized_paths = if is_sparse {
                    disk_files
                        .iter()
                        .map(|file| file.path.clone())
                        .collect::<Vec<_>>()
                } else {
                    built.files.keys().cloned().collect::<Vec<_>>()
                };
                (built, materialized_paths, previous_files, None)
            }
        };
        let diff = self.diff_file_maps(&previous_files, &built.files)?;
        if diff.changes.is_empty() {
            if is_sparse {
                self.write_sparse_workdir_manifest(&workdir_path, materialized_paths.iter())?;
            }
            if let Some(disk_manifest) = &clean_disk_manifest {
                self.write_clean_workdir_manifest_from_disk_manifest(
                    &workdir_path,
                    &head.root_id,
                    disk_manifest,
                    materialized_paths.iter(),
                )?;
            } else {
                self.write_clean_workdir_manifest(
                    &workdir_path,
                    &head.root_id,
                    &built.files,
                    materialized_paths.iter(),
                )?;
            }
            return Ok(AgentRecordReport {
                agent_id: branch.agent_id,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }
        if let Some(session_id) = &branch.session_id {
            self.ensure_agent_session(&branch.agent_id, session_id, None)?;
        }
        let turn_id = self.open_agent_turn(
            &branch.agent_id,
            branch.session_id.as_deref(),
            &branch.base_change,
            &head.change_id,
            Some(&serde_json::json!({
                "kind": "workdir_record",
                "path_count": diff.summaries.len()
            })),
        )?;

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::AgentRecord,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.ref_name.clone(),
            actor,
            session_id: branch.session_id.clone(),
            message: message.as_deref().map(redact_sensitive_text),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        let message_id = if let Some(message) = message {
            Some(self.store_message(
                "agent",
                &message,
                Some(&branch.agent_id),
                branch.session_id.as_deref(),
                Some(&change_id),
                operation.created_at,
            )?)
        } else {
            None
        };
        self.conn.execute(
            "UPDATE agent_branches SET head_change = ?1, head_root = ?2, updated_at = ?3 WHERE agent_id = ?4",
            params![change_id.0, built.root_id.0, now_ts(), branch.agent_id],
        )?;
        if is_sparse {
            self.write_sparse_workdir_manifest(&workdir_path, materialized_paths.iter())?;
        }
        if let Some(disk_manifest) = &clean_disk_manifest {
            self.write_clean_workdir_manifest_from_disk_manifest(
                &workdir_path,
                &built.root_id,
                disk_manifest,
                materialized_paths.iter(),
            )?;
        } else {
            self.write_clean_workdir_manifest(
                &workdir_path,
                &built.root_id,
                &built.files,
                materialized_paths.iter(),
            )?;
        }
        self.insert_agent_event_with_context(
            &branch.agent_id,
            branch.session_id.as_deref(),
            Some(&turn_id),
            "workdir_recorded",
            Some(&change_id),
            message_id.as_ref(),
            &serde_json::json!({
                "workdir": workdir,
                "root_id": built.root_id.0.clone(),
                "session_id": branch.session_id.clone(),
                "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
            }),
        )?;
        self.finish_agent_turn(&turn_id, "completed", Some(&change_id))?;
        Ok(AgentRecordReport {
            agent_id: branch.agent_id,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    pub fn watch_agent_workdir(
        &mut self,
        agent: &str,
        message: Option<String>,
        interval: Duration,
        iterations: Option<u64>,
    ) -> Result<AgentWatchReport> {
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        if branch.workdir.is_none() {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        }
        let mut report = AgentWatchReport {
            agent_id: branch.agent_id,
            iterations: 0,
            recorded_operations: Vec::new(),
            changed_paths: Vec::new(),
        };
        loop {
            let record = self.record_agent_workdir(agent, message.clone())?;
            report.iterations += 1;
            if let Some(operation) = record.operation {
                report.recorded_operations.push(operation);
                report.changed_paths.extend(record.changed_paths);
            }
            if iterations.is_some_and(|limit| report.iterations >= limit) {
                break;
            }
            std::thread::sleep(interval);
        }
        Ok(report)
    }
}
