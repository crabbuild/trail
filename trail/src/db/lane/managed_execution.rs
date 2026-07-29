use super::*;
use std::any::Any;

#[doc(hidden)]
pub struct ManagedExecutionContext {
    pub execution_id: String,
    pub surface: String,
    pub lane: String,
    pub lane_id: String,
    pub command_fingerprint: String,
    pub source_root: ObjectId,
    pub view: Option<LaneWorkspaceViewReport>,
    pub workdir: PathBuf,
    pub environment: Vec<(String, String)>,
    pub environment_generation: Option<String>,
    mount: Option<Box<dyn Any + Send>>,
    phases: Vec<ManagedExecutionPhaseReceipt>,
    #[cfg(test)]
    injected_disposal_error: Option<String>,
}

impl Trail {
    #[doc(hidden)]
    pub fn prepare_managed_lane_execution(
        &self,
        lane: &str,
        surface: &str,
        command: &[String],
    ) -> Result<ManagedExecutionContext> {
        if command.is_empty() {
            return Err(Error::InvalidInput(format!("{surface} requires a command")));
        }
        let execution_id = managed_execution_id(lane, surface, command)?;
        let command_fingerprint = sha256_hex(&serde_json::to_vec(command)?);
        let mut phases = Vec::new();

        let branch = match self.lane_branch(lane) {
            Ok(branch) => branch,
            Err(error) => {
                self.record_managed_execution_phase(
                    None,
                    &execution_id,
                    surface,
                    &command_fingerprint,
                    "resolve",
                    "failed",
                    Some(&error.to_string()),
                    None,
                )?;
                return Err(error);
            }
        };
        if branch.status == "archived" {
            let error = Error::InvalidInput(format!(
                "lane `{lane}` is archived; run `trail lane unarchive {lane}` before execution"
            ));
            self.record_managed_execution_phase(
                Some(&branch.lane_id),
                &execution_id,
                surface,
                &command_fingerprint,
                "resolve",
                "failed",
                Some(&error.to_string()),
                None,
            )?;
            return Err(error);
        }
        let workdir = branch
            .workdir
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "lane `{lane}` does not have a materialized workdir"
                ))
            })?;
        let head = self.get_ref(&branch.ref_name)?;
        self.push_managed_execution_phase(
            &mut phases,
            &branch.lane_id,
            &execution_id,
            surface,
            &command_fingerprint,
            "resolve",
            "succeeded",
            None,
            None,
        )?;

        let view = self.lane_workspace_view(lane)?;
        let discovered = match self.discover_workspace_environment(lane, None) {
            Ok(report) if report.conflicts.is_empty() => report,
            Ok(report) => {
                let error = Error::InvalidInput(format!(
                    "environment discovery found {} unresolved component identity conflict(s)",
                    report.conflicts.len()
                ));
                self.push_managed_execution_phase(
                    &mut phases,
                    &branch.lane_id,
                    &execution_id,
                    surface,
                    &command_fingerprint,
                    "discover_plan",
                    "failed",
                    Some(&error.to_string()),
                    None,
                )?;
                return Err(error);
            }
            Err(error) => {
                self.push_managed_execution_phase(
                    &mut phases,
                    &branch.lane_id,
                    &execution_id,
                    surface,
                    &command_fingerprint,
                    "discover_plan",
                    "failed",
                    Some(&error.to_string()),
                    None,
                )?;
                return Err(error);
            }
        };
        if view.is_none() && !discovered.components.is_empty() {
            self.push_managed_execution_phase(
                &mut phases,
                &branch.lane_id,
                &execution_id,
                surface,
                &command_fingerprint,
                "discover_plan",
                "succeeded",
                None,
                Some(serde_json::json!({
                    "component_count": discovered.components.len(),
                    "graph_nodes": 0,
                    "graph_edges": 0,
                })),
            )?;
            let error = Error::InvalidInput(format!(
                "lane `{lane}` declares workspace environments but does not use a layered COW workdir"
            ));
            self.push_managed_execution_phase(
                &mut phases,
                &branch.lane_id,
                &execution_id,
                surface,
                &command_fingerprint,
                "sync_all",
                "failed",
                Some(&error.to_string()),
                None,
            )?;
            return Err(error);
        }
        let graph = match self.workspace_environment_graph(lane, None) {
            Ok(graph) => graph,
            Err(error) => {
                self.push_managed_execution_phase(
                    &mut phases,
                    &branch.lane_id,
                    &execution_id,
                    surface,
                    &command_fingerprint,
                    "discover_plan",
                    "failed",
                    Some(&error.to_string()),
                    None,
                )?;
                return Err(error);
            }
        };
        self.push_managed_execution_phase(
            &mut phases,
            &branch.lane_id,
            &execution_id,
            surface,
            &command_fingerprint,
            "discover_plan",
            "succeeded",
            None,
            Some(serde_json::json!({
                "component_count": discovered.components.len(),
                "graph_nodes": graph.total_nodes,
                "graph_edges": graph.total_edges,
            })),
        )?;

        let existing_environment = if view.is_some() {
            self.workspace_environment_status(lane)?
        } else {
            Vec::new()
        };
        let desired_environment = graph
            .nodes
            .iter()
            .map(|node| (node.component_id.clone(), node.component_key.clone()))
            .collect::<BTreeMap<_, _>>();
        let has_environment = !desired_environment.is_empty() || !existing_environment.is_empty();
        let must_sync = has_environment
            && !managed_environment_is_current(&desired_environment, &existing_environment);
        if must_sync && view.is_none() {
            let error = Error::InvalidInput(format!(
                "lane `{lane}` declares workspace environments but does not use a layered COW workdir"
            ));
            self.push_managed_execution_phase(
                &mut phases,
                &branch.lane_id,
                &execution_id,
                surface,
                &command_fingerprint,
                "sync_all",
                "failed",
                Some(&error.to_string()),
                None,
            )?;
            return Err(error);
        }
        if must_sync {
            if let Err(error) = self.sync_all_workspace_environments(lane, None) {
                self.push_managed_execution_phase(
                    &mut phases,
                    &branch.lane_id,
                    &execution_id,
                    surface,
                    &command_fingerprint,
                    "sync_all",
                    "failed",
                    Some(&error.to_string()),
                    None,
                )?;
                return Err(error);
            }
            self.push_managed_execution_phase(
                &mut phases,
                &branch.lane_id,
                &execution_id,
                surface,
                &command_fingerprint,
                "sync_all",
                "succeeded",
                None,
                None,
            )?;
        } else {
            self.push_managed_execution_phase(
                &mut phases,
                &branch.lane_id,
                &execution_id,
                surface,
                &command_fingerprint,
                "sync_all",
                "skipped",
                None,
                None,
            )?;
        }

        let active_generation = if view.is_some() {
            self.active_environment_generation(lane)?
        } else {
            None
        };
        let has_runtime = active_generation.as_ref().is_some_and(|generation| {
            generation
                .components
                .iter()
                .any(|component| !component.runtime_resources.is_empty())
        });
        if has_runtime {
            if let Err(error) = self.reconcile_workspace_environment_runtime(lane) {
                self.push_managed_execution_phase(
                    &mut phases,
                    &branch.lane_id,
                    &execution_id,
                    surface,
                    &command_fingerprint,
                    "reconcile",
                    "failed",
                    Some(&error.to_string()),
                    None,
                )?;
                let cleanup = self.stop_workspace_environment_runtime(lane);
                if let Err(cleanup_error) = cleanup {
                    return Err(Error::Corrupt(format!(
                        "{error}; additionally failed to stop partially reconciled runtime resources: {cleanup_error}"
                    )));
                }
                return Err(error);
            }
            self.push_managed_execution_phase(
                &mut phases,
                &branch.lane_id,
                &execution_id,
                surface,
                &command_fingerprint,
                "reconcile",
                "succeeded",
                None,
                None,
            )?;
        } else {
            self.push_managed_execution_phase(
                &mut phases,
                &branch.lane_id,
                &execution_id,
                surface,
                &command_fingerprint,
                "reconcile",
                "skipped",
                None,
                None,
            )?;
        }

        let mut mount = if let Some(view) = &view {
            let record = self.lane_record(&branch.lane_id)?;
            let mode = self.lane_workdir_mode_for(&record, &branch)?;
            let mount: Result<Box<dyn Any + Send>> = match mode {
                LaneWorkdirMode::FuseCow => self
                    .mount_fuse_cow_workdir_for_lane(lane)
                    .map(|mount| Box::new(mount) as Box<dyn Any + Send>),
                LaneWorkdirMode::NfsCow => self
                    .mount_nfs_cow_workdir_for_lane(lane)
                    .map(|mount| Box::new(mount) as Box<dyn Any + Send>),
                LaneWorkdirMode::DokanCow => {
                    #[cfg(target_os = "windows")]
                    {
                        self.mount_dokan_cow_workdir_for_lane(lane)
                            .map(|mount| Box::new(mount) as Box<dyn Any + Send>)
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        Err(Error::InvalidInput(
                            "dokan-cow workdirs are currently supported only on Windows"
                                .to_string(),
                        ))
                    }
                }
                _ => Err(Error::Corrupt(format!(
                    "workspace view `{}` is paired with non-layered mode `{}`",
                    view.view_id,
                    mode.as_str()
                ))),
            };
            match mount {
                Ok(mount) => Some(mount),
                Err(error) => {
                    self.push_managed_execution_phase(
                        &mut phases,
                        &branch.lane_id,
                        &execution_id,
                        surface,
                        &command_fingerprint,
                        "mount",
                        "failed",
                        Some(&error.to_string()),
                        None,
                    )?;
                    if has_runtime {
                        let _ = self.stop_workspace_environment_runtime(lane);
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        self.push_managed_execution_phase(
            &mut phases,
            &branch.lane_id,
            &execution_id,
            surface,
            &command_fingerprint,
            "mount",
            if view.is_some() {
                "succeeded"
            } else {
                "skipped"
            },
            None,
            None,
        )?;

        let environment = match view
            .as_ref()
            .map(|view| self.workspace_command_environment(view, &head.root_id))
            .transpose()
        {
            Ok(Some(environment)) => environment,
            Ok(None) => {
                vec![
                    (
                        "TRAIL_WORKSPACE".to_string(),
                        self.workspace_root.to_string_lossy().into_owned(),
                    ),
                    ("TRAIL_LANE".to_string(), branch.lane_id.clone()),
                    ("TRAIL_SOURCE_ROOT".to_string(), head.root_id.0.clone()),
                ]
            }
            Err(error) => {
                let disposal = if has_runtime {
                    self.stop_workspace_environment_runtime(lane).map(|_| ())
                } else {
                    Ok(())
                };
                let disposal_error = disposal.err();
                self.push_managed_execution_phase(
                    &mut phases,
                    &branch.lane_id,
                    &execution_id,
                    surface,
                    &command_fingerprint,
                    "dispose",
                    if disposal_error.is_some() {
                        "failed"
                    } else if has_runtime {
                        "succeeded"
                    } else {
                        "skipped"
                    },
                    disposal_error
                        .as_ref()
                        .map(|error| error.to_string())
                        .as_deref(),
                    None,
                )?;
                let had_mount = mount.take().is_some();
                self.push_managed_execution_phase(
                    &mut phases,
                    &branch.lane_id,
                    &execution_id,
                    surface,
                    &command_fingerprint,
                    "unmount",
                    if had_mount { "succeeded" } else { "skipped" },
                    None,
                    None,
                )?;
                if let Some(disposal_error) = disposal_error {
                    return Err(Error::Corrupt(format!(
                        "{error}; managed preparation cleanup also failed: {disposal_error}"
                    )));
                }
                return Err(error);
            }
        };

        Ok(ManagedExecutionContext {
            execution_id,
            surface: surface.to_string(),
            lane: lane.to_string(),
            lane_id: branch.lane_id,
            command_fingerprint,
            source_root: head.root_id,
            view,
            workdir,
            environment,
            environment_generation: active_generation.map(|generation| generation.generation_id),
            mount,
            phases,
            #[cfg(test)]
            injected_disposal_error: None,
        })
    }

    #[doc(hidden)]
    pub fn mark_managed_lane_execution_command(
        &self,
        context: &mut ManagedExecutionContext,
        status: &str,
        error: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<()> {
        let lane_id = context.lane_id.clone();
        let execution_id = context.execution_id.clone();
        let surface = context.surface.clone();
        let fingerprint = context.command_fingerprint.clone();
        self.push_managed_execution_phase(
            &mut context.phases,
            &lane_id,
            &execution_id,
            &surface,
            &fingerprint,
            "execute",
            status,
            error,
            Some(serde_json::json!({"exit_code": exit_code})),
        )
    }

    #[doc(hidden)]
    pub fn finalize_managed_lane_execution(
        &mut self,
        context: ManagedExecutionContext,
        checkpoint_message: Option<String>,
    ) -> ManagedExecutionLifecycleReport {
        self.finalize_managed_lane_execution_for_turn(context, checkpoint_message, None)
    }

    #[doc(hidden)]
    pub fn finalize_managed_lane_execution_for_turn(
        &mut self,
        mut context: ManagedExecutionContext,
        checkpoint_message: Option<String>,
        turn_id: Option<&str>,
    ) -> ManagedExecutionLifecycleReport {
        let checkpoint = if let Some(turn_id) = turn_id {
            self.record_lane_workdir_for_turn(&context.lane, turn_id, checkpoint_message)
                .map(|record| {
                    (
                        workspace_checkpoint_from_lane_record(record.clone()),
                        Some(record),
                    )
                })
        } else if context.view.is_some() {
            self.checkpoint_lane_workspace(&context.lane, checkpoint_message)
                .map(|checkpoint| (checkpoint, None))
        } else {
            self.record_lane_workdir(&context.lane, checkpoint_message)
                .map(|record| {
                    (
                        workspace_checkpoint_from_lane_record(record.clone()),
                        Some(record),
                    )
                })
        };
        let (checkpoint, recorded, checkpoint_error_code, checkpoint_error) = match checkpoint {
            Ok((checkpoint, recorded)) => {
                let _ = self.push_managed_context_phase(
                    &mut context,
                    "checkpoint",
                    "succeeded",
                    None,
                    Some(serde_json::json!({
                        "operation": checkpoint.operation.as_ref().map(|id| id.0.as_str()),
                        "root_id": checkpoint.root_id.0,
                        "source_path_count": checkpoint.source_paths.len(),
                        "generated_dirty_paths": checkpoint.generated_dirty_paths,
                    })),
                );
                (Some(checkpoint), recorded, None, None)
            }
            Err(error) => {
                let code = error.code().to_string();
                let reason = format!("Trail managed execution checkpoint failed ({code}): {error}");
                let _ = self.push_managed_context_phase(
                    &mut context,
                    "checkpoint",
                    "failed",
                    Some(&reason),
                    None,
                );
                (None, None, Some(code), Some(reason))
            }
        };

        let has_runtime = context.view.is_some()
            && self
                .active_environment_generation(&context.lane)
                .ok()
                .flatten()
                .is_some_and(|generation| {
                    generation
                        .components
                        .iter()
                        .any(|component| !component.runtime_resources.is_empty())
                });
        #[cfg(test)]
        let disposal = if let Some(error) = context.injected_disposal_error.take() {
            Err(Error::Corrupt(error))
        } else if has_runtime {
            self.stop_workspace_environment_runtime(&context.lane)
                .map(|_| ())
        } else {
            Ok(())
        };
        #[cfg(not(test))]
        let disposal = if has_runtime {
            self.stop_workspace_environment_runtime(&context.lane)
                .map(|_| ())
        } else {
            Ok(())
        };
        let disposal_error = match disposal {
            Ok(()) => {
                let _ = self.push_managed_context_phase(
                    &mut context,
                    "dispose",
                    if has_runtime { "succeeded" } else { "skipped" },
                    None,
                    None,
                );
                None
            }
            Err(error) => {
                let reason = error.to_string();
                let _ = self.push_managed_context_phase(
                    &mut context,
                    "dispose",
                    "failed",
                    Some(&reason),
                    None,
                );
                Some(reason)
            }
        };

        drop(context.mount.take());
        let had_mount = context.view.is_some();
        let _ = self.push_managed_context_phase(
            &mut context,
            "unmount",
            if had_mount { "succeeded" } else { "skipped" },
            None,
            None,
        );

        ManagedExecutionLifecycleReport {
            execution_id: context.execution_id,
            surface: context.surface,
            command_fingerprint: context.command_fingerprint,
            environment_generation: context.environment_generation,
            checkpoint,
            checkpoint_error,
            checkpoint_error_code,
            disposal_error,
            recorded,
            phases: context.phases,
        }
    }

    fn push_managed_context_phase(
        &self,
        context: &mut ManagedExecutionContext,
        phase: &str,
        status: &str,
        error: Option<&str>,
        details: Option<serde_json::Value>,
    ) -> Result<()> {
        let mut details = match details {
            Some(serde_json::Value::Object(details)) => details,
            Some(details) => serde_json::Map::from_iter([("phase_details".to_string(), details)]),
            None => serde_json::Map::new(),
        };
        details.insert(
            "view_id".to_string(),
            context
                .view
                .as_ref()
                .map(|view| serde_json::Value::String(view.view_id.clone()))
                .unwrap_or(serde_json::Value::Null),
        );
        details.insert(
            "environment_generation".to_string(),
            context
                .environment_generation
                .as_ref()
                .map(|generation| serde_json::Value::String(generation.clone()))
                .unwrap_or(serde_json::Value::Null),
        );
        let lane_id = context.lane_id.clone();
        let execution_id = context.execution_id.clone();
        let surface = context.surface.clone();
        let fingerprint = context.command_fingerprint.clone();
        self.push_managed_execution_phase(
            &mut context.phases,
            &lane_id,
            &execution_id,
            &surface,
            &fingerprint,
            phase,
            status,
            error,
            Some(serde_json::Value::Object(details)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn push_managed_execution_phase(
        &self,
        phases: &mut Vec<ManagedExecutionPhaseReceipt>,
        lane_id: &str,
        execution_id: &str,
        surface: &str,
        command_fingerprint: &str,
        phase: &str,
        status: &str,
        error: Option<&str>,
        details: Option<serde_json::Value>,
    ) -> Result<()> {
        self.record_managed_execution_phase(
            Some(lane_id),
            execution_id,
            surface,
            command_fingerprint,
            phase,
            status,
            error,
            details,
        )?;
        phases.push(ManagedExecutionPhaseReceipt {
            phase: phase.to_string(),
            status: status.to_string(),
            error: error.map(str::to_string),
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_managed_execution_phase(
        &self,
        lane_id: Option<&str>,
        execution_id: &str,
        surface: &str,
        command_fingerprint: &str,
        phase: &str,
        status: &str,
        error: Option<&str>,
        details: Option<serde_json::Value>,
    ) -> Result<()> {
        let Some(lane_id) = lane_id else {
            return Ok(());
        };
        self.insert_lane_event(
            lane_id,
            "managed_execution_phase",
            None,
            None,
            &serde_json::json!({
                "execution_id": execution_id,
                "surface": surface,
                "command_fingerprint": command_fingerprint,
                "phase": phase,
                "status": status,
                "error": error,
                "details": details,
            }),
        )?;
        Ok(())
    }
}

fn managed_execution_id(lane: &str, surface: &str, command: &[String]) -> Result<String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::Corrupt(format!("system clock is before UNIX epoch: {error}")))?
        .as_nanos();
    Ok(format!(
        "exec_{}",
        &sha256_hex(&serde_json::to_vec(&(
            lane,
            surface,
            command,
            std::process::id(),
            nonce
        ))?)[..32]
    ))
}

fn managed_environment_is_current(
    desired: &BTreeMap<String, String>,
    existing: &[WorkspaceEnvironmentReport],
) -> bool {
    desired.len() == existing.len()
        && existing.iter().all(|state| {
            state.status == "ready"
                && state.attached_key.as_deref() == Some(state.expected_key.as_str())
                && desired.get(&state.adapter) == Some(&state.expected_key)
        })
}

fn workspace_checkpoint_from_lane_record(record: LaneRecordReport) -> WorkspaceCheckpointReport {
    WorkspaceCheckpointReport {
        view_id: String::new(),
        operation: record.operation,
        root_id: record.root_id,
        journal_sequence: 0,
        source_paths: record
            .changed_paths
            .into_iter()
            .map(|change| change.path)
            .collect(),
        generated_dirty_paths: record.generated_dirty_paths,
        generated_path_accounting: "managed_execution".to_string(),
        upper_recovery_walks: record.upper_recovery_walks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_environment_sync_is_skipped_only_for_exact_ready_bindings() {
        let desired = BTreeMap::from([("cargo-target-seed".to_string(), "key-a".to_string())]);
        let ready = WorkspaceEnvironmentReport {
            view_id: "view-a".to_string(),
            adapter: "cargo-target-seed".to_string(),
            expected_key: "key-a".to_string(),
            attached_key: Some("key-a".to_string()),
            status: "ready".to_string(),
            reason: None,
            updated_at: 1,
        };
        assert!(managed_environment_is_current(
            &desired,
            std::slice::from_ref(&ready)
        ));

        let mut stale = ready.clone();
        stale.status = "stale".to_string();
        assert!(!managed_environment_is_current(&desired, &[stale]));

        let mut detached = ready.clone();
        detached.attached_key = Some("key-old".to_string());
        assert!(!managed_environment_is_current(&desired, &[detached]));

        assert!(!managed_environment_is_current(
            &BTreeMap::from([("cargo-target-seed".to_string(), "key-new".to_string())]),
            std::slice::from_ref(&ready)
        ));
        assert!(!managed_environment_is_current(
            &BTreeMap::from([
                ("cargo-target-seed".to_string(), "key-a".to_string()),
                ("python-venv".to_string(), "key-b".to_string()),
            ]),
            std::slice::from_ref(&ready)
        ));
        assert!(!managed_environment_is_current(
            &BTreeMap::new(),
            std::slice::from_ref(&ready)
        ));
        assert!(!managed_environment_is_current(
            &BTreeMap::from([("python-venv".to_string(), "key-a".to_string())]),
            &[ready]
        ));
        assert!(managed_environment_is_current(&BTreeMap::new(), &[]));
    }

    #[test]
    fn command_and_cleanup_failures_are_both_retained_in_lifecycle_receipt() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("README.md"), "root\n").unwrap();
        Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(root.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "aggregate-failure",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let mut context = db
            .prepare_managed_lane_execution(
                "aggregate-failure",
                "lane_exec",
                &["/bin/sh".into(), "-c".into(), "exit 7".into()],
            )
            .unwrap();
        db.mark_managed_lane_execution_command(
            &mut context,
            "failed",
            Some("command exited 7"),
            Some(7),
        )
        .unwrap();
        context.injected_disposal_error = Some("injected cleanup failure".to_string());

        let report = db.finalize_managed_lane_execution(
            context,
            Some("aggregate failure checkpoint".to_string()),
        );

        assert_eq!(
            report.disposal_error.as_deref(),
            Some("database corrupt: injected cleanup failure")
        );
        assert!(report.phases.iter().any(|phase| {
            phase.phase == "execute"
                && phase.status == "failed"
                && phase.error.as_deref() == Some("command exited 7")
        }));
        assert!(report.phases.iter().any(|phase| {
            phase.phase == "dispose"
                && phase.status == "failed"
                && phase
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("injected cleanup failure"))
        }));
        assert!(report.phases.iter().any(|phase| phase.phase == "unmount"));
    }
}
