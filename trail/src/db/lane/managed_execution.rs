use super::*;
use std::any::Any;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

#[derive(Clone, Debug)]
pub(crate) struct HotAccessCapture {
    pub(crate) session_id: String,
    pub(crate) entry_limit: u64,
    pub(crate) byte_limit: u64,
}

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
    preparation: ManagedExecutionPreparationReceipt,
    sealing_decisions: Vec<ManagedExecutionSealingDecision>,
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
        let resolution_pins = self.managed_execution_resolution_pins(&discovered)?;
        if let Some(unresolved) = resolution_pins
            .iter()
            .find(|pin| pin.status != EnvironmentComponentProposalStatus::Ready)
        {
            let recovery = unresolved
                .recovery_command
                .as_ref()
                .map(|command| command.join(" "))
                .unwrap_or_else(|| format!("trail env discover {lane}"));
            let error = Error::InvalidInput(format!(
                "managed execution requires explicit resolution for environment component `{}` ({}); run `{recovery}`",
                unresolved.component_id,
                unresolved.status.as_str()
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
                Some(serde_json::json!({
                    "missing_resolution_policy": ManagedExecutionMissingResolutionPolicy::Explicit,
                    "resolution_pins": resolution_pins,
                    "recovery_command": unresolved.recovery_command,
                })),
            )?;
            return Err(error);
        }
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
        let graph = if view.is_none() && discovered.components.is_empty() {
            EnvironmentGraphReport {
                source_root: discovered.source_root.clone(),
                total_nodes: 0,
                total_edges: 0,
                offset: 0,
                next_offset: None,
                nodes: Vec::new(),
                edges: Vec::new(),
            }
        } else {
            match self.workspace_environment_graph(lane, None) {
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
                "missing_resolution_policy": ManagedExecutionMissingResolutionPolicy::Explicit,
                "resolution_pins": resolution_pins,
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
        let (output_pins, sealing_decisions) = if let Some(generation) = &active_generation {
            let bindings =
                self.artifact_generation_bindings_for_generation(&generation.generation_id)?;
            (
                managed_execution_output_pins(generation, &bindings)?,
                managed_execution_sealing_decisions(generation),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        if let (Some(view), Some(generation)) = (&view, &active_generation) {
            let cancelled = AtomicBool::new(false);
            let prefetch =
                self.prefetch_environment_hot_set(generation, &command_fingerprint, &cancelled)?;
            self.push_managed_execution_phase(
                &mut phases,
                &branch.lane_id,
                &execution_id,
                surface,
                &command_fingerprint,
                "prefetch",
                if prefetch.matched {
                    "succeeded"
                } else {
                    "skipped"
                },
                None,
                Some(serde_json::to_value(&prefetch)?),
            )?;
            self.begin_environment_hot_access_session(
                &view.view_id,
                &execution_id,
                &command_fingerprint,
                generation,
            )?;
        } else {
            self.push_managed_execution_phase(
                &mut phases,
                &branch.lane_id,
                &execution_id,
                surface,
                &command_fingerprint,
                "prefetch",
                "skipped",
                None,
                Some(serde_json::json!({"reason": "no_active_environment_generation"})),
            )?;
        }
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
                    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
                    {
                        self.mount_dokan_cow_workdir_for_lane(lane)
                            .map(|mount| Box::new(mount) as Box<dyn Any + Send>)
                    }
                    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
                    {
                        Err(Error::InvalidInput(
                            "dokan-cow workdirs require an x86_64 Windows build".to_string(),
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
                    let _ = self.finish_environment_hot_access_session(&execution_id, false);
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
            source_root: head.root_id.clone(),
            view: view.clone(),
            workdir,
            environment,
            environment_generation: active_generation
                .as_ref()
                .map(|generation| generation.generation_id.clone()),
            preparation: ManagedExecutionPreparationReceipt {
                source_root: head.root_id.clone(),
                view_id: view.as_ref().map(|view| view.view_id.clone()),
                view_generation: view.as_ref().map(|view| view.generation),
                missing_resolution_policy: ManagedExecutionMissingResolutionPolicy::Explicit,
                resolution_pins,
                environment_generation: active_generation
                    .as_ref()
                    .map(|generation| generation.generation_id.clone()),
                output_pins,
            },
            sealing_decisions,
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
        )?;
        self.finish_environment_hot_access_session(&execution_id, status == "succeeded")
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
            self.record_lane_workdir(&context.lane, checkpoint_message)
                .and_then(|record| {
                    let view = self.lane_workspace_view(&context.lane)?.ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "lane `{}` does not have a layered workspace view",
                            context.lane
                        ))
                    })?;
                    let checkpoint = WorkspaceCheckpointReport {
                        view_id: view.view_id,
                        operation: record.operation.clone(),
                        root_id: record.root_id.clone(),
                        journal_sequence: view.checkpoint_seq,
                        source_paths: record
                            .changed_paths
                            .iter()
                            .map(|change| change.path.clone())
                            .collect(),
                        generated_dirty_paths: record.generated_dirty_paths,
                        generated_path_accounting: "journal_interval".to_string(),
                        upper_recovery_walks: record.upper_recovery_walks,
                    };
                    Ok((checkpoint, Some(record)))
                })
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

        let source_root_after = checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.root_id.clone());
        let source_changed = source_root_after
            .as_ref()
            .is_some_and(|root| root != &context.source_root);
        let mut sealing_decisions = context.sealing_decisions.clone();
        if source_changed {
            for decision in &mut sealing_decisions {
                if decision.decision.starts_with("await_") {
                    decision.decision = "replan_required".to_string();
                    decision.reason =
                        "identity-bearing lane source changed during execution; replan before sealing"
                            .to_string();
                }
            }
        }
        let checkpoint_status = if checkpoint_error.is_some() {
            "failed"
        } else {
            "succeeded"
        };
        let disposal_status = if disposal_error.is_some() {
            "failed"
        } else if has_runtime {
            "succeeded"
        } else {
            "skipped"
        };
        let unmount_status = if had_mount { "succeeded" } else { "skipped" };
        let errors = checkpoint_error
            .iter()
            .chain(disposal_error.iter())
            .cloned()
            .collect::<Vec<_>>();
        let finalization = ManagedExecutionFinalizationReceipt {
            source_root_before: context.source_root.clone(),
            source_root_after,
            source_changed,
            checkpoint_status: checkpoint_status.to_string(),
            disposal_status: disposal_status.to_string(),
            unmount_status: unmount_status.to_string(),
            complete: errors.is_empty(),
            sealing_decisions,
            errors,
        };

        ManagedExecutionLifecycleReport {
            execution_id: context.execution_id,
            surface: context.surface,
            command_fingerprint: context.command_fingerprint,
            preparation: Some(context.preparation),
            environment_generation: context.environment_generation,
            checkpoint,
            checkpoint_error,
            checkpoint_error_code,
            disposal_error,
            recorded,
            finalization: Some(finalization),
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
            details.clone(),
        )?;
        phases.push(ManagedExecutionPhaseReceipt {
            phase: phase.to_string(),
            status: status.to_string(),
            error: error.map(str::to_string),
            details,
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

    fn environment_hot_identities(
        generation: &EnvironmentGenerationReport,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let components = generation
            .components
            .iter()
            .map(|component| {
                (
                    component.component_id.clone(),
                    component.component_key.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let manifests = generation
            .components
            .iter()
            .flat_map(|component| {
                component.outputs.iter().filter_map(move |output| {
                    output.manifest_object_id.as_ref().map(|manifest| {
                        (
                            format!("{}:{}", component.component_id, output.name),
                            manifest.clone(),
                        )
                    })
                })
            })
            .collect::<BTreeMap<_, _>>();
        Ok((
            serde_json::to_vec(&components)?,
            serde_json::to_vec(&manifests)?,
        ))
    }

    fn begin_environment_hot_access_session(
        &self,
        view_id: &str,
        execution_id: &str,
        command_fingerprint: &str,
        generation: &EnvironmentGenerationReport,
    ) -> Result<()> {
        let (components, manifests) = Self::environment_hot_identities(generation)?;
        let mut random = [0_u8; 24];
        getrandom::getrandom(&mut random)
            .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
        let session_id = format!("hot_{}", hex::encode(random));
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO environment_hot_access_sessions
             (session_id,execution_id,view_id,command_fingerprint,generation_id,
              component_identities_json,manifest_identities_json,owner_pid,owner_start_token,
              status,entries_json,entry_count,total_bytes,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'recording','[]',0,0,?10,?10)",
            params![
                session_id,
                execution_id,
                view_id,
                command_fingerprint,
                generation.generation_id,
                components,
                manifests,
                std::process::id(),
                current_process_start_token(),
                now,
            ],
        )?;
        Ok(())
    }

    fn finish_environment_hot_access_session(
        &self,
        execution_id: &str,
        succeeded: bool,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE environment_hot_access_sessions
             SET status=?1,updated_at=?2
             WHERE execution_id=?3 AND owner_pid=?4 AND owner_start_token=?5 AND status='recording'",
            params![
                if succeeded { "succeeded" } else { "failed" },
                now_ts(),
                execution_id,
                std::process::id(),
                current_process_start_token(),
            ],
        )?;
        Ok(())
    }

    pub(crate) fn recover_environment_hot_access_sessions(&self) -> Result<()> {
        let sessions = {
            let mut statement = self.conn.prepare(
                "SELECT session_id,owner_pid,owner_start_token
                 FROM environment_hot_access_sessions
                 WHERE status IN ('recording','succeeded') ORDER BY session_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let now = now_ts();
        for (session_id, pid, token) in sessions {
            if !process_matches_start_token(pid, &token) {
                self.conn.execute(
                    "UPDATE environment_hot_access_sessions
                     SET status='failed',updated_at=?1
                     WHERE session_id=?2 AND status IN ('recording','succeeded')",
                    params![now, session_id],
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn hot_access_capture_for_source_upper(
        &self,
        source_upper: &Path,
    ) -> Result<Option<HotAccessCapture>> {
        let capture = self
            .conn
            .query_row(
                "SELECT s.session_id,s.owner_pid,s.owner_start_token
                 FROM environment_hot_access_sessions s
                 JOIN workspace_views v ON v.view_id=s.view_id
                 WHERE v.source_upper=?1 AND s.status IN ('recording','succeeded')
                 ORDER BY s.created_at DESC,s.session_id DESC LIMIT 1",
                params![source_upper.to_string_lossy()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((session_id, owner_pid, owner_token)) = capture else {
            return Ok(None);
        };
        if owner_pid != std::process::id() || owner_token != current_process_start_token() {
            return Ok(None);
        }
        let config = &self.config().workspace_views;
        Ok(Some(HotAccessCapture {
            session_id,
            entry_limit: config.prefetch_max_entries,
            byte_limit: config.prefetch_max_bytes,
        }))
    }

    pub(crate) fn publish_environment_hot_accesses(
        &self,
        capture: &HotAccessCapture,
        entries: &[EnvironmentHotPathEntry],
    ) -> Result<()> {
        let row = self
            .conn
            .query_row(
                "SELECT command_fingerprint,generation_id,component_identities_json,
                        manifest_identities_json,status,owner_pid,owner_start_token
                 FROM environment_hot_access_sessions WHERE session_id=?1",
                params![capture.session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, u32>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((fingerprint, generation_id, components, manifests, status, pid, token)) = row
        else {
            return Ok(());
        };
        if status != "succeeded"
            || pid != std::process::id()
            || token != current_process_start_token()
        {
            return Ok(());
        }
        let mut bounded = entries.to_vec();
        bounded.sort_by(|left, right| {
            (&left.layer_id, &left.path).cmp(&(&right.layer_id, &right.path))
        });
        bounded.dedup_by(|left, right| left.layer_id == right.layer_id && left.path == right.path);
        let mut bytes = 0_u64;
        bounded.retain(|entry| {
            if bytes.saturating_add(entry.size_bytes) > capture.byte_limit {
                return false;
            }
            bytes = bytes.saturating_add(entry.size_bytes);
            true
        });
        bounded.truncate(usize::try_from(capture.entry_limit).unwrap_or(usize::MAX));
        bytes = bounded.iter().map(|entry| entry.size_bytes).sum();
        let entries_json = serde_json::to_vec(&bounded)?;
        let hot_set_id = format!(
            "hotset_{}",
            crate::ids::short_hash(format!("{fingerprint}:{generation_id}").as_bytes(), 24,)
        );
        let now = now_ts();
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO environment_hot_sets
             (hot_set_id,command_fingerprint,generation_id,component_identities_json,
              manifest_identities_json,entries_json,entry_count,total_bytes,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)
             ON CONFLICT(command_fingerprint,generation_id) DO UPDATE SET
              component_identities_json=excluded.component_identities_json,
              manifest_identities_json=excluded.manifest_identities_json,
              entries_json=excluded.entries_json,entry_count=excluded.entry_count,
              total_bytes=excluded.total_bytes,updated_at=excluded.updated_at",
            params![
                hot_set_id,
                fingerprint,
                generation_id,
                components,
                manifests,
                entries_json,
                bounded.len() as u64,
                bytes,
                now,
            ],
        )?;
        transaction.execute(
            "UPDATE environment_hot_access_sessions SET status='published',entries_json=?1,
             entry_count=?2,total_bytes=?3,updated_at=?4 WHERE session_id=?5 AND status='succeeded'",
            params![entries_json, bounded.len() as u64, bytes, now, capture.session_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn prefetch_environment_hot_set(
        &self,
        generation: &EnvironmentGenerationReport,
        command_fingerprint: &str,
        cancelled: &AtomicBool,
    ) -> Result<EnvironmentPrefetchReport> {
        let config = &self.config().workspace_views;
        let mut report = EnvironmentPrefetchReport {
            entry_limit: config.prefetch_max_entries,
            byte_limit: config.prefetch_max_bytes,
            ..EnvironmentPrefetchReport::default()
        };
        let (components, manifests) = Self::environment_hot_identities(generation)?;
        let row = self
            .conn
            .query_row(
                "SELECT component_identities_json,manifest_identities_json,entries_json
                 FROM environment_hot_sets WHERE command_fingerprint=?1 AND generation_id=?2",
                params![command_fingerprint, generation.generation_id],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((stored_components, stored_manifests, entries_json)) = row else {
            return Ok(report);
        };
        if stored_components != components || stored_manifests != manifests {
            return Ok(report);
        }
        report.matched = true;
        let entries: Vec<EnvironmentHotPathEntry> = serde_json::from_slice(&entries_json)?;
        for entry in entries {
            if cancelled.load(AtomicOrdering::Acquire) {
                report.cancelled = true;
                break;
            }
            report.entries_considered = report.entries_considered.saturating_add(1);
            if report.entries_prefetched >= report.entry_limit
                || report.bytes_prefetched >= report.byte_limit
            {
                break;
            }
            let storage_path = self
                .conn
                .query_row(
                    "SELECT storage_path FROM workspace_layers WHERE layer_id=?1 AND state='ready'",
                    params![entry.layer_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let Some(storage_path) = storage_path else {
                continue;
            };
            let path = safe_join(Path::new(&storage_path), &entry.path)?;
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    metadata
                }
                _ => continue,
            };
            let remaining = report.byte_limit.saturating_sub(report.bytes_prefetched);
            let amount = metadata.len().min(entry.size_bytes).min(remaining);
            let mut file = File::open(&path)?;
            let copied = std::io::copy(
                &mut std::io::Read::by_ref(&mut file).take(amount),
                &mut std::io::sink(),
            )?;
            report.bytes_prefetched = report.bytes_prefetched.saturating_add(copied);
            report.entries_prefetched = report.entries_prefetched.saturating_add(1);
        }
        Ok(report)
    }

    fn managed_execution_resolution_pins(
        &self,
        discovery: &EnvironmentDiscoveryReport,
    ) -> Result<Vec<ManagedExecutionResolutionPin>> {
        discovery
            .components
            .iter()
            .map(|component| {
                let snapshot = self.artifact_resolution_snapshot_for_component(
                    &discovery.source_root,
                    &component.component_id,
                    &component.adapter_identity,
                )?;
                let (snapshot_id, proposal_key) = snapshot
                    .map(|(snapshot_id, snapshot)| (Some(snapshot_id), Some(snapshot.proposal_key)))
                    .unwrap_or((None, None));
                Ok(ManagedExecutionResolutionPin {
                    component_id: component.component_id.clone(),
                    adapter_identity: component.adapter_identity.clone(),
                    status: component.status.clone(),
                    proposal_key,
                    snapshot_id,
                    recovery_command: component
                        .recovery_actions
                        .iter()
                        .find_map(|action| action.command.clone()),
                })
            })
            .collect()
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

fn managed_execution_output_pins(
    generation: &EnvironmentGenerationReport,
    bindings: &[ArtifactGenerationBindingReportV1],
) -> Result<Vec<ManagedExecutionOutputPin>> {
    let bindings = bindings
        .iter()
        .map(|binding| {
            (
                (binding.component_id.as_str(), binding.output_name.as_str()),
                binding,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut pins = Vec::new();
    for component in &generation.components {
        for output in &component.outputs {
            let binding = bindings
                .get(&(component.component_id.as_str(), output.name.as_str()))
                .copied();
            if output.policy.has_immutable_layer() && binding.is_none() {
                return Err(Error::Corrupt(format!(
                    "managed execution cannot pin immutable output `{}/{}` because generation `{}` has no artifact binding",
                    component.component_id, output.name, generation.generation_id
                )));
            }
            if let Some(binding) = binding
                && binding.desired_key != component.component_key
            {
                return Err(Error::Corrupt(format!(
                    "managed execution artifact binding for `{}/{}` disagrees with generation component identity",
                    component.component_id, output.name
                )));
            }
            pins.push(ManagedExecutionOutputPin {
                component_id: component.component_id.clone(),
                output_name: output.name.clone(),
                component_key: component.component_key.clone(),
                policy: output.policy,
                storage_identity: output.storage_identity.clone(),
                artifact_binding_id: binding.map(|binding| binding.binding_id.clone()),
                artifact_envelope_id: binding.map(|binding| binding.envelope_id.clone()),
                artifact_tree_root_id: binding.map(|binding| binding.tree_root_id.clone()),
                artifact_binding_identity: binding.map(|binding| binding.binding_identity.clone()),
            });
        }
    }
    Ok(pins)
}

fn managed_execution_sealing_decisions(
    generation: &EnvironmentGenerationReport,
) -> Vec<ManagedExecutionSealingDecision> {
    generation
        .components
        .iter()
        .flat_map(|component| {
            component.outputs.iter().map(|output| {
                let (decision, reason) = match (output.policy, output.publish) {
                    (EnvironmentOutputPolicy::Disposable, _) => {
                        ("dispose", "disposable output is never sealed or promoted")
                    }
                    (_, EnvironmentPublicationTrigger::SuccessfulGate) => (
                        "await_successful_gate",
                        "seal only after the named successful gate revalidates its pins",
                    ),
                    (_, EnvironmentPublicationTrigger::OnSync) => (
                        "await_sync",
                        "seal only during a later environment synchronization",
                    ),
                    (_, EnvironmentPublicationTrigger::Manual) => (
                        "await_manual_promotion",
                        "retain private changes until explicit promotion",
                    ),
                    (EnvironmentOutputPolicy::ImmutableShared, _) => (
                        "preserve_verified_artifact",
                        "mounted immutable content is already sealed and cannot be modified",
                    ),
                    (EnvironmentOutputPolicy::ImmutableSeedPrivate, _) => (
                        "retain_private_delta",
                        "writes remain in the lane-private upper",
                    ),
                    (EnvironmentOutputPolicy::WritablePrivate, _) => {
                        ("retain_private", "writable output remains lane-private")
                    }
                };
                ManagedExecutionSealingDecision {
                    component_id: component.component_id.clone(),
                    output_name: output.name.clone(),
                    policy: output.policy,
                    publication: output.publish,
                    gate: output.gate.clone(),
                    decision: decision.to_string(),
                    reason: reason.to_string(),
                }
            })
        })
        .collect()
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
    use crate::{ArtifactEnvelopeId, ArtifactTreeId};

    fn hot_generation(manifest: &str) -> EnvironmentGenerationReport {
        EnvironmentGenerationReport {
            generation_id: "generation-hot".to_string(),
            view_id: "view-hot".to_string(),
            generation_sequence: 1,
            source_root: ObjectId("object_source".to_string()),
            specification_digest: "specification".to_string(),
            predecessor_generation_id: None,
            state: "active".to_string(),
            components: vec![EnvironmentGenerationComponentReport {
                component_id: "component".to_string(),
                adapter_identity: "recipe".to_string(),
                kind: "command".to_string(),
                component_key: "component-key".to_string(),
                layer_id: Some("layer-hot".to_string()),
                mount_path: Some("generated".to_string()),
                dependencies: Vec::new(),
                outputs: vec![EnvironmentGenerationOutputReport {
                    name: "output".to_string(),
                    policy: EnvironmentOutputPolicy::ImmutableShared,
                    reuse: EnvironmentReuseMode::Exact,
                    scope: EnvironmentSharingScope::Workspace,
                    publish: EnvironmentPublicationTrigger::OnSync,
                    gate: None,
                    storage_identity: "storage".to_string(),
                    layer_id: Some("layer-hot".to_string()),
                    manifest_object_id: Some(manifest.to_string()),
                    publication_id: None,
                    mount_path: "generated".to_string(),
                    layer_subpath: String::new(),
                }],
                caches: Vec::new(),
                external_artifacts: Vec::new(),
                runtime_resources: Vec::new(),
            }],
            created_at: 1,
            activated_at: Some(1),
            retired_at: None,
        }
    }

    #[test]
    fn hot_sets_are_authenticated_bounded_exact_and_cancellable() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("README.md"), "root\n").unwrap();
        Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(root.path()).unwrap();
        let layer = tempfile::tempdir().unwrap();
        std::fs::write(layer.path().join("hot.bin"), vec![7_u8; 32]).unwrap();
        db.conn
            .execute(
                "INSERT INTO workspace_layers
                 (layer_id,kind,cache_key,adapter,adapter_version,manifest_object_id,storage_path,
                  state,logical_bytes,physical_bytes,entry_count,portability_scope,builder_id,
                  lease_expires_at,last_used_at,created_at)
                 VALUES('layer-hot','generated','cache-hot','recipe',1,'manifest-hot',?1,
                        'ready',32,32,1,'workspace',NULL,NULL,1,1)",
                params![layer.path().to_string_lossy()],
            )
            .unwrap();
        let generation = hot_generation("manifest-hot");
        db.begin_environment_hot_access_session(
            "view-hot",
            "execution-hot",
            "command-hot",
            &generation,
        )
        .unwrap();
        db.finish_environment_hot_access_session("execution-hot", true)
            .unwrap();
        let session_id: String = db
            .conn
            .query_row(
                "SELECT session_id FROM environment_hot_access_sessions WHERE execution_id='execution-hot'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        db.publish_environment_hot_accesses(
            &HotAccessCapture {
                session_id,
                entry_limit: 1,
                byte_limit: 32,
            },
            &[
                EnvironmentHotPathEntry {
                    layer_id: "layer-hot".to_string(),
                    path: "hot.bin".to_string(),
                    size_bytes: 32,
                },
                EnvironmentHotPathEntry {
                    layer_id: "layer-hot".to_string(),
                    path: "ignored.bin".to_string(),
                    size_bytes: 32,
                },
            ],
        )
        .unwrap();

        let report = db
            .prefetch_environment_hot_set(&generation, "command-hot", &AtomicBool::new(false))
            .unwrap();
        assert!(report.matched);
        assert_eq!(report.entries_prefetched, 1);
        assert_eq!(report.bytes_prefetched, 32);

        let cancelled = AtomicBool::new(true);
        let report = db
            .prefetch_environment_hot_set(&generation, "command-hot", &cancelled)
            .unwrap();
        assert!(report.matched);
        assert!(report.cancelled);
        assert_eq!(report.bytes_prefetched, 0);

        let changed = db
            .prefetch_environment_hot_set(
                &hot_generation("manifest-changed"),
                "command-hot",
                &AtomicBool::new(false),
            )
            .unwrap();
        assert!(!changed.matched);
    }

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
        let preparation = report.preparation.as_ref().unwrap();
        assert_eq!(
            preparation.missing_resolution_policy,
            ManagedExecutionMissingResolutionPolicy::Explicit
        );
        assert!(preparation.resolution_pins.is_empty());
        assert!(preparation.output_pins.is_empty());
        let finalization = report.finalization.as_ref().unwrap();
        assert!(!finalization.complete);
        assert_eq!(finalization.disposal_status, "failed");
        assert!(finalization
            .errors
            .iter()
            .any(|error| error.contains("injected cleanup failure")));
    }

    #[test]
    fn managed_preparation_requires_explicit_resolution_with_exact_recovery() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname='managed-resolution'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(root.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "needs-resolution",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();

        let error = db
            .prepare_managed_lane_execution(
                "needs-resolution",
                "lane_exec",
                &["cargo".into(), "test".into()],
            )
            .err()
            .unwrap();
        assert!(error.to_string().contains(
            "managed execution requires explicit resolution for environment component `cargo-target-seed` (resolvable)"
        ));
        assert!(error
            .to_string()
            .contains("trail env resolve component cargo-target-seed --lane needs-resolution"));
    }

    #[test]
    fn managed_output_pins_and_sealing_decisions_are_exact_and_deterministic() {
        let envelope_id =
            ArtifactEnvelopeId::parse(format!("artifact_envelope_{}", "a".repeat(64))).unwrap();
        let tree_root_id =
            ArtifactTreeId::parse(format!("artifact_tree_{}", "b".repeat(64))).unwrap();
        let generation = EnvironmentGenerationReport {
            generation_id: "envgen-test".into(),
            view_id: "view-test".into(),
            generation_sequence: 1,
            source_root: ObjectId("source-root".into()),
            specification_digest: "specification".into(),
            predecessor_generation_id: None,
            state: "active".into(),
            components: vec![EnvironmentGenerationComponentReport {
                component_id: "fixture".into(),
                adapter_identity: "trail/fixture@1".into(),
                kind: "generated".into(),
                component_key: "desired-key".into(),
                layer_id: Some("layer-test".into()),
                mount_path: Some("generated".into()),
                dependencies: Vec::new(),
                outputs: vec![
                    EnvironmentGenerationOutputReport {
                        name: "shared".into(),
                        policy: EnvironmentOutputPolicy::ImmutableShared,
                        reuse: EnvironmentReuseMode::Exact,
                        scope: EnvironmentSharingScope::Workspace,
                        publish: EnvironmentPublicationTrigger::Never,
                        gate: None,
                        storage_identity: "layer-test".into(),
                        layer_id: Some("layer-test".into()),
                        manifest_object_id: Some("manifest-test".into()),
                        publication_id: None,
                        mount_path: "generated".into(),
                        layer_subpath: String::new(),
                    },
                    EnvironmentGenerationOutputReport {
                        name: "private".into(),
                        policy: EnvironmentOutputPolicy::WritablePrivate,
                        reuse: EnvironmentReuseMode::None,
                        scope: EnvironmentSharingScope::Lane,
                        publish: EnvironmentPublicationTrigger::Manual,
                        gate: None,
                        storage_identity: "private-test".into(),
                        layer_id: None,
                        manifest_object_id: None,
                        publication_id: None,
                        mount_path: "private".into(),
                        layer_subpath: String::new(),
                    },
                ],
                caches: Vec::new(),
                external_artifacts: Vec::new(),
                runtime_resources: Vec::new(),
            }],
            created_at: 1,
            activated_at: Some(1),
            retired_at: None,
        };
        let bindings = vec![ArtifactGenerationBindingReportV1 {
            binding_id: "binding-test".into(),
            generation_id: generation.generation_id.clone(),
            component_id: "fixture".into(),
            output_name: "shared".into(),
            desired_key: "desired-key".into(),
            envelope_id: envelope_id.clone(),
            tree_root_id: tree_root_id.clone(),
            binding_identity: "artifact-binding-test".into(),
            created_at: 1,
        }];

        let pins = managed_execution_output_pins(&generation, &bindings).unwrap();
        assert_eq!(pins.len(), 2);
        assert_eq!(pins[0].artifact_envelope_id, Some(envelope_id));
        assert_eq!(pins[0].artifact_tree_root_id, Some(tree_root_id));
        assert!(pins[1].artifact_envelope_id.is_none());
        let decisions = managed_execution_sealing_decisions(&generation);
        assert_eq!(decisions[0].decision, "preserve_verified_artifact");
        assert_eq!(decisions[1].decision, "await_manual_promotion");
    }
}
