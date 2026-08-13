use super::workdir::{classify_view_path, lane_workdir_ignore_matcher, ViewPathClass};
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read, Write};
use std::process::{Child, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const MAX_GUEST_DIAGNOSTIC_BYTES: usize = 16 * 1024;
const DEFAULT_MAX_PROJECTION_ENTRIES: u64 = 100_000;
const DEFAULT_MAX_PROJECTION_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const GUEST_EXECUTION_ROOT: &str = "/tmp/trail-executions";
const MAX_GUEST_STDOUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_GUEST_STDERR_BYTES: usize = 16 * 1024 * 1024;
const GUEST_MANIFEST_SCHEMA: u32 = 1;
const MAX_GUEST_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_GUEST_MANIFESTS: usize = 4096;
const MAX_RETAINED_TERMINAL_GUEST_MANIFESTS: usize = 256;
const MAX_CONCURRENT_GUEST_EXECUTIONS: usize = 4;
const GUEST_PROTOCOL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const GUEST_CANCELLATION_WAIT: Duration = Duration::from_secs(30);
pub(super) const DEFAULT_GUEST_COMMAND_TIMEOUT_SECS: u64 = 60 * 60;
pub(super) const MAX_GUEST_COMMAND_TIMEOUT_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug)]
struct ProjectionLimits {
    entries: u64,
    total_bytes: u64,
    file_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SnapshotEntry {
    kind: String,
    content: String,
    mode: u32,
}

#[derive(Debug)]
struct BuiltProjection {
    archive_path: PathBuf,
    input_digest: String,
    projected_entries: u64,
    projected_bytes: u64,
    source_snapshot: BTreeMap<String, SnapshotEntry>,
}

#[derive(Debug)]
struct CandidateProjection {
    output_digest: String,
    source_snapshot: BTreeMap<String, SnapshotEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GuestExecutionManifest {
    schema: u32,
    execution_id: String,
    lane_id: String,
    profile: String,
    lima_instance: String,
    guest_namespace: String,
    staging_path: String,
    owner_pid: u32,
    owner_start_token: String,
    phase: String,
    input_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    candidate_digest: Option<String>,
    #[serde(default)]
    imported_paths: Vec<String>,
    #[serde(default)]
    removed_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checkpoint_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checkpoint_operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cancellation_requested_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cancellation_completed_at: Option<i64>,
    #[serde(default)]
    process_group_terminated: bool,
    updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GuestCancellationRequest {
    schema: u32,
    execution_id: String,
    lane_id: String,
    requested_at: i64,
}

pub(super) fn preflight_colima_guest_execution(
    toolchain: &super::workspace_runtime_toolchain::ColimaToolchain,
    profile: &str,
) -> Result<()> {
    let instance = super::workspace_runtime::colima_lima_instance(profile);
    let result = run_guest_status(toolchain, &instance, &["true".to_string()], None, None)?;
    if result.status.success() {
        return Ok(());
    }
    Err(Error::InvalidInput(format!(
        "Colima profile `{profile}` cannot execute managed lane commands through Lima instance `{instance}`: {}",
        guest_diagnostic(&result.stderr, &[])
    )))
}

impl Trail {
    pub(crate) fn managed_guest_recovery_doctor_check(&self) -> DoctorCheck {
        match inspect_guest_execution_manifests(self) {
            Ok(inspection) => {
                let status = if inspection.ambiguous > 0 {
                    "error"
                } else if inspection.live > 0 || inspection.recoverable > 0 {
                    "warning"
                } else {
                    "ok"
                };
                let message = if inspection.ambiguous > 0 {
                    format!(
                        "{} interrupted guest execution(s) require explicit lane inspection before retry",
                        inspection.ambiguous
                    )
                } else if inspection.live > 0 || inspection.recoverable > 0 {
                    format!(
                        "{} live and {} safely recoverable guest execution(s) are recorded",
                        inspection.live, inspection.recoverable
                    )
                } else {
                    "no interrupted managed guest executions require recovery".to_string()
                };
                doctor_check(
                    "managed_guest_executions",
                    status,
                    message,
                    Some(serde_json::json!({
                        "active": inspection.active,
                        "live": inspection.live,
                        "recoverable": inspection.recoverable,
                        "ambiguous": inspection.ambiguous,
                        "terminal": inspection.terminal,
                        "phases": inspection.phases,
                    })),
                )
            }
            Err(error) => doctor_check(
                "managed_guest_executions",
                "error",
                format!("could not safely inspect managed guest execution receipts: {error}"),
                None,
            ),
        }
    }

    pub(super) fn run_colima_lane_command(
        &mut self,
        context: &mut ManagedExecutionContext,
        view: &LaneWorkspaceViewReport,
        command: &[String],
        timeout: Duration,
    ) -> Result<CommandRunResult> {
        let run = self.run_colima_managed_command(context, view, command, Some(timeout))?;
        io::stdout().write_all(&run.stdout)?;
        io::stderr().write_all(&run.stderr)?;
        Ok(run)
    }

    pub(super) fn run_colima_managed_command(
        &mut self,
        context: &mut ManagedExecutionContext,
        view: &LaneWorkspaceViewReport,
        command: &[String],
        timeout: Option<Duration>,
    ) -> Result<CommandRunResult> {
        let mut command_marked = false;
        let result = self.run_colima_lane_command_inner(
            context,
            view,
            command,
            timeout,
            &mut command_marked,
            None,
        );
        let result = result.map_err(|error| classify_guest_execution_error(context, error));
        if result.is_err() && !command_marked {
            let error = result.as_ref().err().map(ToString::to_string);
            self.mark_managed_lane_execution_command(context, "failed", error.as_deref(), None)?;
        }
        result
    }

    pub fn cancel_lane_workspace_execution(
        &self,
        lane: &str,
        execution_id: Option<&str>,
    ) -> Result<WorkspaceExecCancellationReport> {
        let (manifest_path, mut manifest, phase_before, owner_was_live, request) = {
            let _cancellation_fence =
                Trail::with_write_lock_wait(GUEST_CANCELLATION_WAIT, || self.acquire_write_lock())?;
            let branch = self.lane_branch(lane)?;
            let directory = guest_manifest_directory(self)?;
            let mut candidates = Vec::new();
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                    continue;
                }
                let manifest = read_guest_manifest(&path)?;
                validate_guest_manifest_identity(self, &manifest)?;
                if manifest.lane_id != branch.lane_id {
                    continue;
                }
                if execution_id.is_some_and(|requested| requested != manifest.execution_id) {
                    continue;
                }
                if guest_cancellation_is_terminal(&manifest) {
                    if execution_id.is_some() {
                        candidates.push((path, manifest));
                    }
                    continue;
                }
                if manifest.cancellation_completed_at.is_some() {
                    candidates.push((path, manifest));
                    continue;
                }
                if !guest_manifest_is_cancellable(&manifest) {
                    if execution_id.is_some() {
                        return Err(Error::InvalidInput(format!(
                            "managed execution `{}` is in non-cancellable phase `{}`",
                            manifest.execution_id, manifest.phase
                        )));
                    }
                    continue;
                }
                candidates.push((path, manifest));
            }
            if candidates.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` has no matching cancellable Colima execution"
                )));
            }
            if candidates.len() != 1 {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` has {} matching Colima executions; pass --execution-id",
                    candidates.len()
                )));
            }
            let (manifest_path, manifest) = candidates.remove(0);
            let phase_before = manifest.phase.clone();
            let owner_was_live =
                process_matches_start_token(manifest.owner_pid, &manifest.owner_start_token);
            let request = if !guest_cancellation_is_terminal(&manifest) {
                let request_path = guest_cancellation_path(self, &manifest.execution_id)?;
                let request = if let Some(request) =
                    read_guest_cancellation_request(&request_path, &manifest)?
                {
                    request
                } else {
                    let request = GuestCancellationRequest {
                        schema: GUEST_MANIFEST_SCHEMA,
                        execution_id: manifest.execution_id.clone(),
                        lane_id: manifest.lane_id.clone(),
                        requested_at: now_ts(),
                    };
                    write_file_atomic(&request_path, &serde_json::to_vec_pretty(&request)?, true)?;
                    request
                };
                Some((request_path, request))
            } else {
                None
            };
            (
                manifest_path,
                manifest,
                phase_before,
                owner_was_live,
                request,
            )
        };

        if let Some((request_path, request)) = request {
            let started = Instant::now();
            while owner_was_live && started.elapsed() < GUEST_CANCELLATION_WAIT {
                std::thread::sleep(Duration::from_millis(50));
                manifest = read_guest_manifest(&manifest_path)?;
                if guest_cancellation_is_terminal(&manifest) {
                    break;
                }
                if !process_matches_start_token(manifest.owner_pid, &manifest.owner_start_token) {
                    break;
                }
            }
            if !guest_cancellation_is_terminal(&manifest)
                && !process_matches_start_token(manifest.owner_pid, &manifest.owner_start_token)
            {
                let _cancellation_fence =
                    Trail::with_write_lock_wait(GUEST_CANCELLATION_WAIT, || {
                        self.acquire_write_lock()
                    })?;
                manifest = read_guest_manifest(&manifest_path)?;
                if !guest_cancellation_is_terminal(&manifest) {
                    let toolchain =
                        super::workspace_runtime_toolchain::ColimaToolchain::resolve(false)?;
                    let process_group_path = format!("{}/process-group", manifest.guest_namespace);
                    manifest.process_group_terminated |= terminate_guest_process_group(
                        &toolchain,
                        &manifest.lima_instance,
                        &process_group_path,
                        false,
                    )?;
                    cleanup_guest_namespace(
                        &toolchain,
                        &manifest.lima_instance,
                        &manifest.guest_namespace,
                    )?;
                    manifest
                        .cancellation_requested_at
                        .get_or_insert(request.requested_at);
                    manifest
                        .cancellation_completed_at
                        .get_or_insert_with(now_ts);
                    update_guest_manifest(
                        &manifest_path,
                        &mut manifest,
                        "terminal_cancelled",
                        None,
                    )?;
                    let _ = fs::remove_file(&request_path);
                }
            }
            if !guest_cancellation_is_terminal(&manifest) {
                return Err(Error::InvalidInput(format!(
                    "cancellation and cleanup of managed execution `{}` were not acknowledged within {} seconds",
                    manifest.execution_id,
                    GUEST_CANCELLATION_WAIT.as_secs()
                )));
            }
        }
        Ok(WorkspaceExecCancellationReport {
            lane_id: manifest.lane_id,
            execution_id: manifest.execution_id,
            status: "cancelled".to_string(),
            phase_before,
            profile: manifest.profile,
            lima_instance: manifest.lima_instance,
            owner_was_live,
            process_group_terminated: manifest.process_group_terminated,
            cleanup_status: if matches!(
                manifest.phase.as_str(),
                "cleanup_failed" | "terminal_failed"
            ) {
                "failed"
            } else {
                "succeeded"
            }
            .to_string(),
        })
    }

    fn run_colima_lane_command_inner(
        &mut self,
        context: &mut ManagedExecutionContext,
        view: &LaneWorkspaceViewReport,
        command: &[String],
        timeout: Option<Duration>,
        command_marked: &mut bool,
        toolchain_override: Option<super::workspace_runtime_toolchain::ColimaToolchain>,
    ) -> Result<CommandRunResult> {
        if self.config.runtime.provider != "colima" {
            return Err(Error::InvalidInput(
                "runtime.execution_backend colima requires runtime.provider colima".to_string(),
            ));
        }
        let profile = super::workspace_runtime::configured_colima_profile(
            &self.config.runtime,
            &self.config.workspace.id.0,
        )?;
        let instance = super::workspace_runtime::colima_lima_instance(&profile);
        let test_override = toolchain_override.is_some();
        let toolchain = match toolchain_override {
            Some(toolchain) => toolchain,
            None => super::workspace_runtime_toolchain::ColimaToolchain::resolve(false)?,
        };
        if !test_override && !toolchain.state_is_ready() {
            return Err(Error::InvalidInput(
                "Trail's isolated Colima state is unavailable; run `trail env runtime setup colima --execution-backend colima`"
                    .to_string(),
            ));
        }
        if !test_override && !toolchain.contained_profile_verified(&profile) {
            return Err(Error::InvalidInput(format!(
                "Colima profile `{profile}` lacks Trail's no-host-mount containment receipt; stop it and rerun `trail env runtime setup colima --profile {profile} --execution-backend colima`"
            )));
        }
        preflight_colima_guest_execution(&toolchain, &profile)?;

        let limits = projection_limits(&self.config.workspace_views);
        let staging_root = self.db_dir.join("tmp/managed-execution");
        ensure_private_staging_root(&staging_root)?;
        let staging = tempfile::Builder::new()
            .prefix("colima-")
            .tempdir_in(&staging_root)?;
        let projection = build_projection(Path::new(&view.mountpoint), staging.path(), limits)
            .map_err(|error| Error::ExecutionValidation {
                execution_id: context.execution_id.clone(),
                reason: redact_sensitive_text(&error.to_string()),
            })?;
        let source_ignore = lane_workdir_ignore_matcher(Path::new(&view.mountpoint))?;
        let workspace_key = &sha256_hex(self.config.workspace.id.0.as_bytes())[..16];
        let guest_namespace = format!(
            "{GUEST_EXECUTION_ROOT}/{workspace_key}/{}",
            context.execution_id
        );
        let guest_workspace = format!("{guest_namespace}/workspace");
        let guest_home = format!("{guest_namespace}/home");
        let guest_tmp = format!("{guest_namespace}/tmp");
        let manifest_path = guest_manifest_path(self, &context.execution_id)?;
        let cancellation_path = guest_cancellation_path(self, &context.execution_id)?;
        let mut manifest = GuestExecutionManifest {
            schema: GUEST_MANIFEST_SCHEMA,
            execution_id: context.execution_id.clone(),
            lane_id: context.lane_id.clone(),
            profile: profile.clone(),
            lima_instance: instance.clone(),
            guest_namespace: guest_namespace.clone(),
            staging_path: staging.path().to_string_lossy().into_owned(),
            owner_pid: std::process::id(),
            owner_start_token: current_process_start_token(),
            phase: "creating".to_string(),
            input_digest: projection.input_digest.clone(),
            candidate_digest: None,
            imported_paths: Vec::new(),
            removed_paths: Vec::new(),
            checkpoint_root: None,
            checkpoint_operation: None,
            error: None,
            cancellation_requested_at: None,
            cancellation_completed_at: None,
            process_group_terminated: false,
            updated_at: now_ts(),
        };
        {
            // Recovery, admission, and publication share one workspace fence so
            // concurrent callers cannot all observe the same free execution slot.
            let _manifest_fence =
                Trail::with_write_lock_wait(GUEST_PROTOCOL_TIMEOUT, || self.acquire_write_lock())?;
            recover_guest_execution_manifests(self, &toolchain, &profile, &instance)?;
            write_guest_manifest(&manifest_path, &manifest)?;
        }
        context.guest_manifest_path = Some(manifest_path.clone());
        self.set_managed_execution_sandbox_preparation(
            context,
            ManagedExecutionSandboxPreparationReceipt {
                backend: "colima".to_string(),
                provider: "colima".to_string(),
                profile: profile.clone(),
                lima_instance: instance.clone(),
                guest_namespace: guest_namespace.clone(),
                toolchain_source: toolchain.source.to_string(),
                toolchain_version: toolchain.version.clone(),
                input_digest: projection.input_digest.clone(),
                projected_entries: projection.projected_entries,
                projected_bytes: projection.projected_bytes,
                entry_limit: limits.entries,
                total_bytes_limit: limits.total_bytes,
                file_bytes_limit: limits.file_bytes,
                service_bindings: guest_service_binding_identities(&context.environment)?,
            },
        );

        let mut namespace_created = false;
        let execution_result = (|| {
            require_guest_status(
                &toolchain,
                &instance,
                &[
                    "mkdir".to_string(),
                    "-p".to_string(),
                    "--".to_string(),
                    guest_workspace.clone(),
                    guest_home.clone(),
                    guest_tmp.clone(),
                ],
                None,
                None,
            )?;
            namespace_created = true;
            update_guest_manifest(&manifest_path, &mut manifest, "namespace_created", None)?;
            let archive = File::open(&projection.archive_path)?;
            require_guest_status(
                &toolchain,
                &instance,
                &[
                    "tar".to_string(),
                    "-xpf".to_string(),
                    "-".to_string(),
                    "-C".to_string(),
                    guest_workspace.clone(),
                ],
                Some(Stdio::from(archive)),
                None,
            )?;
            update_guest_manifest(&manifest_path, &mut manifest, "projected", None)?;
            self.record_managed_execution_context_phase(
                context,
                "guest_project",
                "succeeded",
                None,
                Some(serde_json::json!({
                    "backend": "colima",
                    "profile": profile,
                    "lima_instance": instance,
                    "guest_namespace": guest_namespace,
                    "input_digest": projection.input_digest,
                    "entries": projection.projected_entries,
                    "bytes": projection.projected_bytes,
                })),
            )?;

            let environment =
                guest_environment(context, view, &guest_workspace, &guest_home, &guest_tmp)?;
            let mut guest_args = vec!["env".to_string(), "-i".to_string()];
            guest_args.extend(environment);
            guest_args.extend(command.iter().cloned());
            update_guest_manifest(&manifest_path, &mut manifest, "executing", None)?;
            let run = run_guest_command(
                &toolchain,
                &instance,
                &guest_args,
                &guest_workspace,
                timeout,
                &cancellation_path,
                &manifest,
            )?;
            if run.cancelled {
                let request = read_guest_cancellation_request(&cancellation_path, &manifest)?
                    .ok_or_else(|| {
                        Error::Corrupt(
                            "managed guest command reported cancellation without a request"
                                .to_string(),
                        )
                    })?;
                manifest.cancellation_requested_at = Some(request.requested_at);
                manifest.cancellation_completed_at = Some(now_ts());
                manifest.process_group_terminated = run.process_group_terminated;
                update_guest_manifest(&manifest_path, &mut manifest, "cancelled", None)?;
                self.mark_managed_lane_execution_command(
                    context,
                    "cancelled",
                    None,
                    run.exit_code,
                )?;
                *command_marked = true;
                return Err(Error::ExecutionCancelled {
                    execution_id: context.execution_id.clone(),
                });
            }
            update_guest_manifest(&manifest_path, &mut manifest, "executed", None)?;
            self.mark_managed_lane_execution_command(
                context,
                if run.success { "succeeded" } else { "failed" },
                (!run.success && run.exit_code.is_none())
                    .then(|| String::from_utf8_lossy(&run.stderr).trim().to_string())
                    .as_deref(),
                run.exit_code,
            )?;
            *command_marked = true;

            if let Some(request) = read_guest_cancellation_request(&cancellation_path, &manifest)? {
                manifest.cancellation_requested_at = Some(request.requested_at);
                manifest.cancellation_completed_at = Some(now_ts());
                update_guest_manifest(&manifest_path, &mut manifest, "cancelled", None)?;
                return Err(Error::ExecutionCancelled {
                    execution_id: context.execution_id.clone(),
                });
            }

            let candidate_archive = staging.path().join("candidate.tar");
            let candidate_file = File::create(&candidate_archive)?;
            let candidate_allowance = limits.total_bytes.saturating_add(16 * 1024 * 1024);
            export_guest_archive(
                &toolchain,
                &instance,
                &guest_workspace,
                candidate_file,
                candidate_allowance,
            )?;
            let archive_bytes = fs::metadata(&candidate_archive)?.len();
            if archive_bytes > candidate_allowance {
                return Err(Error::InvalidInput(format!(
                    "guest candidate archive is {archive_bytes} bytes, exceeding its bounded export allowance"
                )));
            }
            let candidate_root = staging.path().join("candidate");
            fs::create_dir(&candidate_root)?;
            let candidate = validate_and_extract_candidate(
                &candidate_archive,
                &candidate_root,
                limits,
                &source_ignore,
            )
            .map_err(|error| Error::ExecutionValidation {
                execution_id: context.execution_id.clone(),
                reason: redact_sensitive_text(&error.to_string()),
            })?;
            manifest.candidate_digest = Some(candidate.output_digest.clone());
            update_guest_manifest(&manifest_path, &mut manifest, "exported", None)?;
            self.record_managed_execution_context_phase(
                context,
                "guest_export",
                "succeeded",
                None,
                Some(serde_json::json!({
                    "output_digest": candidate.output_digest,
                    "archive_bytes": archive_bytes,
                })),
            )?;

            let _cancellation_fence =
                Trail::with_write_lock_wait(GUEST_CANCELLATION_WAIT, || self.acquire_write_lock())?;
            update_guest_manifest(&manifest_path, &mut manifest, "importing", None)?;
            if let Some(request) = read_guest_cancellation_request(&cancellation_path, &manifest)? {
                manifest.cancellation_requested_at = Some(request.requested_at);
                manifest.cancellation_completed_at = Some(now_ts());
                update_guest_manifest(&manifest_path, &mut manifest, "cancelled", None)?;
                return Err(Error::ExecutionCancelled {
                    execution_id: context.execution_id.clone(),
                });
            }

            let current =
                source_snapshot_with_ignore(Path::new(&view.mountpoint), limits, &source_ignore)?;
            if current != projection.source_snapshot {
                return Err(Error::ExecutionValidation {
                    execution_id: context.execution_id.clone(),
                    reason: "lane source changed on the host while its Colima execution was running; refusing to overwrite concurrent work"
                        .to_string(),
                });
            }
            let (imported_paths, removed_paths) = apply_candidate_source(
                Path::new(&view.mountpoint),
                &candidate_root,
                &projection.source_snapshot,
                &candidate.source_snapshot,
            )?;
            let unchanged = imported_paths.is_empty() && removed_paths.is_empty();
            manifest.imported_paths = imported_paths.clone();
            manifest.removed_paths = removed_paths.clone();
            update_guest_manifest(&manifest_path, &mut manifest, "imported", None)?;
            self.record_managed_execution_context_phase(
                context,
                "guest_import",
                if unchanged { "skipped" } else { "succeeded" },
                None,
                Some(serde_json::json!({
                    "output_digest": candidate.output_digest,
                    "imported_paths": imported_paths,
                    "removed_paths": removed_paths,
                })),
            )?;
            Ok((
                run,
                candidate.output_digest,
                imported_paths,
                removed_paths,
                unchanged,
            ))
        })();

        let cleanup = if namespace_created {
            cleanup_guest_namespace(&toolchain, &instance, &guest_namespace)
        } else {
            Ok(())
        };
        let cleanup_error = cleanup.as_ref().err().map(ToString::to_string);
        if cleanup_error.is_none() && namespace_created {
            let _ = update_guest_manifest(&manifest_path, &mut manifest, "cleaned", None);
        } else if let Some(error) = cleanup_error.as_deref() {
            let _ =
                update_guest_manifest(&manifest_path, &mut manifest, "cleanup_failed", Some(error));
        }
        let _ = self.record_managed_execution_context_phase(
            context,
            "guest_cleanup",
            if cleanup_error.is_some() {
                "failed"
            } else if namespace_created {
                "succeeded"
            } else {
                "skipped"
            },
            cleanup_error.as_deref(),
            Some(serde_json::json!({"guest_namespace": guest_namespace})),
        );
        if manifest.cancellation_completed_at.is_some() {
            let _ = fs::remove_file(&cancellation_path);
        }

        match execution_result {
            Ok((run, output_digest, imported_paths, removed_paths, unchanged)) => {
                self.set_managed_execution_sandbox_finalization(
                    context,
                    ManagedExecutionSandboxFinalizationReceipt {
                        output_digest,
                        imported_paths,
                        removed_paths,
                        unchanged,
                        cleanup_status: if cleanup_error.is_some() {
                            "failed"
                        } else {
                            "succeeded"
                        }
                        .to_string(),
                        cleanup_error: cleanup_error.clone(),
                    },
                );
                if let Some(error) = cleanup_error {
                    Err(Error::Corrupt(format!(
                        "guest command completed but its owned namespace cleanup failed: {error}"
                    )))
                } else {
                    Ok(run)
                }
            }
            Err(error) => {
                if let Some(cleanup_error) = cleanup_error {
                    Err(Error::Corrupt(format!(
                        "{error}; guest namespace cleanup also failed: {cleanup_error}"
                    )))
                } else {
                    Err(error)
                }
            }
        }
    }
}

fn classify_guest_execution_error(context: &ManagedExecutionContext, error: Error) -> Error {
    if matches!(
        &error,
        Error::ExecutionCancelled { .. }
            | Error::ExecutionValidation { .. }
            | Error::ExecutionInfrastructure { .. }
    ) {
        return error;
    }
    let phase = context
        .guest_manifest_path
        .as_deref()
        .and_then(|path| read_guest_manifest(path).ok())
        .map(|manifest| manifest.phase)
        .unwrap_or_else(|| "guest_prepare".to_string());
    Error::ExecutionInfrastructure {
        execution_id: context.execution_id.clone(),
        phase,
        reason: redact_sensitive_text(&error.to_string()),
    }
}

#[derive(Default)]
struct GuestManifestInspection {
    active: usize,
    live: usize,
    recoverable: usize,
    ambiguous: usize,
    terminal: usize,
    phases: BTreeMap<String, usize>,
}

fn inspect_guest_execution_manifests(db: &Trail) -> Result<GuestManifestInspection> {
    let directory = db.db_dir.join("managed-executions");
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(GuestManifestInspection::default());
        }
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(Error::Corrupt(
                "managed guest execution receipt path is not a real directory".to_string(),
            ));
        }
        Err(error) => return Err(error.into()),
    }
    let mut inspection = GuestManifestInspection::default();
    let scan_limit = MAX_GUEST_MANIFESTS + MAX_RETAINED_TERMINAL_GUEST_MANIFESTS + 1;
    let mut scanned = 0_usize;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        scanned = scanned.saturating_add(1);
        if scanned > scan_limit {
            return Err(Error::InvalidInput(format!(
                "managed guest manifest count exceeds the doctor scan limit of {scan_limit}"
            )));
        }
        let manifest = read_guest_manifest(&path)?;
        validate_guest_manifest_identity(db, &manifest)?;
        *inspection.phases.entry(manifest.phase.clone()).or_default() += 1;
        if manifest.phase.starts_with("terminal_") {
            inspection.terminal = inspection.terminal.saturating_add(1);
        } else {
            inspection.active = inspection.active.saturating_add(1);
            if process_matches_start_token(manifest.owner_pid, &manifest.owner_start_token) {
                inspection.live = inspection.live.saturating_add(1);
            } else if guest_manifest_is_safely_discardable(&manifest) {
                inspection.recoverable = inspection.recoverable.saturating_add(1);
            } else {
                inspection.ambiguous = inspection.ambiguous.saturating_add(1);
            }
        }
    }
    Ok(inspection)
}

pub(super) fn finalize_guest_execution_manifest(
    db: &Trail,
    context: &ManagedExecutionContext,
    checkpoint: Option<&WorkspaceCheckpointReport>,
    checkpoint_error: Option<&str>,
) -> Result<()> {
    let Some(path) = context.guest_manifest_path.as_deref() else {
        return Ok(());
    };
    let mut manifest = read_guest_manifest(path)?;
    validate_guest_manifest_identity(db, &manifest)?;
    if manifest.execution_id != context.execution_id
        || manifest.lane_id != context.lane_id
        || manifest.owner_pid != std::process::id()
        || manifest.owner_start_token != current_process_start_token()
    {
        return Err(Error::Corrupt(format!(
            "managed guest manifest `{}` no longer belongs to execution `{}`",
            path.display(),
            context.execution_id
        )));
    }
    manifest.checkpoint_root = checkpoint.map(|checkpoint| checkpoint.root_id.0.clone());
    manifest.checkpoint_operation = checkpoint
        .and_then(|checkpoint| checkpoint.operation.as_ref())
        .map(|operation| operation.0.clone());
    let cleanup_failed = context
        .sandbox_finalization
        .as_ref()
        .is_some_and(|receipt| receipt.cleanup_status == "failed");
    let terminal_phase = if manifest.cancellation_completed_at.is_some() && !cleanup_failed {
        "terminal_cancelled"
    } else if checkpoint.is_some() && checkpoint_error.is_none() && !cleanup_failed {
        "terminal_succeeded"
    } else {
        "terminal_failed"
    };
    update_guest_manifest(path, &mut manifest, terminal_phase, checkpoint_error)
}

fn guest_manifest_directory(db: &Trail) -> Result<PathBuf> {
    let directory = db.db_dir.join("managed-executions");
    ensure_private_staging_root(&directory)?;
    Ok(directory)
}

fn guest_manifest_path(db: &Trail, execution_id: &str) -> Result<PathBuf> {
    if !execution_id.starts_with("exec_")
        || !execution_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(Error::InvalidInput(
            "managed guest execution id is not safe for durable storage".to_string(),
        ));
    }
    Ok(guest_manifest_directory(db)?.join(format!("{execution_id}.json")))
}

fn guest_cancellation_path(db: &Trail, execution_id: &str) -> Result<PathBuf> {
    let _ = guest_manifest_path(db, execution_id)?;
    Ok(guest_manifest_directory(db)?.join(format!("{execution_id}.cancel")))
}

fn read_guest_cancellation_request(
    path: &Path,
    manifest: &GuestExecutionManifest,
) -> Result<Option<GuestCancellationRequest>> {
    let metadata = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Ok(metadata) => metadata,
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_GUEST_MANIFEST_BYTES
    {
        return Err(Error::Corrupt(format!(
            "managed guest cancellation request `{}` is not a bounded regular file",
            path.display()
        )));
    }
    let request = serde_json::from_slice::<GuestCancellationRequest>(&fs::read(path)?)?;
    if request.schema != GUEST_MANIFEST_SCHEMA
        || request.execution_id != manifest.execution_id
        || request.lane_id != manifest.lane_id
    {
        return Err(Error::Corrupt(format!(
            "managed guest cancellation request `{}` does not match its execution receipt",
            path.display()
        )));
    }
    Ok(Some(request))
}

fn write_guest_manifest(path: &Path, manifest: &GuestExecutionManifest) -> Result<()> {
    write_file_atomic(path, &serde_json::to_vec_pretty(manifest)?, true)
}

fn update_guest_manifest(
    path: &Path,
    manifest: &mut GuestExecutionManifest,
    phase: &str,
    error: Option<&str>,
) -> Result<()> {
    manifest.phase = phase.to_string();
    manifest.error = error.map(redact_sensitive_text);
    manifest.updated_at = now_ts();
    write_guest_manifest(path, manifest)
}

fn read_guest_manifest(path: &Path) -> Result<GuestExecutionManifest> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_GUEST_MANIFEST_BYTES
    {
        return Err(Error::Corrupt(format!(
            "managed guest manifest `{}` is not a bounded regular file",
            path.display()
        )));
    }
    let manifest = serde_json::from_slice::<GuestExecutionManifest>(&fs::read(path)?)?;
    if manifest.schema != GUEST_MANIFEST_SCHEMA {
        return Err(Error::Corrupt(format!(
            "managed guest manifest `{}` has unsupported schema {}",
            path.display(),
            manifest.schema
        )));
    }
    Ok(manifest)
}

fn validate_guest_manifest_identity(db: &Trail, manifest: &GuestExecutionManifest) -> Result<()> {
    let expected = format!(
        "{GUEST_EXECUTION_ROOT}/{}/{}",
        &sha256_hex(db.config.workspace.id.0.as_bytes())[..16],
        manifest.execution_id
    );
    if manifest.guest_namespace != expected {
        return Err(Error::Corrupt(format!(
            "managed guest manifest `{}` has namespace `{}` instead of `{expected}`",
            manifest.execution_id, manifest.guest_namespace
        )));
    }
    if !manifest.execution_id.starts_with("exec_")
        || !manifest
            .execution_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(Error::Corrupt(
            "managed guest manifest has an invalid execution id".to_string(),
        ));
    }
    Ok(())
}

fn recover_guest_execution_manifests(
    db: &Trail,
    toolchain: &super::workspace_runtime_toolchain::ColimaToolchain,
    profile: &str,
    instance: &str,
) -> Result<()> {
    let directory = guest_manifest_directory(db)?;
    let mut manifests = Vec::new();
    let mut terminal = BTreeMap::<(i64, String), PathBuf>::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let manifest = read_guest_manifest(&path)?;
        validate_guest_manifest_identity(db, &manifest)?;
        if manifest.phase.starts_with("terminal_") {
            let file_name = entry.file_name().into_string().map_err(|_| {
                Error::Corrupt("managed guest manifest name is not Unicode".to_string())
            })?;
            terminal.insert((manifest.updated_at, file_name), path);
            if terminal.len() > MAX_RETAINED_TERMINAL_GUEST_MANIFESTS {
                let key = terminal.keys().next().cloned().ok_or_else(|| {
                    Error::Corrupt("terminal guest manifest retention is empty".to_string())
                })?;
                let retired = terminal.remove(&key).ok_or_else(|| {
                    Error::Corrupt("terminal guest manifest retention changed".to_string())
                })?;
                fs::remove_file(retired)?;
            }
            continue;
        }
        if manifests.len() >= MAX_GUEST_MANIFESTS {
            return Err(Error::InvalidInput(format!(
                "active managed guest manifest count exceeds the recovery limit of {MAX_GUEST_MANIFESTS}"
            )));
        }
        manifests.push((entry.file_name(), path, manifest));
    }
    manifests.sort_by(|left, right| left.0.cmp(&right.0));
    let mut live = 0_usize;
    for (_, path, mut manifest) in manifests {
        if manifest.profile != profile || manifest.lima_instance != instance {
            continue;
        }
        if process_matches_start_token(manifest.owner_pid, &manifest.owner_start_token) {
            live = live.saturating_add(1);
            continue;
        }
        let cancellation_path = guest_cancellation_path(db, &manifest.execution_id)?;
        if let Some(request) = read_guest_cancellation_request(&cancellation_path, &manifest)? {
            let process_group_path = format!("{}/process-group", manifest.guest_namespace);
            manifest.process_group_terminated =
                terminate_guest_process_group(toolchain, instance, &process_group_path, false)?;
            cleanup_guest_namespace(toolchain, instance, manifest.guest_namespace.as_str())?;
            manifest.cancellation_requested_at = Some(request.requested_at);
            manifest.cancellation_completed_at = Some(now_ts());
            update_guest_manifest(&path, &mut manifest, "terminal_cancelled", None)?;
            fs::remove_file(cancellation_path)?;
            continue;
        }
        if !guest_manifest_is_safely_discardable(&manifest) {
            return Err(Error::InvalidInput(format!(
                "managed guest execution `{}` was interrupted in phase `{}`; Trail preserved lane and candidate state and will not guess. Inspect the lane, run `trail lane checkpoint {}`, then retry or remove the recovered execution through doctor tooling",
                manifest.execution_id, manifest.phase, manifest.lane_id
            )));
        }
        cleanup_guest_namespace(toolchain, instance, manifest.guest_namespace.as_str())?;
        update_guest_manifest(&path, &mut manifest, "terminal_recovered_discarded", None)?;
    }
    if live >= MAX_CONCURRENT_GUEST_EXECUTIONS {
        return Err(Error::InvalidInput(format!(
            "workspace already has {live} live Colima managed executions, reaching the limit of {MAX_CONCURRENT_GUEST_EXECUTIONS}"
        )));
    }
    Ok(())
}

fn guest_manifest_is_safely_discardable(manifest: &GuestExecutionManifest) -> bool {
    let imported = !manifest.imported_paths.is_empty() || !manifest.removed_paths.is_empty();
    matches!(
        manifest.phase.as_str(),
        "creating"
            | "namespace_created"
            | "projected"
            | "executed"
            | "cancelled"
            | "cleanup_failed"
    ) || (manifest.phase == "cleaned" && !imported)
}

fn guest_manifest_is_cancellable(manifest: &GuestExecutionManifest) -> bool {
    matches!(
        manifest.phase.as_str(),
        "creating" | "namespace_created" | "projected" | "executing" | "executed" | "exported"
    )
}

fn guest_cancellation_is_terminal(manifest: &GuestExecutionManifest) -> bool {
    manifest.cancellation_completed_at.is_some()
        && matches!(
            manifest.phase.as_str(),
            "cleaned" | "cleanup_failed" | "terminal_cancelled" | "terminal_failed"
        )
}

fn projection_limits(config: &WorkspaceViewsConfig) -> ProjectionLimits {
    ProjectionLimits {
        entries: nonzero_or(config.upper_file_count, DEFAULT_MAX_PROJECTION_ENTRIES),
        total_bytes: nonzero_or(config.upper_logical_bytes, DEFAULT_MAX_PROJECTION_BYTES),
        file_bytes: nonzero_or(config.single_file_bytes, DEFAULT_MAX_FILE_BYTES),
    }
}

fn guest_service_binding_identities(environment: &[(String, String)]) -> Result<Vec<String>> {
    let Some((_, services)) = environment
        .iter()
        .find(|(name, _)| name == "TRAIL_SERVICES_JSON")
    else {
        return Ok(Vec::new());
    };
    let value: serde_json::Value = serde_json::from_str(services).map_err(|error| {
        Error::Corrupt(format!(
            "managed runtime service bindings are not valid JSON: {error}"
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        Error::Corrupt("managed runtime service bindings are not an object".to_string())
    })?;
    let mut identities = object.keys().cloned().collect::<Vec<_>>();
    identities.sort();
    Ok(identities)
}

fn nonzero_or(value: u64, fallback: u64) -> u64 {
    if value == 0 {
        fallback
    } else {
        value
    }
}

fn ensure_private_staging_root(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(Error::InvalidPath {
            path: path.to_string_lossy().into_owned(),
            reason: "managed execution staging root must be a real directory".to_string(),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn build_projection(
    root: &Path,
    staging: &Path,
    limits: ProjectionLimits,
) -> Result<BuiltProjection> {
    let ignore = lane_workdir_ignore_matcher(root)?;
    let archive_path = staging.join("input.tar");
    let archive_file = File::create(&archive_path)?;
    let mut builder = tar::Builder::new(archive_file);
    builder.follow_symlinks(false);
    let mut projected_entries = 0_u64;
    let mut projected_bytes = 0_u64;
    let mut folded_paths = BTreeSet::new();
    let mut entries = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| Error::InvalidInput(error.to_string()))?;
    entries.sort_by(|left, right| left.path().cmp(right.path()));
    for entry in entries {
        let relative = entry.path().strip_prefix(root).map_err(|_| {
            Error::Corrupt(format!(
                "projection entry `{}` escaped `{}`",
                entry.path().display(),
                root.display()
            ))
        })?;
        if relative.as_os_str().is_empty() || entry.file_type().is_dir() {
            continue;
        }
        let relative = relative.to_str().ok_or_else(|| Error::InvalidPath {
            path: relative.to_string_lossy().into_owned(),
            reason: "guest projections require Unicode paths".to_string(),
        })?;
        let relative = normalize_relative_path(&relative.replace(std::path::MAIN_SEPARATOR, "/"))?;
        if ignore
            .matched_path_or_any_parents(path_from_rel(&relative), entry.file_type().is_dir())
            .is_ignore()
        {
            continue;
        }
        let class = classify_view_path(&relative);
        if matches!(
            class,
            ViewPathClass::Internal | ViewPathClass::Secret | ViewPathClass::Scratch
        ) {
            continue;
        }
        validate_projection_path_collision(&mut folded_paths, &relative)?;
        projected_entries = projected_entries.saturating_add(1);
        if projected_entries > limits.entries {
            return Err(Error::InvalidInput(format!(
                "guest projection exceeds the {}-entry limit",
                limits.entries
            )));
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        let mode = portable_mode(&metadata);
        let mut header = tar::Header::new_gnu();
        header.set_path(&relative)?;
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_mode(mode);
        if metadata.file_type().is_file() {
            if metadata.len() > limits.file_bytes {
                return Err(Error::InvalidInput(format!(
                    "guest projection file `{relative}` is {} bytes, exceeding the {}-byte file limit",
                    metadata.len(), limits.file_bytes
                )));
            }
            projected_bytes = projected_bytes.saturating_add(metadata.len());
            if projected_bytes > limits.total_bytes {
                return Err(Error::InvalidInput(format!(
                    "guest projection exceeds the {}-byte total limit",
                    limits.total_bytes
                )));
            }
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(metadata.len());
            header.set_cksum();
            let file = File::open(entry.path())?;
            builder.append(&header, file)?;
        } else if metadata.file_type().is_symlink() {
            let target = fs::read_link(entry.path())?;
            validate_relative_symlink(&relative, &target)?;
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_link_name(&target)?;
            header.set_cksum();
            builder.append(&header, io::empty())?;
        } else {
            return Err(Error::InvalidPath {
                path: relative,
                reason: "guest projections support only regular files, directories, and relative symlinks"
                    .to_string(),
            });
        }
    }
    builder.finish()?;
    drop(builder);
    let source_snapshot = source_snapshot_with_ignore(root, limits, &ignore)?;
    let input_digest = snapshot_digest(&source_snapshot)?;
    Ok(BuiltProjection {
        archive_path,
        input_digest,
        projected_entries,
        projected_bytes,
        source_snapshot,
    })
}

fn validate_and_extract_candidate(
    archive_path: &Path,
    output_root: &Path,
    limits: ProjectionLimits,
    source_ignore: &ignore::gitignore::Gitignore,
) -> Result<CandidateProjection> {
    let mut archive = tar::Archive::new(File::open(archive_path)?);
    let mut count = 0_u64;
    let mut total_bytes = 0_u64;
    let mut folded_paths = BTreeSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let raw_path = entry.path()?;
        let raw_path = raw_path.to_str().ok_or_else(|| Error::InvalidPath {
            path: raw_path.to_string_lossy().into_owned(),
            reason: "guest candidate paths must be Unicode".to_string(),
        })?;
        let trimmed = raw_path.trim_start_matches("./");
        if trimmed.is_empty() {
            continue;
        }
        let relative = normalize_relative_path(trimmed)?;
        let class = classify_view_path(&relative);
        if matches!(class, ViewPathClass::Internal | ViewPathClass::Secret) {
            return Err(Error::InvalidPath {
                path: relative,
                reason: "guest candidate contains a private or internal path".to_string(),
            });
        }
        validate_projection_path_collision(&mut folded_paths, &relative)?;
        count = count.saturating_add(1);
        if count > limits.entries {
            return Err(Error::InvalidInput(format!(
                "guest candidate exceeds the {}-entry limit",
                limits.entries
            )));
        }
        let destination = safe_join(output_root, &relative)?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            fs::create_dir_all(&destination)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        if entry_type.is_file() {
            let size = entry.header().size()?;
            if size > limits.file_bytes {
                return Err(Error::InvalidInput(format!(
                    "guest candidate file `{relative}` is {size} bytes, exceeding the {}-byte file limit",
                    limits.file_bytes
                )));
            }
            total_bytes = total_bytes.saturating_add(size);
            if total_bytes > limits.total_bytes {
                return Err(Error::InvalidInput(format!(
                    "guest candidate exceeds the {}-byte total limit",
                    limits.total_bytes
                )));
            }
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&destination)?;
            let copied = io::copy(&mut entry, &mut output)?;
            if copied != size {
                return Err(Error::Corrupt(format!(
                    "guest candidate file `{relative}` declared {size} bytes but yielded {copied}"
                )));
            }
            set_portable_mode(&destination, entry.header().mode()?)?;
        } else if entry_type.is_symlink() {
            let target = entry
                .link_name()?
                .ok_or_else(|| Error::InvalidPath {
                    path: relative.clone(),
                    reason: "guest candidate symlink has no target".to_string(),
                })?
                .into_owned();
            validate_relative_symlink(&relative, &target)?;
            create_relative_symlink(&target, &destination)?;
        } else {
            return Err(Error::InvalidPath {
                path: relative,
                reason: "guest candidate contains an unsupported archive entry type".to_string(),
            });
        }
    }
    let source_snapshot = source_snapshot_with_ignore(output_root, limits, source_ignore)?;
    let output_digest = snapshot_digest(&source_snapshot)?;
    Ok(CandidateProjection {
        output_digest,
        source_snapshot,
    })
}

#[cfg(test)]
fn source_snapshot(
    root: &Path,
    limits: ProjectionLimits,
) -> Result<BTreeMap<String, SnapshotEntry>> {
    let ignore = lane_workdir_ignore_matcher(root)?;
    source_snapshot_with_ignore(root, limits, &ignore)
}

fn source_snapshot_with_ignore(
    root: &Path,
    limits: ProjectionLimits,
    ignore: &ignore::gitignore::Gitignore,
) -> Result<BTreeMap<String, SnapshotEntry>> {
    let mut snapshot = BTreeMap::new();
    let mut count = 0_u64;
    let mut total_bytes = 0_u64;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| Error::InvalidInput(error.to_string()))?;
        if entry.file_type().is_dir() {
            continue;
        }
        let relative = entry.path().strip_prefix(root).map_err(|_| {
            Error::Corrupt(format!(
                "snapshot entry `{}` escaped its root",
                entry.path().display()
            ))
        })?;
        let Some(relative) = relative.to_str() else {
            return Err(Error::InvalidPath {
                path: relative.to_string_lossy().into_owned(),
                reason: "managed execution snapshots require Unicode paths".to_string(),
            });
        };
        let relative = normalize_relative_path(&relative.replace(std::path::MAIN_SEPARATOR, "/"))?;
        if ignore
            .matched_path_or_any_parents(path_from_rel(&relative), entry.file_type().is_dir())
            .is_ignore()
        {
            continue;
        }
        if classify_view_path(&relative) != ViewPathClass::Source {
            continue;
        }
        count = count.saturating_add(1);
        if count > limits.entries {
            return Err(Error::InvalidInput(
                "source snapshot entry limit exceeded".to_string(),
            ));
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        let mode = portable_mode(&metadata);
        let snapshot_entry = if metadata.file_type().is_file() {
            if metadata.len() > limits.file_bytes {
                return Err(Error::InvalidInput(format!(
                    "source file `{relative}` exceeds the managed execution file limit"
                )));
            }
            total_bytes = total_bytes.saturating_add(metadata.len());
            if total_bytes > limits.total_bytes {
                return Err(Error::InvalidInput(
                    "source snapshot byte limit exceeded".to_string(),
                ));
            }
            SnapshotEntry {
                kind: "file".to_string(),
                content: sha256_file_hex(entry.path())?,
                mode,
            }
        } else if metadata.file_type().is_symlink() {
            let target = fs::read_link(entry.path())?;
            validate_relative_symlink(&relative, &target)?;
            SnapshotEntry {
                kind: "symlink".to_string(),
                content: target.to_string_lossy().into_owned(),
                mode,
            }
        } else {
            return Err(Error::InvalidPath {
                path: relative,
                reason: "source snapshot contains an unsupported file kind".to_string(),
            });
        };
        if snapshot.insert(relative.clone(), snapshot_entry).is_some() {
            return Err(Error::InvalidPath {
                path: relative,
                reason: "source snapshot contains a duplicate path".to_string(),
            });
        }
    }
    Ok(snapshot)
}

fn snapshot_digest(snapshot: &BTreeMap<String, SnapshotEntry>) -> Result<String> {
    Ok(sha256_hex(&serde_json::to_vec(snapshot)?))
}

fn apply_candidate_source(
    mountpoint: &Path,
    candidate_root: &Path,
    input: &BTreeMap<String, SnapshotEntry>,
    candidate: &BTreeMap<String, SnapshotEntry>,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut removed = input
        .keys()
        .filter(|path| !candidate.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>();
    removed.sort_by(|left, right| {
        right
            .matches('/')
            .count()
            .cmp(&left.matches('/').count())
            .then(right.cmp(left))
    });
    for relative in &removed {
        let destination = safe_join(mountpoint, relative)?;
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                fs::remove_file(destination)?;
            }
            Ok(_) => {
                return Err(Error::InvalidPath {
                    path: relative.clone(),
                    reason: "candidate deletion would remove a non-file entry".to_string(),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    let mut imported = candidate
        .iter()
        .filter(|(path, entry)| input.get(*path) != Some(*entry))
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    imported.sort();
    for relative in &imported {
        let source = safe_join(candidate_root, relative)?;
        let destination = safe_join(mountpoint, relative)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                fs::remove_file(&destination)?;
            }
            Ok(_) => {
                return Err(Error::InvalidPath {
                    path: relative.clone(),
                    reason: "candidate file would replace a directory or special entry".to_string(),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = fs::symlink_metadata(&source)?;
        if metadata.file_type().is_file() {
            let bytes = fs::read(&source)?;
            write_file_atomic(&destination, &bytes, false)?;
            set_portable_mode(&destination, portable_mode(&metadata))?;
        } else if metadata.file_type().is_symlink() {
            let target = fs::read_link(&source)?;
            validate_relative_symlink(relative, &target)?;
            create_relative_symlink(&target, &destination)?;
        } else {
            return Err(Error::InvalidPath {
                path: relative.clone(),
                reason: "validated candidate changed file kind before import".to_string(),
            });
        }
    }
    Ok((imported, removed))
}

fn guest_environment(
    context: &ManagedExecutionContext,
    view: &LaneWorkspaceViewReport,
    guest_workspace: &str,
    guest_home: &str,
    guest_tmp: &str,
) -> Result<Vec<String>> {
    let mountpoint = Path::new(&view.mountpoint);
    let mut environment = BTreeMap::from([
        ("HOME".to_string(), guest_home.to_string()),
        ("TMPDIR".to_string(), guest_tmp.to_string()),
        (
            "PATH".to_string(),
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        ),
        ("TRAIL_EXECUTION_BACKEND".to_string(), "colima".to_string()),
    ]);
    for (name, value) in &context.environment {
        if is_sensitive_json_key(name) || matches!(name.as_str(), "GIT_DIR" | "GIT_INDEX_FILE") {
            continue;
        }
        let translated = if name == "TRAIL_WORKSPACE" {
            guest_workspace.to_string()
        } else if Path::new(value).is_absolute() {
            let relative = Path::new(value).strip_prefix(mountpoint).map_err(|_| {
                Error::InvalidInput(format!(
                    "managed guest environment `{name}` references host path `{value}` outside the lane view"
                ))
            })?;
            if relative.as_os_str().is_empty() {
                guest_workspace.to_string()
            } else {
                let relative = relative.to_str().ok_or_else(|| Error::InvalidPath {
                    path: relative.to_string_lossy().into_owned(),
                    reason: "guest environment paths must be Unicode".to_string(),
                })?;
                format!(
                    "{guest_workspace}/{}",
                    relative.replace(std::path::MAIN_SEPARATOR, "/")
                )
            }
        } else {
            value.clone()
        };
        if translated.as_bytes().contains(&0) || translated.contains('\n') {
            return Err(Error::InvalidInput(format!(
                "managed guest environment `{name}` contains an unsupported value"
            )));
        }
        environment.insert(name.clone(), translated);
    }
    Ok(environment
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect())
}

struct GuestProtocolResult {
    status: ExitStatus,
    stderr: Vec<u8>,
    timed_out: bool,
}

fn run_guest_status(
    toolchain: &super::workspace_runtime_toolchain::ColimaToolchain,
    instance: &str,
    guest_args: &[String],
    stdin: Option<Stdio>,
    workdir: Option<&str>,
) -> Result<GuestProtocolResult> {
    let started = Instant::now();
    let mut process = toolchain.limactl_command();
    process.arg("shell");
    if let Some(workdir) = workdir {
        process.args(["--workdir", workdir]);
    }
    process.arg(instance).arg("--").args(guest_args);
    if let Some(stdin) = stdin {
        process.stdin(stdin);
    } else {
        process.stdin(Stdio::null());
    }
    process.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = process.spawn()?;
    let stderr = child.stderr.take().ok_or_else(|| {
        Error::Corrupt("managed guest protocol command did not expose stderr".to_string())
    })?;
    let stderr_reader =
        std::thread::spawn(move || read_bounded_stream(stderr, MAX_GUEST_DIAGNOSTIC_BYTES));
    let (status, timed_out) = wait_for_child(&mut child, started, Some(GUEST_PROTOCOL_TIMEOUT))?;
    let stderr = stderr_reader.join().map_err(|_| {
        Error::Corrupt("managed guest protocol stderr reader panicked".to_string())
    })??;
    Ok(GuestProtocolResult {
        status,
        stderr,
        timed_out,
    })
}

fn run_guest_command(
    toolchain: &super::workspace_runtime_toolchain::ColimaToolchain,
    instance: &str,
    guest_args: &[String],
    workdir: &str,
    timeout: Option<Duration>,
    cancellation_path: &Path,
    manifest: &GuestExecutionManifest,
) -> Result<CommandRunResult> {
    let started = Instant::now();
    let process_group_path = format!("{}/process-group", manifest.guest_namespace);
    let mut process = toolchain.limactl_command();
    process
        .args(["shell", "--workdir", workdir, instance, "--"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process.args([
        "sh",
        "-c",
        "pidfile=$1; shift; setsid \"$@\" & child=$!; printf '%s\\n' \"$child\" > \"$pidfile\"; wait \"$child\"; status=$?; rm -f -- \"$pidfile\"; exit \"$status\"",
        "trail-guest-launch",
        &process_group_path,
    ]);
    if let Some(timeout) = timeout {
        process.args([
            "timeout",
            "--signal=TERM",
            "--kill-after=5s",
            &format!("{}s", timeout.as_secs()),
        ]);
    }
    process.args(guest_args);
    let mut child = process.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Corrupt("managed guest command did not expose stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Corrupt("managed guest command did not expose stderr".to_string()))?;
    let stdout_reader =
        std::thread::spawn(move || read_bounded_stream(stdout, MAX_GUEST_STDOUT_BYTES));
    let stderr_reader =
        std::thread::spawn(move || read_bounded_stream(stderr, MAX_GUEST_STDERR_BYTES));
    let host_deadline = timeout.map(|timeout| timeout.saturating_add(Duration::from_secs(15)));
    let mut cancelled = false;
    let mut process_group_terminated = false;
    let (status, host_timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, false);
        }
        if !cancelled && read_guest_cancellation_request(cancellation_path, manifest)?.is_some() {
            cancelled = true;
            process_group_terminated =
                terminate_guest_process_group(toolchain, instance, &process_group_path, true)?;
        }
        if cancelled && started.elapsed() >= Duration::from_secs(15) {
            let _ = child.kill();
            break (child.wait()?, false);
        }
        if host_deadline.is_some_and(|deadline| started.elapsed() >= deadline) {
            let _ = child.kill();
            break (child.wait()?, true);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| Error::Corrupt("managed guest stdout reader panicked".to_string()))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| Error::Corrupt("managed guest stderr reader panicked".to_string()))??;
    let exit_code = status.code();
    let guest_timed_out = timeout.is_some() && exit_code == Some(124);
    Ok(CommandRunResult {
        success: status.success(),
        exit_code,
        timed_out: host_timed_out || guest_timed_out,
        cancelled,
        process_group_terminated,
        duration_ms: elapsed_ms(started.elapsed()),
        stdout,
        stderr,
    })
}

fn terminate_guest_process_group(
    toolchain: &super::workspace_runtime_toolchain::ColimaToolchain,
    instance: &str,
    process_group_path: &str,
    wait_for_receipt: bool,
) -> Result<bool> {
    let started = Instant::now();
    let process_group = loop {
        let result = run_guest_capture(
            toolchain,
            instance,
            &[
                "cat".to_string(),
                "--".to_string(),
                process_group_path.to_string(),
            ],
        )?;
        if result.status.success() {
            let value = String::from_utf8_lossy(&result.stdout).trim().to_string();
            let process_group = value.parse::<u32>().map_err(|_| {
                Error::Corrupt("managed guest process-group receipt is invalid".to_string())
            })?;
            if process_group <= 1 {
                return Err(Error::Corrupt(
                    "managed guest process-group receipt is unsafe".to_string(),
                ));
            }
            break process_group;
        }
        if !wait_for_receipt || started.elapsed() >= Duration::from_secs(5) {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let negative_group = format!("-{process_group}");
    let term = run_guest_status(
        toolchain,
        instance,
        &[
            "kill".to_string(),
            "-TERM".to_string(),
            "--".to_string(),
            negative_group.clone(),
        ],
        None,
        None,
    )?;
    if !term.status.success() {
        return Ok(false);
    }
    let wait_started = Instant::now();
    while wait_started.elapsed() < Duration::from_secs(2) {
        let probe = run_guest_status(
            toolchain,
            instance,
            &[
                "kill".to_string(),
                "-0".to_string(),
                "--".to_string(),
                negative_group.clone(),
            ],
            None,
            None,
        )?;
        if !probe.status.success() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = run_guest_status(
        toolchain,
        instance,
        &[
            "kill".to_string(),
            "-KILL".to_string(),
            "--".to_string(),
            negative_group,
        ],
        None,
        None,
    )?;
    Ok(true)
}

struct GuestCaptureResult {
    status: ExitStatus,
    stdout: Vec<u8>,
}

fn run_guest_capture(
    toolchain: &super::workspace_runtime_toolchain::ColimaToolchain,
    instance: &str,
    guest_args: &[String],
) -> Result<GuestCaptureResult> {
    let mut process = toolchain.limactl_command();
    process
        .arg("shell")
        .arg(instance)
        .arg("--")
        .args(guest_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = process.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Corrupt("managed guest capture did not expose stdout".to_string()))?;
    let reader = std::thread::spawn(move || read_bounded_stream(stdout, 64));
    let (status, timed_out) =
        wait_for_child(&mut child, Instant::now(), Some(Duration::from_secs(10)))?;
    let stdout = reader
        .join()
        .map_err(|_| Error::Corrupt("managed guest capture reader panicked".to_string()))??;
    if timed_out {
        return Err(Error::InvalidInput(
            "managed guest process-group inspection timed out".to_string(),
        ));
    }
    Ok(GuestCaptureResult { status, stdout })
}

fn wait_for_child(
    child: &mut Child,
    started: Instant,
    deadline: Option<Duration>,
) -> Result<(ExitStatus, bool)> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, false));
        }
        if deadline.is_some_and(|deadline| started.elapsed() >= deadline) {
            let _ = child.kill();
            return Ok((child.wait()?, true));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn read_bounded_stream(mut reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(output.len());
        let retained = remaining.min(read);
        output.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
    }
    if truncated {
        output.extend_from_slice(b"\n[Trail guest output truncated]\n");
    }
    Ok(output)
}

fn require_guest_status(
    toolchain: &super::workspace_runtime_toolchain::ColimaToolchain,
    instance: &str,
    guest_args: &[String],
    stdin: Option<Stdio>,
    workdir: Option<&str>,
) -> Result<()> {
    let result = run_guest_status(toolchain, instance, guest_args, stdin, workdir)?;
    if result.status.success() {
        Ok(())
    } else {
        let failure = if result.timed_out {
            format!(
                "timed out after {} seconds",
                GUEST_PROTOCOL_TIMEOUT.as_secs()
            )
        } else {
            format!(
                "exited with code {}: {}",
                result.status.code().unwrap_or(128),
                guest_diagnostic(&result.stderr, &[])
            )
        };
        Err(Error::InvalidInput(format!(
            "managed guest protocol command `{}` {failure}",
            guest_args.first().map(String::as_str).unwrap_or("unknown"),
        )))
    }
}

fn export_guest_archive(
    toolchain: &super::workspace_runtime_toolchain::ColimaToolchain,
    instance: &str,
    guest_workspace: &str,
    output: File,
    byte_limit: u64,
) -> Result<()> {
    let started = Instant::now();
    let mut process = toolchain.limactl_command();
    process
        .args([
            "shell",
            instance,
            "--",
            "tar",
            "-cpf",
            "-",
            "-C",
            guest_workspace,
            ".",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = process.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Corrupt("managed guest export did not expose stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Corrupt("managed guest export did not expose stderr".to_string()))?;
    let export_reader =
        std::thread::spawn(move || copy_stream_to_bounded_file(stdout, output, byte_limit));
    let stderr_reader =
        std::thread::spawn(move || read_bounded_stream(stderr, MAX_GUEST_DIAGNOSTIC_BYTES));
    let (status, timed_out) = wait_for_child(&mut child, started, Some(GUEST_PROTOCOL_TIMEOUT))?;
    let export = export_reader
        .join()
        .map_err(|_| Error::Corrupt("managed guest export reader panicked".to_string()))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| Error::Corrupt("managed guest export stderr reader panicked".to_string()))??;
    if timed_out {
        return Err(Error::InvalidInput(format!(
            "managed guest export timed out after {} seconds",
            GUEST_PROTOCOL_TIMEOUT.as_secs()
        )));
    }
    if export.exceeded {
        return Err(Error::InvalidInput(format!(
            "managed guest candidate archive exceeds its {byte_limit}-byte allowance"
        )));
    }
    if status.success() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "could not export the managed guest candidate: {}",
            guest_diagnostic(&stderr, &[])
        )))
    }
}

struct BoundedFileCopy {
    exceeded: bool,
}

fn copy_stream_to_bounded_file(
    mut reader: impl Read,
    mut output: File,
    byte_limit: u64,
) -> Result<BoundedFileCopy> {
    let mut written = 0_u64;
    let mut exceeded = false;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = byte_limit.saturating_sub(written);
        let retained = usize::try_from(remaining.min(read as u64)).unwrap_or(read);
        output.write_all(&buffer[..retained])?;
        written = written.saturating_add(retained as u64);
        exceeded |= retained < read;
    }
    output.sync_all()?;
    Ok(BoundedFileCopy { exceeded })
}

fn cleanup_guest_namespace(
    toolchain: &super::workspace_runtime_toolchain::ColimaToolchain,
    instance: &str,
    guest_namespace: &str,
) -> Result<()> {
    if !guest_namespace.starts_with(&format!("{GUEST_EXECUTION_ROOT}/"))
        || guest_namespace.contains("..")
    {
        return Err(Error::InvalidPath {
            path: guest_namespace.to_string(),
            reason: "refusing to clean an invalid guest execution namespace".to_string(),
        });
    }
    let result = run_guest_status(
        toolchain,
        instance,
        &[
            "rm".to_string(),
            "-rf".to_string(),
            "--".to_string(),
            guest_namespace.to_string(),
        ],
        Some(Stdio::null()),
        None,
    )?;
    if result.status.success() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "guest namespace cleanup failed: {}",
            if result.timed_out {
                "timed out".to_string()
            } else {
                guest_diagnostic(&result.stderr, &[])
            }
        )))
    }
}

fn validate_projection_path_collision(folded: &mut BTreeSet<String>, path: &str) -> Result<()> {
    let key = path.to_ascii_lowercase();
    if folded.insert(key) {
        Ok(())
    } else {
        Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "guest projection contains a duplicate or case-colliding path".to_string(),
        })
    }
}

fn validate_relative_symlink(path: &str, target: &Path) -> Result<()> {
    if target.is_absolute() {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "guest projections reject absolute symlink targets".to_string(),
        });
    }
    let Some(target) = target.to_str() else {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "guest symlink targets must be Unicode".to_string(),
        });
    };
    let parent = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
    let resolved = parent.join(target);
    let resolved = resolved.to_str().ok_or_else(|| Error::InvalidPath {
        path: path.to_string(),
        reason: "guest symlink target cannot be represented safely".to_string(),
    })?;
    normalize_relative_path(resolved).map(|_| ())
}

#[cfg(unix)]
fn portable_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn portable_mode(_metadata: &fs::Metadata) -> u32 {
    0o644
}

#[cfg(unix)]
fn set_portable_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_portable_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_relative_symlink(target: &Path, destination: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, destination)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_relative_symlink(_target: &Path, destination: &Path) -> Result<()> {
    Err(Error::InvalidPath {
        path: destination.to_string_lossy().into_owned(),
        reason: "managed Colima symlink import is unsupported on this host".to_string(),
    })
}

fn guest_diagnostic(stderr: &[u8], stdout: &[u8]) -> String {
    let bytes = if stderr.is_empty() { stdout } else { stderr };
    let limited = &bytes[..bytes.len().min(MAX_GUEST_DIAGNOSTIC_BYTES)];
    let mut message = String::from_utf8_lossy(limited).trim().to_string();
    if bytes.len() > limited.len() {
        message.push_str(" [truncated]");
    }
    if message.is_empty() {
        "guest command failed without diagnostic output".to_string()
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guest_test_manifest(namespace: String) -> GuestExecutionManifest {
        GuestExecutionManifest {
            schema: GUEST_MANIFEST_SCHEMA,
            execution_id: "exec_test".to_string(),
            lane_id: "lane_test".to_string(),
            profile: "trail-test".to_string(),
            lima_instance: "colima-trail-test".to_string(),
            guest_namespace: namespace,
            staging_path: "/private/staging".to_string(),
            owner_pid: std::process::id(),
            owner_start_token: current_process_start_token(),
            phase: "executing".to_string(),
            input_digest: "input".to_string(),
            candidate_digest: None,
            imported_paths: Vec::new(),
            removed_paths: Vec::new(),
            checkpoint_root: None,
            checkpoint_operation: None,
            error: None,
            cancellation_requested_at: None,
            cancellation_completed_at: None,
            process_group_terminated: false,
            updated_at: 1,
        }
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, script).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    fn write_fake_setsid(root: &Path) {
        write_executable(
            &root.join("setsid"),
            "#!/usr/bin/env python3\nimport os, sys\nos.setsid()\nos.execvp(sys.argv[1], sys.argv[1:])\n",
        );
    }

    #[cfg(unix)]
    fn fake_limactl_script() -> &'static str {
        "#!/bin/sh\n[ \"$1\" = shell ] || exit 91\nshift\nif [ \"$1\" = --workdir ]; then workdir=$2; shift 2; fi\nshift\n[ \"$1\" = -- ] || exit 92\nshift\n[ -z \"$workdir\" ] || cd \"$workdir\" || exit 93\nPATH=\"$(dirname \"$0\"):$PATH\"\nCOPYFILE_DISABLE=1\nexport PATH COPYFILE_DISABLE\nexec \"$@\"\n"
    }

    #[test]
    fn guest_diagnostic_is_bounded() {
        let diagnostic = guest_diagnostic(&vec![b'x'; MAX_GUEST_DIAGNOSTIC_BYTES + 20], &[]);
        assert!(diagnostic.ends_with(" [truncated]"));
        assert!(diagnostic.len() <= MAX_GUEST_DIAGNOSTIC_BYTES + " [truncated]".len());
    }

    #[test]
    fn guest_manifest_errors_are_redacted_before_durable_storage() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("manifest.json");
        let mut manifest = guest_test_manifest("/tmp/trail-executions/test/exec_test".to_string());
        write_guest_manifest(&path, &manifest).unwrap();
        update_guest_manifest(
            &path,
            &mut manifest,
            "cleanup_failed",
            Some("request failed with token=supersecret"),
        )
        .unwrap();
        let durable = fs::read_to_string(path).unwrap();
        assert!(!durable.contains("supersecret"));
        assert!(durable.contains("[REDACTED]"));
    }

    #[test]
    fn projection_excludes_private_paths_and_tracks_only_source_for_import() {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        fs::write(root.path().join("README.md"), "source").unwrap();
        fs::write(root.path().join(".env"), "SECRET=value").unwrap();
        fs::write(root.path().join(".trailignore"), "ignored.txt\n").unwrap();
        fs::write(root.path().join("ignored.txt"), "ignored").unwrap();
        fs::create_dir(root.path().join("target")).unwrap();
        fs::write(root.path().join("target/output.bin"), "generated").unwrap();
        let projection = build_projection(
            root.path(),
            staging.path(),
            ProjectionLimits {
                entries: 20,
                total_bytes: 1024,
                file_bytes: 1024,
            },
        )
        .unwrap();
        let mut archive = tar::Archive::new(File::open(projection.archive_path).unwrap());
        let paths = archive
            .entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(paths, [".trailignore", "README.md", "target/output.bin"]);
        assert_eq!(
            projection
                .source_snapshot
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            [".trailignore", "README.md"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn projection_rejects_escaping_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("../outside", root.path().join("escape")).unwrap();
        let error = build_projection(
            root.path(),
            staging.path(),
            ProjectionLimits {
                entries: 20,
                total_bytes: 1024,
                file_bytes: 1024,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("stay inside the workspace"));
    }

    #[test]
    fn candidate_import_applies_only_validated_source_delta() {
        let mount = tempfile::tempdir().unwrap();
        let candidate = tempfile::tempdir().unwrap();
        fs::write(mount.path().join("README.md"), "before").unwrap();
        fs::write(mount.path().join("removed.txt"), "remove").unwrap();
        fs::write(candidate.path().join("README.md"), "after").unwrap();
        fs::write(candidate.path().join("added.txt"), "add").unwrap();
        fs::create_dir(candidate.path().join("target")).unwrap();
        fs::write(candidate.path().join("target/generated"), "ignore").unwrap();
        let limits = ProjectionLimits {
            entries: 20,
            total_bytes: 1024,
            file_bytes: 1024,
        };
        let input = source_snapshot(mount.path(), limits).unwrap();
        let output = source_snapshot(candidate.path(), limits).unwrap();
        let (imported, removed) =
            apply_candidate_source(mount.path(), candidate.path(), &input, &output).unwrap();
        assert_eq!(imported, ["README.md", "added.txt"]);
        assert_eq!(removed, ["removed.txt"]);
        assert_eq!(
            fs::read_to_string(mount.path().join("README.md")).unwrap(),
            "after"
        );
        assert_eq!(
            fs::read_to_string(mount.path().join("added.txt")).unwrap(),
            "add"
        );
        assert!(!mount.path().join("removed.txt").exists());
        assert!(!mount.path().join("target/generated").exists());
    }

    #[test]
    fn candidate_archive_rejects_absolute_symlink_target() {
        let root = tempfile::tempdir().unwrap();
        let archive_path = root.path().join("candidate.tar");
        let mut builder = tar::Builder::new(File::create(&archive_path).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_path("link").unwrap();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("/etc/passwd").unwrap();
        header.set_cksum();
        builder.append(&header, io::empty()).unwrap();
        builder.finish().unwrap();
        drop(builder);
        let output = root.path().join("output");
        fs::create_dir(&output).unwrap();
        let ignore = lane_workdir_ignore_matcher(&output).unwrap();
        let error = validate_and_extract_candidate(
            &archive_path,
            &output,
            ProjectionLimits {
                entries: 20,
                total_bytes: 1024,
                file_bytes: 1024,
            },
            &ignore,
        )
        .unwrap_err();
        assert!(error.to_string().contains("absolute symlink"));
    }

    #[test]
    fn candidate_archive_rejects_entry_limit_and_case_collisions() {
        let root = tempfile::tempdir().unwrap();
        let archive_path = root.path().join("candidate.tar");
        let mut builder = tar::Builder::new(File::create(&archive_path).unwrap());
        for path in ["Readme.md", "README.md"] {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(1);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &b"x"[..]).unwrap();
        }
        builder.finish().unwrap();
        drop(builder);
        let output = root.path().join("output");
        fs::create_dir(&output).unwrap();
        let ignore = lane_workdir_ignore_matcher(&output).unwrap();
        let error = validate_and_extract_candidate(
            &archive_path,
            &output,
            ProjectionLimits {
                entries: 20,
                total_bytes: 1024,
                file_bytes: 1024,
            },
            &ignore,
        )
        .unwrap_err();
        assert!(error.to_string().contains("case-colliding"));

        fs::remove_dir_all(&output).unwrap();
        fs::create_dir(&output).unwrap();
        let ignore = lane_workdir_ignore_matcher(&output).unwrap();
        let error = validate_and_extract_candidate(
            &archive_path,
            &output,
            ProjectionLimits {
                entries: 0,
                total_bytes: 1024,
                file_bytes: 1024,
            },
            &ignore,
        )
        .unwrap_err();
        assert!(error.to_string().contains("entry limit"));
    }

    #[test]
    fn output_capture_drains_but_retains_only_the_bound() {
        let captured = read_bounded_stream(&b"0123456789"[..], 4).unwrap();
        assert!(captured.starts_with(b"0123"));
        assert!(captured.ends_with(b"[Trail guest output truncated]\n"));
        assert!(!captured.windows(4).any(|window| window == b"4567"));
    }

    #[test]
    fn archive_capture_drains_but_never_writes_past_the_bound() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("candidate.tar");
        let result =
            copy_stream_to_bounded_file(&b"0123456789"[..], File::create(&path).unwrap(), 4)
                .unwrap();
        assert!(result.exceeded);
        assert_eq!(fs::read(path).unwrap(), b"0123");
    }

    #[test]
    fn service_receipt_records_only_sorted_service_identities() {
        let bindings = guest_service_binding_identities(&[(
            "TRAIL_SERVICES_JSON".to_string(),
            r#"{"z-service":{"port":5432},"a-service":{"port":1234}}"#.to_string(),
        )])
        .unwrap();
        assert_eq!(bindings, ["a-service", "z-service"]);
    }

    #[cfg(unix)]
    #[test]
    fn fake_limactl_protocol_preserves_direct_command_arguments() {
        let root = tempfile::tempdir().unwrap();
        let limactl = root.path().join("limactl");
        write_executable(&limactl, fake_limactl_script());
        write_fake_setsid(root.path());
        let toolchain =
            super::super::workspace_runtime_toolchain::ColimaToolchain::for_guest_protocol_test(
                limactl,
                root.path(),
            );
        let manifest = guest_test_manifest(root.path().to_string_lossy().into_owned());
        let run = run_guest_command(
            &toolchain,
            "colima-test",
            &[
                "/bin/echo".to_string(),
                "literal;$(touch should-not-exist)".to_string(),
            ],
            root.path().to_str().unwrap(),
            None,
            &root.path().join("exec_test.cancel"),
            &manifest,
        )
        .unwrap();
        assert!(run.success);
        assert_eq!(
            String::from_utf8(run.stdout).unwrap(),
            "literal;$(touch should-not-exist)\n"
        );
        assert!(!root.path().join("should-not-exist").exists());
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_terminates_only_owned_guest_process_group() {
        let root = tempfile::tempdir().unwrap();
        let limactl = root.path().join("limactl");
        write_executable(&limactl, fake_limactl_script());
        write_fake_setsid(root.path());
        let toolchain =
            super::super::workspace_runtime_toolchain::ColimaToolchain::for_guest_protocol_test(
                limactl,
                root.path(),
            );
        let manifest = guest_test_manifest(root.path().to_string_lossy().into_owned());
        let cancellation_path = root.path().join("exec_test.cancel");
        let started_path = root.path().join("started");
        let completed_path = root.path().join("completed");
        let request_path = cancellation_path.clone();
        let request_manifest = manifest.clone();
        let started_for_request = started_path.clone();
        let requester = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !started_for_request.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(started_for_request.exists());
            let request = GuestCancellationRequest {
                schema: GUEST_MANIFEST_SCHEMA,
                execution_id: request_manifest.execution_id,
                lane_id: request_manifest.lane_id,
                requested_at: now_ts(),
            };
            write_file_atomic(
                &request_path,
                &serde_json::to_vec_pretty(&request).unwrap(),
                true,
            )
            .unwrap();
        });
        let mut unrelated = Command::new("sleep").arg("30").spawn().unwrap();
        let run = run_guest_command(
            &toolchain,
            "colima-test",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "touch \"$1\"; sleep 30; touch \"$2\"".to_string(),
                "trail-cancel-test".to_string(),
                started_path.to_string_lossy().into_owned(),
                completed_path.to_string_lossy().into_owned(),
            ],
            root.path().to_str().unwrap(),
            None,
            &cancellation_path,
            &manifest,
        )
        .unwrap();
        requester.join().unwrap();
        assert!(run.cancelled);
        assert!(run.process_group_terminated);
        assert!(!run.success);
        assert!(!completed_path.exists());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }

    #[test]
    fn recovery_discards_only_preimport_terminally_safe_phases() {
        let mut manifest = GuestExecutionManifest {
            schema: GUEST_MANIFEST_SCHEMA,
            execution_id: "exec_0123".to_string(),
            lane_id: "lane".to_string(),
            profile: "trail-test".to_string(),
            lima_instance: "colima-trail-test".to_string(),
            guest_namespace: "/tmp/trail-executions/workspace/exec_0123".to_string(),
            staging_path: "/private/staging".to_string(),
            owner_pid: 1,
            owner_start_token: "owner".to_string(),
            phase: "projected".to_string(),
            input_digest: "input".to_string(),
            candidate_digest: None,
            imported_paths: Vec::new(),
            removed_paths: Vec::new(),
            checkpoint_root: None,
            checkpoint_operation: None,
            error: None,
            cancellation_requested_at: None,
            cancellation_completed_at: None,
            process_group_terminated: false,
            updated_at: 1,
        };
        assert!(guest_manifest_is_safely_discardable(&manifest));
        manifest.phase = "executing".to_string();
        assert!(!guest_manifest_is_safely_discardable(&manifest));
        manifest.phase = "exported".to_string();
        assert!(!guest_manifest_is_safely_discardable(&manifest));
        manifest.phase = "cleaned".to_string();
        assert!(guest_manifest_is_safely_discardable(&manifest));
        manifest.imported_paths.push("README.md".to_string());
        assert!(!guest_manifest_is_safely_discardable(&manifest));
    }

    #[test]
    fn doctor_reports_ambiguous_guest_execution_without_mutating_it() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("README.md"), "root\n").unwrap();
        Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(root.path()).unwrap();
        let execution_id = "exec_0123";
        let workspace_hash = &sha256_hex(db.config.workspace.id.0.as_bytes())[..16];
        let manifest = GuestExecutionManifest {
            schema: GUEST_MANIFEST_SCHEMA,
            execution_id: execution_id.to_string(),
            lane_id: "lane".to_string(),
            profile: "trail-test".to_string(),
            lima_instance: "colima-trail-test".to_string(),
            guest_namespace: format!("{GUEST_EXECUTION_ROOT}/{workspace_hash}/{execution_id}"),
            staging_path: "/private/staging".to_string(),
            owner_pid: u32::MAX,
            owner_start_token: "not-live".to_string(),
            phase: "exported".to_string(),
            input_digest: "input".to_string(),
            candidate_digest: Some("candidate".to_string()),
            imported_paths: Vec::new(),
            removed_paths: Vec::new(),
            checkpoint_root: None,
            checkpoint_operation: None,
            error: None,
            cancellation_requested_at: None,
            cancellation_completed_at: None,
            process_group_terminated: false,
            updated_at: 1,
        };
        let path = guest_manifest_path(&db, execution_id).unwrap();
        write_guest_manifest(&path, &manifest).unwrap();

        let check = db.managed_guest_recovery_doctor_check();
        assert_eq!(check.name, "managed_guest_executions");
        assert_eq!(check.status, "error");
        assert_eq!(check.details.unwrap()["ambiguous"], 1);
        assert!(path.exists(), "doctor must remain read-only");
    }

    #[test]
    fn public_cancellation_report_reopens_terminal_receipt_by_execution_id() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("README.md"), "root\n").unwrap();
        Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(root.path()).unwrap();
        let lane = db
            .spawn_lane("cancel-lane", Some("main"), false, None, None)
            .unwrap();
        let execution_id = "exec_cancelled";
        let workspace_hash = &sha256_hex(db.config.workspace.id.0.as_bytes())[..16];
        let mut manifest = guest_test_manifest(format!(
            "{GUEST_EXECUTION_ROOT}/{workspace_hash}/{execution_id}"
        ));
        manifest.execution_id = execution_id.to_string();
        manifest.lane_id = lane.lane_id.clone();
        manifest.phase = "terminal_cancelled".to_string();
        manifest.cancellation_requested_at = Some(1);
        manifest.cancellation_completed_at = Some(2);
        manifest.process_group_terminated = true;
        write_guest_manifest(&guest_manifest_path(&db, execution_id).unwrap(), &manifest).unwrap();

        let report = db
            .cancel_lane_workspace_execution("cancel-lane", Some(execution_id))
            .unwrap();
        assert_eq!(report.execution_id, execution_id);
        assert_eq!(report.status, "cancelled");
        assert!(report.process_group_terminated);
        assert_eq!(report.cleanup_status, "succeeded");
    }

    #[cfg(unix)]
    #[test]
    fn fake_guest_full_protocol_imports_source_and_cleans_namespace() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("README.md"), "root\n").unwrap();
        Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(root.path()).unwrap();
        let lane = db
            .spawn_lane("guest-protocol", Some("main"), true, None, None)
            .unwrap();
        let workdir = PathBuf::from(lane.workdir.unwrap());
        let view = LaneWorkspaceViewReport {
            view_id: "view_fake_guest".to_string(),
            lane_id: lane.lane_id,
            base_change: lane.base_change,
            base_root: db.get_ref("refs/lanes/guest-protocol").unwrap().root_id,
            backend: "fake".to_string(),
            mountpoint: workdir.to_string_lossy().into_owned(),
            source_upper: workdir.to_string_lossy().into_owned(),
            generated_upper: workdir.join("target").to_string_lossy().into_owned(),
            scratch_upper: workdir.join(".scratch").to_string_lossy().into_owned(),
            meta_dir: workdir.join(".meta").to_string_lossy().into_owned(),
            journal_path: workdir.join(".journal").to_string_lossy().into_owned(),
            generation: 1,
            checkpoint_seq: 0,
            checkpoint_root: None,
            status: "ready".to_string(),
            owner_pid: None,
            owner_start_token: None,
            heartbeat_at: None,
            created_at: 1,
            updated_at: 1,
        };
        db.config.runtime.provider = "colima".to_string();
        db.config.runtime.execution_backend = "colima".to_string();
        db.config.runtime.colima_profile = Some("trail-test".to_string());
        let mut context = db
            .prepare_managed_lane_execution("guest-protocol", "lane_exec", &["/bin/sh".to_string()])
            .unwrap();
        let fake = tempfile::tempdir().unwrap();
        let limactl = fake.path().join("limactl");
        write_executable(&limactl, fake_limactl_script());
        write_fake_setsid(fake.path());
        let toolchain =
            super::super::workspace_runtime_toolchain::ColimaToolchain::for_guest_protocol_test(
                limactl,
                fake.path(),
            );
        let mut marked = false;
        let run = db
            .run_colima_lane_command_inner(
                &mut context,
                &view,
                &[
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    "printf guest > guest-source.txt; mkdir -p target; printf generated > target/build.bin"
                        .to_string(),
                ],
                None,
                &mut marked,
                Some(toolchain),
            )
            .unwrap();
        assert!(run.success);
        assert!(marked);
        assert_eq!(
            fs::read_to_string(workdir.join("guest-source.txt")).unwrap(),
            "guest"
        );
        assert!(!workdir.join("target/build.bin").exists());
        let manifest_path = context.guest_manifest_path.clone().unwrap();
        let manifest = read_guest_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.phase, "cleaned");
        assert!(manifest.guest_namespace.starts_with(GUEST_EXECUTION_ROOT));
        assert!(!Path::new(&manifest.guest_namespace).exists());
        let lifecycle = db.finalize_managed_lane_execution(
            context,
            Some("Fake guest protocol checkpoint".to_string()),
        );
        assert!(lifecycle
            .checkpoint
            .as_ref()
            .unwrap()
            .source_paths
            .iter()
            .any(|path| path == "guest-source.txt"));
        assert_eq!(
            read_guest_manifest(&manifest_path).unwrap().phase,
            "terminal_succeeded"
        );
    }

    #[cfg(unix)]
    #[test]
    fn recovery_discards_safe_owned_namespace_and_preserves_unrelated_profile() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("README.md"), "root\n").unwrap();
        Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(root.path()).unwrap();
        let limactl = root.path().join("limactl");
        write_executable(
            &limactl,
            "#!/bin/sh\ncase \" $* \" in *' cat -- '*) exit 1 ;; esac\nexit 0\n",
        );
        let toolchain =
            super::super::workspace_runtime_toolchain::ColimaToolchain::for_guest_protocol_test(
                limactl,
                root.path(),
            );
        toolchain.prepare_state().unwrap();
        let workspace_hash = &sha256_hex(db.config.workspace.id.0.as_bytes())[..16];
        for (execution_id, profile, phase) in [
            ("exec_safe", "trail-test", "projected"),
            ("exec_cancel", "trail-test", "executing"),
            ("exec_other", "other-profile", "exported"),
        ] {
            let manifest = GuestExecutionManifest {
                schema: GUEST_MANIFEST_SCHEMA,
                execution_id: execution_id.to_string(),
                lane_id: "lane".to_string(),
                profile: profile.to_string(),
                lima_instance: format!("colima-{profile}"),
                guest_namespace: format!("{GUEST_EXECUTION_ROOT}/{workspace_hash}/{execution_id}"),
                staging_path: "/private/staging".to_string(),
                owner_pid: u32::MAX,
                owner_start_token: "not-live".to_string(),
                phase: phase.to_string(),
                input_digest: "input".to_string(),
                candidate_digest: None,
                imported_paths: Vec::new(),
                removed_paths: Vec::new(),
                checkpoint_root: None,
                checkpoint_operation: None,
                error: None,
                cancellation_requested_at: None,
                cancellation_completed_at: None,
                process_group_terminated: false,
                updated_at: 1,
            };
            write_guest_manifest(&guest_manifest_path(&db, execution_id).unwrap(), &manifest)
                .unwrap();
            if execution_id == "exec_cancel" {
                let request = GuestCancellationRequest {
                    schema: GUEST_MANIFEST_SCHEMA,
                    execution_id: execution_id.to_string(),
                    lane_id: "lane".to_string(),
                    requested_at: 2,
                };
                write_file_atomic(
                    &guest_cancellation_path(&db, execution_id).unwrap(),
                    &serde_json::to_vec_pretty(&request).unwrap(),
                    true,
                )
                .unwrap();
            }
        }

        recover_guest_execution_manifests(&db, &toolchain, "trail-test", "colima-trail-test")
            .unwrap();
        assert_eq!(
            read_guest_manifest(&guest_manifest_path(&db, "exec_safe").unwrap())
                .unwrap()
                .phase,
            "terminal_recovered_discarded"
        );
        assert_eq!(
            read_guest_manifest(&guest_manifest_path(&db, "exec_cancel").unwrap())
                .unwrap()
                .phase,
            "terminal_cancelled"
        );
        assert!(!guest_cancellation_path(&db, "exec_cancel")
            .unwrap()
            .exists());
        assert_eq!(
            read_guest_manifest(&guest_manifest_path(&db, "exec_other").unwrap())
                .unwrap()
                .phase,
            "exported"
        );
    }
}
