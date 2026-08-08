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
    }
}
