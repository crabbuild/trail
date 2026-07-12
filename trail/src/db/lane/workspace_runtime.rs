use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::thread;

use rusqlite::params;
use serde_json::Value;

use super::*;

const TRAIL_MANAGED_LABEL: &str = "io.trail.managed";
const TRAIL_WORKSPACE_LABEL: &str = "io.trail.workspace";
const TRAIL_ALLOCATION_LABEL: &str = "io.trail.allocation";
const TRAIL_CLEANUP_LABEL: &str = "io.trail.cleanup-token";
const TRAIL_SECRET_BINDINGS_LABEL: &str = "io.trail.secret-bindings";
const MAX_PROVIDER_DIAGNOSTIC_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug)]
struct RuntimeAllocation {
    generation_id: String,
    component_id: String,
    resource_name: String,
    image_reference: String,
    image_digest: String,
    container_port: u16,
    health_timeout_ms: u64,
    restart_policy: String,
    volume_target: Option<String>,
    allocation_id: String,
    provider_resource_id: Option<String>,
    container_name: String,
    network_name: String,
    volume_name: Option<String>,
    host_port: Option<u16>,
    cleanup_token: String,
    owner_pid: Option<u32>,
    owner_start_token: Option<String>,
    secrets: Vec<RuntimeSecretReference>,
}

#[derive(Clone, Debug)]
struct RuntimeSecretReference {
    name: String,
    provider: String,
    reference: String,
    target: String,
    required: bool,
    purpose: String,
    environment: Option<String>,
}

#[derive(Clone, Debug)]
struct ResolvedRuntimeSecret {
    source_path: PathBuf,
    target: String,
    environment: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeContainerObservation {
    provider_resource_id: String,
    running: bool,
    host_port: Option<u16>,
    labels: BTreeMap<String, String>,
}

trait RuntimeProvider {
    fn provider_name(&self) -> &str;
    fn ensure_image(&self, reference: &str, expected_digest: &str) -> Result<()>;
    fn ensure_network(&self, allocation: &RuntimeAllocation) -> Result<()>;
    fn ensure_volume(&self, allocation: &RuntimeAllocation) -> Result<()>;
    fn inspect_container(
        &self,
        allocation: &RuntimeAllocation,
    ) -> Result<Option<RuntimeContainerObservation>>;
    fn create_container(
        &self,
        allocation: &RuntimeAllocation,
        secrets: &[ResolvedRuntimeSecret],
    ) -> Result<String>;
    fn start_container(&self, allocation: &RuntimeAllocation) -> Result<()>;
    fn stop_container(&self, allocation: &RuntimeAllocation) -> Result<()>;
    fn remove_container(&self, allocation: &RuntimeAllocation) -> Result<()>;
    fn remove_network(&self, allocation: &RuntimeAllocation) -> Result<()>;
    fn remove_volume(&self, allocation: &RuntimeAllocation) -> Result<()>;
}

struct CliRuntimeProvider {
    name: String,
    executable: PathBuf,
    workspace_id: String,
}

impl CliRuntimeProvider {
    fn detect(workspace_id: &str) -> Result<Self> {
        let mut failures = Vec::new();
        for name in ["docker", "podman"] {
            let tool = match super::workspace_environment::resolve_workspace_tool_executable(name) {
                Ok(tool) => tool,
                Err(err) => {
                    failures.push(err.to_string());
                    continue;
                }
            };
            let output = Command::new(&tool.path)
                .args(["info", "--format", "{{json .ServerVersion}}"])
                .output();
            match output {
                Ok(output) if output.status.success() => {
                    return Ok(Self {
                        name: name.to_string(),
                        executable: tool.path,
                        workspace_id: workspace_id.to_string(),
                    });
                }
                Ok(output) => {
                    failures.push(format!("{name}: {}", provider_output_diagnostic(&output)))
                }
                Err(err) => failures.push(format!("{name}: {err}")),
            }
        }
        Err(Error::InvalidInput(format!(
            "no usable OCI runtime provider was found; install and start Docker or Podman ({})",
            failures.join("; ")
        )))
    }

    fn run(&self, operation: &str, args: &[String]) -> Result<Output> {
        let output = Command::new(&self.executable).args(args).output()?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(Error::InvalidInput(format!(
                "{} runtime {operation} failed: {}",
                self.name,
                provider_output_diagnostic(&output)
            )))
        }
    }

    fn inspect_resource_labels(
        &self,
        kind: &str,
        name: &str,
    ) -> Result<Option<BTreeMap<String, String>>> {
        let output = Command::new(&self.executable)
            .args([kind, "inspect", name, "--format", "{{json .Labels}}"])
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        let labels =
            serde_json::from_slice::<BTreeMap<String, String>>(&output.stdout).map_err(|err| {
                Error::Corrupt(format!(
                    "{} returned invalid {kind} label JSON for `{name}`: {err}",
                    self.name
                ))
            })?;
        Ok(Some(labels))
    }

    fn ownership_labels(&self, allocation: &RuntimeAllocation) -> Vec<String> {
        self.label_args([
            format!("{TRAIL_MANAGED_LABEL}=true"),
            format!("{TRAIL_WORKSPACE_LABEL}={}", self.workspace_id),
            format!("{TRAIL_ALLOCATION_LABEL}={}", allocation.allocation_id),
            format!("{TRAIL_CLEANUP_LABEL}={}", allocation.cleanup_token),
        ])
    }

    fn network_labels(&self, allocation: &RuntimeAllocation) -> Vec<String> {
        self.label_args([
            format!("{TRAIL_MANAGED_LABEL}=true"),
            format!("{TRAIL_WORKSPACE_LABEL}={}", self.workspace_id),
            format!("io.trail.network={}", allocation.network_name),
        ])
    }

    fn volume_labels(&self, volume_name: &str) -> Vec<String> {
        self.label_args([
            format!("{TRAIL_MANAGED_LABEL}=true"),
            format!("{TRAIL_WORKSPACE_LABEL}={}", self.workspace_id),
            format!("io.trail.volume={volume_name}"),
        ])
    }

    fn label_args<const N: usize>(&self, labels: [String; N]) -> Vec<String> {
        labels
            .into_iter()
            .flat_map(|label| ["--label".to_string(), label])
            .collect()
    }

    fn validate_workspace_labels(
        &self,
        kind: &str,
        name: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<()> {
        if labels.get(TRAIL_MANAGED_LABEL).map(String::as_str) != Some("true")
            || labels.get(TRAIL_WORKSPACE_LABEL) != Some(&self.workspace_id)
        {
            return Err(Error::InvalidInput(format!(
                "{kind} name `{name}` is occupied by a resource this Trail workspace does not own; refusing to adopt or replace it"
            )));
        }
        Ok(())
    }
}

impl RuntimeProvider for CliRuntimeProvider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    fn ensure_image(&self, reference: &str, expected_digest: &str) -> Result<()> {
        let inspect = Command::new(&self.executable)
            .args([
                "image",
                "inspect",
                reference,
                "--format",
                "{{json .RepoDigests}}",
            ])
            .output()?;
        let output = if inspect.status.success() {
            inspect
        } else {
            self.run("image pull", &["pull".to_string(), reference.to_string()])?;
            self.run(
                "image inspect",
                &[
                    "image".to_string(),
                    "inspect".to_string(),
                    reference.to_string(),
                    "--format".to_string(),
                    "{{json .RepoDigests}}".to_string(),
                ],
            )?
        };
        let observed = String::from_utf8_lossy(&output.stdout);
        if !observed.contains(expected_digest) {
            return Err(Error::InvalidInput(format!(
                "{} resolved image `{reference}` without expected digest `{expected_digest}`",
                self.name
            )));
        }
        Ok(())
    }

    fn ensure_network(&self, allocation: &RuntimeAllocation) -> Result<()> {
        if let Some(labels) = self.inspect_resource_labels("network", &allocation.network_name)? {
            self.validate_workspace_labels("network", &allocation.network_name, &labels)?;
            if labels.get("io.trail.network") != Some(&allocation.network_name) {
                return Err(Error::InvalidInput(format!(
                    "network `{}` has incompatible Trail ownership metadata",
                    allocation.network_name
                )));
            }
            return Ok(());
        }
        let mut args = vec!["network".to_string(), "create".to_string()];
        args.extend(self.network_labels(allocation));
        args.push(allocation.network_name.clone());
        self.run("network create", &args)?;
        Ok(())
    }

    fn ensure_volume(&self, allocation: &RuntimeAllocation) -> Result<()> {
        let Some(volume_name) = allocation.volume_name.as_deref() else {
            return Ok(());
        };
        if let Some(labels) = self.inspect_resource_labels("volume", volume_name)? {
            self.validate_workspace_labels("volume", volume_name, &labels)?;
            if labels.get("io.trail.volume").map(String::as_str) != Some(volume_name) {
                return Err(Error::InvalidInput(format!(
                    "volume `{volume_name}` has incompatible Trail ownership metadata"
                )));
            }
            return Ok(());
        }
        let mut args = vec!["volume".to_string(), "create".to_string()];
        args.extend(self.volume_labels(volume_name));
        args.push(volume_name.to_string());
        self.run("volume create", &args)?;
        Ok(())
    }

    fn inspect_container(
        &self,
        allocation: &RuntimeAllocation,
    ) -> Result<Option<RuntimeContainerObservation>> {
        let output = Command::new(&self.executable)
            .args([
                "container",
                "inspect",
                &allocation.container_name,
                "--format",
                "{{json .}}",
            ])
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        let value: Value = serde_json::from_slice(&output.stdout).map_err(|err| {
            Error::Corrupt(format!(
                "{} returned invalid container inspection JSON: {err}",
                self.name
            ))
        })?;
        let provider_resource_id = value
            .get("Id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| Error::Corrupt("container inspection omitted Id".to_string()))?
            .to_string();
        let running = value
            .pointer("/State/Running")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let labels = value
            .pointer("/Config/Labels")
            .and_then(Value::as_object)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .as_str()
                            .map(|value| (name.clone(), value.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let port_key = format!("{}/tcp", allocation.container_port);
        let host_port = value
            .pointer("/NetworkSettings/Ports")
            .and_then(Value::as_object)
            .and_then(|ports| ports.get(&port_key))
            .and_then(Value::as_array)
            .and_then(|bindings| bindings.first())
            .and_then(|binding| binding.get("HostPort"))
            .and_then(Value::as_str)
            .and_then(|port| port.parse::<u16>().ok());
        Ok(Some(RuntimeContainerObservation {
            provider_resource_id,
            running,
            host_port,
            labels,
        }))
    }

    fn create_container(
        &self,
        allocation: &RuntimeAllocation,
        secrets: &[ResolvedRuntimeSecret],
    ) -> Result<String> {
        let host_port = allocation.host_port.ok_or_else(|| {
            Error::Corrupt(format!(
                "runtime allocation `{}` has no reserved host port",
                allocation.allocation_id
            ))
        })?;
        let mut args = vec![
            "container".to_string(),
            "create".to_string(),
            "--name".to_string(),
            allocation.container_name.clone(),
            "--network".to_string(),
            allocation.network_name.clone(),
            "--publish".to_string(),
            format!("127.0.0.1:{host_port}:{}", allocation.container_port),
            "--restart".to_string(),
            match allocation.restart_policy.as_str() {
                "never" => "no".to_string(),
                "on_failure" => "on-failure".to_string(),
                "always" => "always".to_string(),
                other => {
                    return Err(Error::Corrupt(format!(
                        "unsupported persisted restart policy `{other}`"
                    )))
                }
            },
        ];
        args.extend(self.ownership_labels(allocation));
        if !allocation.secrets.is_empty() {
            args.extend(self.label_args([format!(
                "{TRAIL_SECRET_BINDINGS_LABEL}={}",
                runtime_secret_binding_digest(secrets)
            )]));
        }
        if let (Some(volume_name), Some(volume_target)) = (
            allocation.volume_name.as_deref(),
            allocation.volume_target.as_deref(),
        ) {
            args.extend([
                "--mount".to_string(),
                format!("type=volume,src={volume_name},dst={volume_target}"),
            ]);
        }
        for secret in secrets {
            args.extend([
                "--mount".to_string(),
                format!(
                    "type=bind,src={},dst={},readonly",
                    secret.source_path.to_string_lossy(),
                    secret.target
                ),
            ]);
            if let Some(environment) = secret.environment.as_deref() {
                args.extend([
                    "--env".to_string(),
                    format!("{environment}={}", secret.target),
                ]);
            }
        }
        args.push(allocation.image_reference.clone());
        let output = self.run("container create", &args)?;
        let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if id.is_empty() {
            return Err(Error::Corrupt(format!(
                "{} created container `{}` without returning an ID",
                self.name, allocation.container_name
            )));
        }
        Ok(id)
    }

    fn start_container(&self, allocation: &RuntimeAllocation) -> Result<()> {
        self.run(
            "container start",
            &["start".to_string(), allocation.container_name.clone()],
        )?;
        Ok(())
    }

    fn stop_container(&self, allocation: &RuntimeAllocation) -> Result<()> {
        if self.inspect_container(allocation)?.is_none() {
            return Ok(());
        }
        self.run(
            "container stop",
            &[
                "stop".to_string(),
                "--time".to_string(),
                "10".to_string(),
                allocation.container_name.clone(),
            ],
        )?;
        Ok(())
    }

    fn remove_container(&self, allocation: &RuntimeAllocation) -> Result<()> {
        let Some(observation) = self.inspect_container(allocation)? else {
            return Ok(());
        };
        validate_runtime_container_ownership(allocation, &observation)?;
        self.run(
            "container remove",
            &[
                "rm".to_string(),
                "--force".to_string(),
                allocation.container_name.clone(),
            ],
        )?;
        Ok(())
    }

    fn remove_network(&self, allocation: &RuntimeAllocation) -> Result<()> {
        let Some(labels) = self.inspect_resource_labels("network", &allocation.network_name)?
        else {
            return Ok(());
        };
        self.validate_workspace_labels("network", &allocation.network_name, &labels)?;
        if labels.get("io.trail.network") != Some(&allocation.network_name) {
            return Err(Error::InvalidInput(format!(
                "network `{}` has incompatible Trail ownership metadata",
                allocation.network_name
            )));
        }
        self.run(
            "network remove",
            &[
                "network".to_string(),
                "rm".to_string(),
                allocation.network_name.clone(),
            ],
        )?;
        Ok(())
    }

    fn remove_volume(&self, allocation: &RuntimeAllocation) -> Result<()> {
        let Some(volume_name) = allocation.volume_name.as_deref() else {
            return Ok(());
        };
        let Some(labels) = self.inspect_resource_labels("volume", volume_name)? else {
            return Ok(());
        };
        self.validate_workspace_labels("volume", volume_name, &labels)?;
        if labels.get("io.trail.volume").map(String::as_str) != Some(volume_name) {
            return Err(Error::InvalidInput(format!(
                "volume `{volume_name}` has incompatible Trail ownership metadata"
            )));
        }
        self.run(
            "volume remove",
            &[
                "volume".to_string(),
                "rm".to_string(),
                volume_name.to_string(),
            ],
        )?;
        Ok(())
    }
}

impl Trail {
    pub(crate) fn recover_workspace_runtime_leases(&self) -> Result<()> {
        let rows = self.runtime_allocations_with_live_statuses(&["allocating", "stopping"])?;
        for allocation in rows {
            let owner_is_live = allocation
                .owner_pid
                .zip(allocation.owner_start_token.as_deref())
                .is_some_and(|(pid, token)| process_matches_start_token(pid, token));
            if owner_is_live {
                continue;
            }
            self.conn.execute(
                "UPDATE environment_generation_runtime_resources
                 SET status = 'orphaned', health_status = 'unknown',
                     reason = ?1, owner_pid = NULL, owner_start_token = NULL, updated_at = ?2
                 WHERE allocation_id = ?3 AND status IN ('allocating', 'stopping')",
                params![
                    "runtime lifecycle owner exited; reconcile will inspect and adopt matching provider resources",
                    now_ts(),
                    allocation.allocation_id
                ],
            )?;
        }
        Ok(())
    }

    pub fn reconcile_workspace_environment_runtime(
        &self,
        lane: &str,
    ) -> Result<EnvironmentGenerationReport> {
        let generation = self.active_environment_generation(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` has no active environment generation"
            ))
        })?;
        let allocations = self.runtime_allocations_for_generation(&generation.generation_id)?;
        if allocations.is_empty() {
            return Ok(generation);
        }
        self.recover_workspace_runtime_leases()?;
        let provider = CliRuntimeProvider::detect(&self.config.workspace.id.0)?;
        self.reconcile_workspace_environment_runtime_with(&provider, &allocations)?;
        self.active_environment_generation(lane)?.ok_or_else(|| {
            Error::Corrupt("active generation disappeared after runtime reconcile".to_string())
        })
    }

    pub fn stop_workspace_environment_runtime(
        &self,
        lane: &str,
    ) -> Result<EnvironmentGenerationReport> {
        let generation = self.active_environment_generation(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` has no active environment generation"
            ))
        })?;
        let allocations = self.runtime_allocations_for_generation(&generation.generation_id)?;
        if allocations.is_empty() {
            return Ok(generation);
        }
        self.recover_workspace_runtime_leases()?;
        let provider = CliRuntimeProvider::detect(&self.config.workspace.id.0)?;
        for allocation in allocations.iter().rev() {
            self.claim_runtime_allocation(allocation, "stopping", "stopped")?;
            let stopped = (|| -> Result<()> {
                if let Some(observation) = provider.inspect_container(allocation)? {
                    validate_runtime_container_ownership(allocation, &observation)?;
                }
                provider.stop_container(allocation)
            })();
            match stopped {
                Ok(()) => self.finish_runtime_allocation(
                    allocation,
                    "stopped",
                    "stopped",
                    allocation.provider_resource_id.as_deref(),
                    allocation.host_port,
                    None,
                )?,
                Err(err) => {
                    self.finish_runtime_allocation(
                        allocation,
                        "failed",
                        "unknown",
                        allocation.provider_resource_id.as_deref(),
                        allocation.host_port,
                        Some(&err.to_string()),
                    )?;
                    return Err(err);
                }
            }
        }
        self.active_environment_generation(lane)?.ok_or_else(|| {
            Error::Corrupt("active generation disappeared after runtime stop".to_string())
        })
    }

    /// Remove Trail-owned runtime resources belonging only to retired
    /// generations. A logical lane-service volume is retained while any
    /// active generation references it.
    pub fn cleanup_retired_workspace_environment_runtime(
        &self,
        lane: &str,
    ) -> Result<EnvironmentGenerationReport> {
        let generation = self.active_environment_generation(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` has no active environment generation"
            ))
        })?;
        self.recover_workspace_runtime_leases()?;
        let allocations = self.runtime_allocations_where(
            "WHERE generation_id IN (
                 SELECT generation_id FROM environment_generations
                 WHERE view_id = ?1 AND state = 'retired'
             ) AND status != 'stopped'
             ORDER BY generation_id, component_id, resource_name",
            [generation.view_id.as_str()],
        )?;
        if allocations.is_empty() {
            return Ok(generation);
        }
        let provider = CliRuntimeProvider::detect(&self.config.workspace.id.0)?;
        self.cleanup_retired_workspace_environment_runtime_with(&provider, &allocations)?;
        Ok(generation)
    }

    fn cleanup_retired_workspace_environment_runtime_with(
        &self,
        provider: &dyn RuntimeProvider,
        allocations: &[RuntimeAllocation],
    ) -> Result<()> {
        for allocation in allocations {
            self.claim_runtime_allocation(allocation, "stopping", "stopped")?;
            let cleanup = (|| -> Result<()> {
                provider.remove_container(allocation)?;
                if let Some(volume_name) = allocation.volume_name.as_deref() {
                    let active_references = self.conn.query_row(
                        "SELECT COUNT(*)
                         FROM environment_generation_runtime_resources r
                         JOIN environment_generations g ON g.generation_id = r.generation_id
                         WHERE g.state = 'active' AND r.volume_name = ?1",
                        params![volume_name],
                        |row| row.get::<_, i64>(0),
                    )?;
                    if active_references == 0 {
                        provider.remove_volume(allocation)?;
                    }
                }
                Ok(())
            })();
            match cleanup {
                Ok(()) => self.finish_runtime_allocation(
                    allocation,
                    "stopped",
                    "stopped",
                    allocation.provider_resource_id.as_deref(),
                    allocation.host_port,
                    Some("retired generation runtime resources were cleaned"),
                )?,
                Err(err) => {
                    self.finish_runtime_allocation(
                        allocation,
                        "failed",
                        "unknown",
                        allocation.provider_resource_id.as_deref(),
                        allocation.host_port,
                        Some(&err.to_string()),
                    )?;
                    return Err(err);
                }
            }
        }
        let mut networks = BTreeMap::<String, &RuntimeAllocation>::new();
        for allocation in allocations {
            networks
                .entry(allocation.network_name.clone())
                .or_insert(allocation);
        }
        for allocation in networks.into_values() {
            let active_references = self.conn.query_row(
                "SELECT COUNT(*)
                 FROM environment_generation_runtime_resources r
                 JOIN environment_generations g ON g.generation_id = r.generation_id
                 WHERE g.state = 'active' AND r.network_name = ?1",
                params![&allocation.network_name],
                |row| row.get::<_, i64>(0),
            )?;
            if active_references == 0 {
                provider.remove_network(allocation)?;
            }
        }
        Ok(())
    }

    fn reconcile_workspace_environment_runtime_with(
        &self,
        provider: &dyn RuntimeProvider,
        allocations: &[RuntimeAllocation],
    ) -> Result<()> {
        let mut first_error = None;
        for allocation in allocations {
            if let Err(err) = self.claim_runtime_allocation(allocation, "allocating", "starting") {
                if first_error.is_none() {
                    first_error = Some(err);
                }
                continue;
            }
            if let Err(err) = self.reconcile_claimed_runtime_allocation(provider, allocation) {
                let mut reason = err.to_string();
                if let Ok(Some(observation)) = provider.inspect_container(allocation) {
                    if validate_runtime_container_ownership(allocation, &observation).is_ok() {
                        if let Err(stop_err) = provider.stop_container(allocation) {
                            reason = format!(
                                "{reason}; additionally failed to stop the unhealthy owned container: {stop_err}"
                            );
                        }
                    }
                }
                self.finish_runtime_allocation(
                    allocation,
                    "failed",
                    "unhealthy",
                    allocation.provider_resource_id.as_deref(),
                    allocation.host_port,
                    Some(&reason),
                )?;
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(())
        }
    }

    fn reconcile_claimed_runtime_allocation(
        &self,
        provider: &dyn RuntimeProvider,
        allocation: &RuntimeAllocation,
    ) -> Result<()> {
        let resolved_secrets = self.resolve_runtime_secrets(allocation)?;
        provider.ensure_image(&allocation.image_reference, &allocation.image_digest)?;
        provider.ensure_network(allocation)?;
        provider.ensure_volume(allocation)?;
        let mut allocation = allocation.clone();
        let mut observation = provider.inspect_container(&allocation)?;
        if let Some(existing) = observation.as_ref() {
            validate_runtime_container_ownership(&allocation, existing)?;
            if !runtime_secret_bindings_match(&allocation, &resolved_secrets, existing) {
                provider.remove_container(&allocation)?;
                observation = None;
            }
        }
        if observation.is_none() {
            if allocation.host_port.is_none() {
                allocation.host_port = Some(self.reserve_runtime_host_port(&allocation)?);
            }
            let provider_resource_id = provider.create_container(&allocation, &resolved_secrets)?;
            self.update_runtime_provider_identity(&allocation, &provider_resource_id)?;
        }
        if !observation.as_ref().is_some_and(|state| state.running) {
            provider.start_container(&allocation)?;
        }
        observation = provider.inspect_container(&allocation)?;
        let observation = observation.ok_or_else(|| {
            Error::InvalidInput(format!(
                "{} started `{}` but it cannot be inspected",
                provider.provider_name(),
                allocation.container_name
            ))
        })?;
        validate_runtime_container_ownership(&allocation, &observation)?;
        let host_port = observation.host_port.ok_or_else(|| {
            Error::InvalidInput(format!(
                "{} container `{}` has no host port for {}/tcp",
                provider.provider_name(),
                allocation.container_name,
                allocation.container_port
            ))
        })?;
        wait_for_tcp_health(host_port, allocation.health_timeout_ms)?;
        self.finish_runtime_allocation(
            &allocation,
            "running",
            "healthy",
            Some(&observation.provider_resource_id),
            Some(host_port),
            None,
        )
    }

    fn resolve_runtime_secrets(
        &self,
        allocation: &RuntimeAllocation,
    ) -> Result<Vec<ResolvedRuntimeSecret>> {
        let mut resolved = Vec::with_capacity(allocation.secrets.len());
        for secret in &allocation.secrets {
            let path = match secret.provider.as_str() {
                "file" => Some(PathBuf::from(&secret.reference)),
                "environment_file" => std::env::var_os(&secret.reference).map(PathBuf::from),
                _ => None,
            };
            let Some(path) = path else {
                self.update_runtime_secret_status(
                    allocation,
                    secret,
                    "unavailable",
                    Some("secret provider did not return a file handle"),
                    None,
                )?;
                if secret.required {
                    return Err(Error::InvalidInput(format!(
                        "required secret reference `{}` is unavailable from provider `{}`",
                        secret.name, secret.provider
                    )));
                }
                continue;
            };
            if let Err(err) = validate_runtime_secret_file(&path) {
                self.update_runtime_secret_status(
                    allocation,
                    secret,
                    "unavailable",
                    Some("secret provider returned an unsafe or inaccessible file handle"),
                    None,
                )?;
                if secret.required {
                    return Err(Error::InvalidInput(format!(
                        "required secret reference `{}` failed file-handle validation: {err}",
                        secret.name
                    )));
                }
                continue;
            }
            let canonical_path = fs::canonicalize(&path).map_err(|_| {
                Error::InvalidInput(format!(
                    "required secret reference `{}` could not be canonicalized",
                    secret.name
                ))
            })?;
            if canonical_path.starts_with(&self.workspace_root)
                || canonical_path.starts_with(&self.db_dir)
            {
                self.update_runtime_secret_status(
                    allocation,
                    secret,
                    "unavailable",
                    Some("secret provider file handle points inside the Trail workspace"),
                    None,
                )?;
                if secret.required {
                    return Err(Error::InvalidInput(format!(
                        "secret reference `{}` must resolve outside the Trail workspace so it cannot enter checkpoints or artifacts",
                        secret.name
                    )));
                }
                continue;
            }
            self.update_runtime_secret_status(
                allocation,
                secret,
                "available",
                None,
                Some(now_ts()),
            )?;
            resolved.push(ResolvedRuntimeSecret {
                source_path: canonical_path,
                target: secret.target.clone(),
                environment: secret.environment.clone(),
            });
        }
        Ok(resolved)
    }

    fn update_runtime_secret_status(
        &self,
        allocation: &RuntimeAllocation,
        secret: &RuntimeSecretReference,
        status: &str,
        reason: Option<&str>,
        resolved_at: Option<i64>,
    ) -> Result<()> {
        self.conn
            .execute_batch("SAVEPOINT trail_runtime_secret_access")?;
        let result = (|| -> Result<()> {
            let changed = self.conn.execute(
                "UPDATE environment_generation_runtime_secrets
                 SET status = ?1, reason = ?2, resolved_at = ?3, updated_at = ?4
                 WHERE generation_id = ?5 AND component_id = ?6
                   AND resource_name = ?7 AND secret_name = ?8",
                params![
                    status,
                    reason,
                    resolved_at,
                    now_ts(),
                    allocation.generation_id,
                    allocation.component_id,
                    allocation.resource_name,
                    secret.name
                ],
            )?;
            if changed != 1 {
                return Err(Error::Corrupt(format!(
                    "runtime secret reference `{}` disappeared during resolution",
                    secret.name
                )));
            }
            let access_id = format!(
                "secret_access_{}",
                crate::ids::short_hash(
                    format!(
                        "{}:{}:{}:{}:{}",
                        allocation.allocation_id,
                        secret.name,
                        status,
                        now_nanos(),
                        std::process::id()
                    )
                    .as_bytes(),
                    16
                )
            );
            self.conn.execute(
                "INSERT INTO environment_secret_access_audit
                 (access_id, generation_id, component_id, resource_name, secret_name,
                  provider, purpose, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    access_id,
                    allocation.generation_id,
                    allocation.component_id,
                    allocation.resource_name,
                    secret.name,
                    secret.provider,
                    secret.purpose,
                    status,
                    now_ts()
                ],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn
                    .execute_batch("RELEASE SAVEPOINT trail_runtime_secret_access")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_runtime_secret_access;
                     RELEASE SAVEPOINT trail_runtime_secret_access",
                );
                Err(err)
            }
        }
    }

    fn claim_runtime_allocation(
        &self,
        allocation: &RuntimeAllocation,
        status: &str,
        health_status: &str,
    ) -> Result<()> {
        let token = current_process_start_token();
        let changed = self.conn.execute(
            "UPDATE environment_generation_runtime_resources
             SET status = ?1, health_status = ?2, reason = NULL,
                 owner_pid = ?3, owner_start_token = ?4, updated_at = ?5
             WHERE allocation_id = ?6
               AND (owner_pid IS NULL OR (owner_pid = ?3 AND owner_start_token = ?4))",
            params![
                status,
                health_status,
                std::process::id(),
                token,
                now_ts(),
                allocation.allocation_id
            ],
        )?;
        if changed != 1 {
            return Err(Error::InvalidInput(format!(
                "runtime allocation `{}` is owned by another live process",
                allocation.allocation_id
            )));
        }
        Ok(())
    }

    fn update_runtime_provider_identity(
        &self,
        allocation: &RuntimeAllocation,
        provider_resource_id: &str,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE environment_generation_runtime_resources
             SET provider_resource_id = ?1, updated_at = ?2
             WHERE allocation_id = ?3 AND owner_pid = ?4 AND owner_start_token = ?5",
            params![
                provider_resource_id,
                now_ts(),
                allocation.allocation_id,
                std::process::id(),
                current_process_start_token()
            ],
        )?;
        if changed != 1 {
            return Err(Error::InvalidInput(format!(
                "runtime allocation `{}` ownership changed during create",
                allocation.allocation_id
            )));
        }
        Ok(())
    }

    fn reserve_runtime_host_port(&self, allocation: &RuntimeAllocation) -> Result<u16> {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let host_port = listener.local_addr()?.port();
        let changed = self.conn.execute(
            "UPDATE environment_generation_runtime_resources
             SET host_port = ?1, updated_at = ?2
             WHERE allocation_id = ?3 AND owner_pid = ?4 AND owner_start_token = ?5",
            params![
                host_port,
                now_ts(),
                allocation.allocation_id,
                std::process::id(),
                current_process_start_token()
            ],
        )?;
        if changed != 1 {
            return Err(Error::InvalidInput(format!(
                "runtime allocation `{}` ownership changed while reserving a loopback port",
                allocation.allocation_id
            )));
        }
        drop(listener);
        Ok(host_port)
    }

    fn finish_runtime_allocation(
        &self,
        allocation: &RuntimeAllocation,
        status: &str,
        health_status: &str,
        provider_resource_id: Option<&str>,
        host_port: Option<u16>,
        reason: Option<&str>,
    ) -> Result<()> {
        let reason = reason.map(sanitize_provider_text);
        let now = now_ts();
        let changed = self.conn.execute(
            "UPDATE environment_generation_runtime_resources
             SET status = ?1, health_status = ?2, reason = ?3,
                 provider_resource_id = COALESCE(?4, provider_resource_id), host_port = ?5,
                 owner_pid = NULL, owner_start_token = NULL, updated_at = ?6,
                 started_at = CASE WHEN ?1 = 'running' THEN COALESCE(started_at, ?6) ELSE started_at END,
                 stopped_at = CASE WHEN ?1 = 'stopped' THEN ?6 ELSE stopped_at END
             WHERE allocation_id = ?7 AND owner_pid = ?8 AND owner_start_token = ?9",
            params![
                status,
                health_status,
                reason,
                provider_resource_id,
                host_port,
                now,
                allocation.allocation_id,
                std::process::id(),
                current_process_start_token()
            ],
        )?;
        if changed != 1 {
            return Err(Error::InvalidInput(format!(
                "runtime allocation `{}` ownership changed before lifecycle state could be recorded",
                allocation.allocation_id
            )));
        }
        Ok(())
    }

    fn runtime_allocations_for_generation(
        &self,
        generation_id: &str,
    ) -> Result<Vec<RuntimeAllocation>> {
        self.runtime_allocations_where(
            "WHERE generation_id = ?1 ORDER BY component_id, resource_name",
            [generation_id],
        )
    }

    fn runtime_allocations_with_live_statuses(
        &self,
        statuses: &[&str],
    ) -> Result<Vec<RuntimeAllocation>> {
        let mut allocations = Vec::new();
        for status in statuses {
            allocations.extend(self.runtime_allocations_where(
                "WHERE status = ?1 ORDER BY generation_id, component_id, resource_name",
                [*status],
            )?);
        }
        Ok(allocations)
    }

    fn runtime_allocations_where<const N: usize>(
        &self,
        clause: &str,
        values: [&str; N],
    ) -> Result<Vec<RuntimeAllocation>> {
        let sql = format!(
            "SELECT generation_id, component_id, resource_name,
                    image_reference, image_digest, container_port, health_timeout_ms, restart_policy,
                    volume_target, allocation_id, provider_resource_id, container_name,
                    network_name, volume_name, host_port, cleanup_token,
                    owner_pid, owner_start_token
             FROM environment_generation_runtime_resources {clause}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(values), |row| {
            let owner_pid = row
                .get::<_, Option<i64>>(16)?
                .and_then(|pid| u32::try_from(pid).ok());
            Ok(RuntimeAllocation {
                generation_id: row.get(0)?,
                component_id: row.get(1)?,
                resource_name: row.get(2)?,
                image_reference: row.get(3)?,
                image_digest: row.get(4)?,
                container_port: row.get(5)?,
                health_timeout_ms: row.get(6)?,
                restart_policy: row.get(7)?,
                volume_target: row.get(8)?,
                allocation_id: row.get(9)?,
                provider_resource_id: row.get(10)?,
                container_name: row.get(11)?,
                network_name: row.get(12)?,
                volume_name: row.get(13)?,
                host_port: row.get(14)?,
                cleanup_token: row.get(15)?,
                owner_pid,
                owner_start_token: row.get(17)?,
                secrets: Vec::new(),
            })
        })?;
        let mut allocations = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        drop(stmt);
        for allocation in &mut allocations {
            let mut secret_stmt = self.conn.prepare(
                "SELECT secret_name, provider, reference, target, required, purpose, environment
                 FROM environment_generation_runtime_secrets
                 WHERE generation_id = ?1 AND component_id = ?2 AND resource_name = ?3
                 ORDER BY secret_name",
            )?;
            allocation.secrets = secret_stmt
                .query_map(
                    params![
                        &allocation.generation_id,
                        &allocation.component_id,
                        &allocation.resource_name
                    ],
                    |row| {
                        Ok(RuntimeSecretReference {
                            name: row.get(0)?,
                            provider: row.get(1)?,
                            reference: row.get(2)?,
                            target: row.get(3)?,
                            required: row.get(4)?,
                            purpose: row.get(5)?,
                            environment: row.get(6)?,
                        })
                    },
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
        }
        Ok(allocations)
    }
}

fn validate_runtime_container_ownership(
    allocation: &RuntimeAllocation,
    observation: &RuntimeContainerObservation,
) -> Result<()> {
    let managed = observation
        .labels
        .get(TRAIL_MANAGED_LABEL)
        .is_some_and(|value| value == "true");
    let allocation_matches = observation
        .labels
        .get(TRAIL_ALLOCATION_LABEL)
        .is_some_and(|value| value == &allocation.allocation_id);
    let cleanup_matches = observation
        .labels
        .get(TRAIL_CLEANUP_LABEL)
        .is_some_and(|value| value == &allocation.cleanup_token);
    if !managed || !allocation_matches || !cleanup_matches {
        return Err(Error::InvalidInput(format!(
            "container name `{}` is occupied by a resource Trail does not own; refusing to adopt or replace it",
            allocation.container_name
        )));
    }
    Ok(())
}

fn runtime_secret_binding_digest(secrets: &[ResolvedRuntimeSecret]) -> String {
    let mut bindings = secrets
        .iter()
        .map(|secret| {
            format!(
                "{}\0{}\0{}",
                secret.source_path.to_string_lossy(),
                secret.target,
                secret.environment.as_deref().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    bindings.sort();
    sha256_hex(bindings.join("\0\0").as_bytes())
}

fn runtime_secret_bindings_match(
    allocation: &RuntimeAllocation,
    secrets: &[ResolvedRuntimeSecret],
    observation: &RuntimeContainerObservation,
) -> bool {
    allocation.secrets.is_empty()
        || observation
            .labels
            .get(TRAIL_SECRET_BINDINGS_LABEL)
            .is_some_and(|digest| digest == &runtime_secret_binding_digest(secrets))
}

fn wait_for_tcp_health(host_port: u16, timeout_ms: u64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), host_port);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::InvalidInput(format!(
                "runtime TCP health check timed out after {timeout_ms} ms on 127.0.0.1:{host_port}"
            )));
        }
        if TcpStream::connect_timeout(&address, remaining.min(Duration::from_millis(500))).is_ok() {
            return Ok(());
        }
        thread::sleep(remaining.min(Duration::from_millis(100)));
    }
}

fn validate_runtime_secret_file(path: &std::path::Path) -> Result<()> {
    if !path.is_absolute()
        || path.to_str().map_or(true, |path| {
            path.contains(',') || path.chars().any(char::is_control)
        })
    {
        return Err(Error::InvalidInput(
            "secret provider file handle must be an absolute UTF-8 path without control characters or commas"
                .to_string(),
        ));
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        Error::InvalidInput("secret provider file handle is inaccessible".to_string())
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return Err(Error::InvalidInput(
            "secret provider file handle must be a non-empty regular file, not a symlink"
                .to_string(),
        ));
    }
    if metadata.len() > 1024 * 1024 {
        return Err(Error::InvalidInput(
            "secret provider file handle exceeds one MiB".to_string(),
        ));
    }
    #[cfg(unix)]
    if metadata.mode() & 0o077 != 0 {
        return Err(Error::InvalidInput(
            "secret provider file permissions grant group or other access".to_string(),
        ));
    }
    Ok(())
}

fn provider_output_diagnostic(output: &Output) -> String {
    let bytes = if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    let text = String::from_utf8_lossy(bytes);
    let diagnostic = sanitize_provider_text(&text);
    if diagnostic.is_empty() {
        format!("process exited with {}", output.status)
    } else {
        diagnostic
    }
}

fn sanitize_provider_text(text: &str) -> String {
    let mut sanitized = text
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .collect::<String>();
    for marker in ["token=", "password=", "secret=", "authorization:"] {
        let mut cursor = 0;
        while let Some(relative_start) = sanitized[cursor..].to_ascii_lowercase().find(marker) {
            let start = cursor + relative_start;
            let value_start = start + marker.len();
            let value_end = sanitized[value_start..]
                .find(char::is_whitespace)
                .map_or(sanitized.len(), |offset| value_start + offset);
            sanitized.replace_range(value_start..value_end, "[REDACTED]");
            cursor = value_start + "[REDACTED]".len();
        }
    }
    if sanitized.len() > MAX_PROVIDER_DIAGNOSTIC_BYTES {
        sanitized.truncate(MAX_PROVIDER_DIAGNOSTIC_BYTES);
        sanitized.push('…');
    }
    sanitized.trim().to_string()
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::Mutex;

    use super::*;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[derive(Default)]
    struct FakeState {
        exists: bool,
        running: bool,
        stopped: bool,
        create_count: usize,
        remove_container_count: usize,
        remove_network_count: usize,
        remove_volume_count: usize,
        secret_binding_digest: Option<String>,
        resolved_secret_mounts: Vec<(PathBuf, String, Option<String>)>,
    }

    struct FakeRuntimeProvider {
        listener: TcpListener,
        state: Mutex<FakeState>,
        foreign_labels: bool,
    }

    impl FakeRuntimeProvider {
        fn new(foreign_labels: bool) -> Self {
            Self {
                listener: TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap(),
                state: Mutex::new(FakeState::default()),
                foreign_labels,
            }
        }

        fn create_count(&self) -> usize {
            self.state.lock().unwrap().create_count
        }
    }

    impl RuntimeProvider for FakeRuntimeProvider {
        fn provider_name(&self) -> &str {
            "fake"
        }

        fn ensure_image(&self, _reference: &str, _expected_digest: &str) -> Result<()> {
            Ok(())
        }

        fn ensure_network(&self, _allocation: &RuntimeAllocation) -> Result<()> {
            Ok(())
        }

        fn ensure_volume(&self, _allocation: &RuntimeAllocation) -> Result<()> {
            Ok(())
        }

        fn inspect_container(
            &self,
            allocation: &RuntimeAllocation,
        ) -> Result<Option<RuntimeContainerObservation>> {
            let state = self.state.lock().unwrap();
            if !state.exists {
                return Ok(None);
            }
            let mut labels = if self.foreign_labels {
                BTreeMap::new()
            } else {
                BTreeMap::from([
                    (TRAIL_MANAGED_LABEL.to_string(), "true".to_string()),
                    (
                        TRAIL_ALLOCATION_LABEL.to_string(),
                        allocation.allocation_id.clone(),
                    ),
                    (
                        TRAIL_CLEANUP_LABEL.to_string(),
                        allocation.cleanup_token.clone(),
                    ),
                ])
            };
            if !self.foreign_labels {
                if let Some(digest) = state.secret_binding_digest.as_ref() {
                    labels.insert(TRAIL_SECRET_BINDINGS_LABEL.to_string(), digest.clone());
                }
            }
            Ok(Some(RuntimeContainerObservation {
                provider_resource_id: "fake-container-id".to_string(),
                running: state.running,
                host_port: Some(self.listener.local_addr().unwrap().port()),
                labels,
            }))
        }

        fn create_container(
            &self,
            allocation: &RuntimeAllocation,
            secrets: &[ResolvedRuntimeSecret],
        ) -> Result<String> {
            let mut state = self.state.lock().unwrap();
            state.exists = true;
            state.create_count += 1;
            state.secret_binding_digest =
                (!allocation.secrets.is_empty()).then(|| runtime_secret_binding_digest(secrets));
            state.resolved_secret_mounts = secrets
                .iter()
                .map(|secret| {
                    (
                        secret.source_path.clone(),
                        secret.target.clone(),
                        secret.environment.clone(),
                    )
                })
                .collect();
            Ok("fake-container-id".to_string())
        }

        fn start_container(&self, _allocation: &RuntimeAllocation) -> Result<()> {
            self.state.lock().unwrap().running = true;
            Ok(())
        }

        fn stop_container(&self, _allocation: &RuntimeAllocation) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.running = false;
            state.stopped = true;
            Ok(())
        }

        fn remove_container(&self, _allocation: &RuntimeAllocation) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.exists = false;
            state.running = false;
            state.remove_container_count += 1;
            state.secret_binding_digest = None;
            state.resolved_secret_mounts.clear();
            Ok(())
        }

        fn remove_network(&self, _allocation: &RuntimeAllocation) -> Result<()> {
            self.state.lock().unwrap().remove_network_count += 1;
            Ok(())
        }

        fn remove_volume(&self, _allocation: &RuntimeAllocation) -> Result<()> {
            self.state.lock().unwrap().remove_volume_count += 1;
            Ok(())
        }
    }

    fn write_service_specification(root: &Path) {
        fs::write(
            root.join("trail.oci.toml"),
            format!(
                "schema = \"trail.oci-images/v1\"\n\n[[image]]\nname = \"database-image\"\nreference = \"example.invalid/postgres@{DIGEST}\"\nplatform = \"linux/amd64\"\n\n[[service]]\nname = \"database\"\nimage = \"database-image\"\ncontainer_port = 5432\nhealth_timeout_ms = 1000\nrestart_policy = \"on_failure\"\nvolume_target = \"/var/lib/postgresql/data\"\n"
            ),
        )
        .unwrap();
    }

    fn runtime_workspace(lane: &str) -> (tempfile::TempDir, Trail) {
        let workspace = tempfile::tempdir().unwrap();
        write_service_specification(workspace.path());
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            if cfg!(target_os = "macos") {
                LaneWorkdirMode::NfsCow
            } else if cfg!(target_os = "windows") {
                LaneWorkdirMode::DokanCow
            } else {
                LaneWorkdirMode::FuseCow
            },
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        db.sync_workspace_environment_component(lane, "oci", None, None)
            .unwrap();
        (workspace, db)
    }

    fn runtime_workspace_with_secret(lane: &str, secret_path: &Path) -> (tempfile::TempDir, Trail) {
        let workspace = tempfile::tempdir().unwrap();
        let reference = secret_path.to_string_lossy();
        fs::write(
            workspace.path().join("trail.oci.toml"),
            format!(
                "schema = \"trail.oci-images/v1\"\n\n[[image]]\nname = \"database-image\"\nreference = \"example.invalid/postgres@{DIGEST}\"\nplatform = \"linux/amd64\"\n\n[[service]]\nname = \"database\"\nimage = \"database-image\"\ncontainer_port = 5432\nhealth_timeout_ms = 1000\nrestart_policy = \"on_failure\"\n\n[[service.secret]]\nname = \"database-password\"\nprovider = \"file\"\nreference = {reference:?}\nversion = \"rotation-7\"\npurpose = \"authenticate the database service\"\ninjection = \"file\"\ntarget = \"/run/secrets/database-password\"\nenvironment = \"DATABASE_PASSWORD_FILE\"\nrequired = true\n"
            ),
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            if cfg!(target_os = "macos") {
                LaneWorkdirMode::NfsCow
            } else if cfg!(target_os = "windows") {
                LaneWorkdirMode::DokanCow
            } else {
                LaneWorkdirMode::FuseCow
            },
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        db.sync_workspace_environment_component(lane, "oci", None, None)
            .unwrap();
        (workspace, db)
    }

    fn assert_tree_does_not_contain(root: &Path, needle: &[u8]) {
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                pending.extend(
                    fs::read_dir(path)
                        .unwrap()
                        .map(|entry| entry.unwrap().path()),
                );
            } else if metadata.is_file() {
                let bytes = fs::read(&path).unwrap();
                assert!(
                    !bytes.windows(needle.len()).any(|window| window == needle),
                    "secret canary leaked into {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn reconcile_is_idempotent_and_recovers_a_dead_lifecycle_owner() {
        let lane = "runtime-reconcile";
        let (workspace, db) = runtime_workspace(lane);
        let generation = db.active_environment_generation(lane).unwrap().unwrap();
        let allocations = db
            .runtime_allocations_for_generation(&generation.generation_id)
            .unwrap();
        let provider = FakeRuntimeProvider::new(false);

        db.reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .unwrap();
        db.reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .unwrap();
        assert_eq!(provider.create_count(), 1);
        let running = db.active_environment_generation(lane).unwrap().unwrap();
        let resource = &running.components[0].runtime_resources[0];
        assert_eq!(resource.status, "running");
        assert_eq!(resource.health_status, "healthy");
        assert_eq!(
            resource.provider_resource_id.as_deref(),
            Some("fake-container-id")
        );
        assert!(resource.host_port.is_some());
        let command_environment = db
            .lane_workspace_environment(lane)
            .unwrap()
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            command_environment
                .get("TRAIL_SERVICE_DATABASE_PORT")
                .and_then(|port| port.parse::<u16>().ok()),
            resource.host_port
        );
        let services: Value =
            serde_json::from_str(command_environment.get("TRAIL_SERVICES_JSON").unwrap()).unwrap();
        assert_eq!(
            services["oci-images/database"]["port"].as_u64(),
            resource.host_port.map(u64::from)
        );
        assert!(!db
            .lane_readiness(lane)
            .unwrap()
            .blockers
            .iter()
            .any(|blocker| blocker.code == "environment_runtime_unhealthy"));

        db.conn
            .execute(
                "UPDATE environment_generation_runtime_resources
                 SET status = 'allocating', health_status = 'starting',
                     owner_pid = ?1, owner_start_token = 'dead-owner'",
                params![i64::from(u32::MAX)],
            )
            .unwrap();
        drop(db);

        let reopened = Trail::open(workspace.path()).unwrap();
        let recovered = reopened
            .active_environment_generation(lane)
            .unwrap()
            .unwrap();
        assert_eq!(
            recovered.components[0].runtime_resources[0].status,
            "orphaned"
        );
        let allocations = reopened
            .runtime_allocations_for_generation(&generation.generation_id)
            .unwrap();
        reopened
            .reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .unwrap();
        assert_eq!(provider.create_count(), 1);
        assert_eq!(
            reopened
                .active_environment_generation(lane)
                .unwrap()
                .unwrap()
                .components[0]
                .runtime_resources[0]
                .status,
            "running"
        );
    }

    #[test]
    fn reconcile_rejects_foreign_container_name_collisions() {
        let lane = "runtime-collision";
        let (_workspace, db) = runtime_workspace(lane);
        let generation = db.active_environment_generation(lane).unwrap().unwrap();
        let allocations = db
            .runtime_allocations_for_generation(&generation.generation_id)
            .unwrap();
        let provider = FakeRuntimeProvider::new(true);
        provider.state.lock().unwrap().exists = true;

        let error = db
            .reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .unwrap_err();
        assert!(error.to_string().contains("does not own"));
        let failed = db.active_environment_generation(lane).unwrap().unwrap();
        let resource = &failed.components[0].runtime_resources[0];
        assert_eq!(resource.status, "failed");
        assert_eq!(resource.health_status, "unhealthy");
        assert!(resource.reason.as_deref().unwrap().contains("does not own"));
    }

    #[test]
    fn opaque_file_secret_is_late_bound_revocable_and_never_persisted() {
        let secret_dir = tempfile::tempdir().unwrap();
        let secret_path = secret_dir.path().join("database-password");
        let canary = b"trail-secret-canary-4b9d0f6d7c2a";
        fs::write(&secret_path, canary).unwrap();
        #[cfg(unix)]
        fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o600)).unwrap();

        let lane = "runtime-secret";
        let (workspace, db) = runtime_workspace_with_secret(lane, &secret_path);
        let generation = db.active_environment_generation(lane).unwrap().unwrap();
        let allocations = db
            .runtime_allocations_for_generation(&generation.generation_id)
            .unwrap();
        let provider = FakeRuntimeProvider::new(false);
        db.reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .unwrap();

        let running = db.active_environment_generation(lane).unwrap().unwrap();
        let resource = &running.components[0].runtime_resources[0];
        assert_eq!(resource.secret_statuses.len(), 1);
        assert_eq!(resource.secret_statuses[0].status, "available");
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM environment_secret_access_audit
                     WHERE generation_id = ?1 AND secret_name = 'database-password'
                       AND status = 'available'",
                    params![generation.generation_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            provider.state.lock().unwrap().resolved_secret_mounts,
            vec![(
                fs::canonicalize(&secret_path).unwrap(),
                "/run/secrets/database-password".to_string(),
                Some("DATABASE_PASSWORD_FILE".to_string())
            )]
        );
        assert_tree_does_not_contain(&workspace.path().join(".trail"), canary);

        fs::remove_file(&secret_path).unwrap();
        let error = db
            .reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .unwrap_err();
        assert!(error.to_string().contains("database-password"));
        assert!(!error
            .to_string()
            .contains(&secret_path.to_string_lossy()[..]));
        assert!(!error.to_string().contains("trail-secret-canary"));
        assert!(provider.state.lock().unwrap().stopped);
        let failed = db.active_environment_generation(lane).unwrap().unwrap();
        let resource = &failed.components[0].runtime_resources[0];
        assert_eq!(resource.status, "failed");
        assert_eq!(resource.secret_statuses[0].status, "unavailable");
        assert!(db
            .lane_readiness(lane)
            .unwrap()
            .blockers
            .iter()
            .any(|blocker| blocker.code == "environment_secret_unavailable"));
        assert!(db
            .lane_workspace_environment(lane)
            .unwrap_err()
            .to_string()
            .contains("env runtime reconcile"));
        assert_tree_does_not_contain(&workspace.path().join(".trail"), canary);
    }

    #[test]
    fn rotating_a_secret_file_handle_recreates_the_owned_container() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let first_path = first_dir.path().join("database-password");
        let second_path = second_dir.path().join("database-password");
        fs::write(&first_path, b"first-rotation-canary").unwrap();
        fs::write(&second_path, b"second-rotation-canary").unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(&first_path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::set_permissions(&second_path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let lane = "runtime-secret-rotation";
        let (_workspace, db) = runtime_workspace_with_secret(lane, &first_path);
        let generation = db.active_environment_generation(lane).unwrap().unwrap();
        let mut allocations = db
            .runtime_allocations_for_generation(&generation.generation_id)
            .unwrap();
        let provider = FakeRuntimeProvider::new(false);
        db.reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .unwrap();

        allocations[0].secrets[0].reference = second_path.to_string_lossy().into_owned();
        db.reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .unwrap();

        let state = provider.state.lock().unwrap();
        assert_eq!(state.create_count, 2);
        assert_eq!(state.remove_container_count, 1);
        assert_eq!(
            state.resolved_secret_mounts,
            vec![(
                fs::canonicalize(&second_path).unwrap(),
                "/run/secrets/database-password".to_string(),
                Some("DATABASE_PASSWORD_FILE".to_string())
            )]
        );
    }

    #[test]
    fn secret_provider_handles_inside_workspace_are_rejected_before_runtime_create() {
        let external = tempfile::NamedTempFile::new().unwrap();
        fs::write(external.path(), b"external-placeholder").unwrap();
        #[cfg(unix)]
        fs::set_permissions(external.path(), fs::Permissions::from_mode(0o600)).unwrap();
        let lane = "runtime-workspace-secret";
        let (workspace, db) = runtime_workspace_with_secret(lane, external.path());
        let in_workspace = workspace.path().join("accidental-secret");
        fs::write(&in_workspace, b"must-not-be-mounted").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&in_workspace, fs::Permissions::from_mode(0o600)).unwrap();
        let generation = db.active_environment_generation(lane).unwrap().unwrap();
        let mut allocations = db
            .runtime_allocations_for_generation(&generation.generation_id)
            .unwrap();
        allocations[0].secrets[0].reference = in_workspace.to_string_lossy().into_owned();
        let provider = FakeRuntimeProvider::new(false);

        let error = db
            .reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .unwrap_err();
        assert!(error.to_string().contains("outside the Trail workspace"));
        assert_eq!(provider.create_count(), 0);
        assert_eq!(
            db.active_environment_generation(lane)
                .unwrap()
                .unwrap()
                .components[0]
                .runtime_resources[0]
                .secret_statuses[0]
                .status,
            "unavailable"
        );
    }

    #[test]
    fn new_generations_reuse_private_volume_and_cleanup_retired_ephemera() {
        let lane = "runtime-roll-forward";
        let (_workspace, db) = runtime_workspace(lane);
        let first_generation = db.active_environment_generation(lane).unwrap().unwrap();
        let first = db
            .runtime_allocations_for_generation(&first_generation.generation_id)
            .unwrap();
        let provider = FakeRuntimeProvider::new(false);
        db.reconcile_workspace_environment_runtime_with(&provider, &first)
            .unwrap();

        {
            let mut state = provider.state.lock().unwrap();
            state.exists = false;
            state.running = false;
        }
        db.sync_workspace_environment_component(lane, "oci", None, None)
            .unwrap();
        let second_generation = db.active_environment_generation(lane).unwrap().unwrap();
        let second = db
            .runtime_allocations_for_generation(&second_generation.generation_id)
            .unwrap();
        assert_ne!(first[0].allocation_id, second[0].allocation_id);
        assert_ne!(first[0].container_name, second[0].container_name);
        assert_ne!(first[0].network_name, second[0].network_name);
        assert_eq!(first[0].volume_name, second[0].volume_name);

        db.reconcile_workspace_environment_runtime_with(&provider, &second)
            .unwrap();
        let retired = db
            .runtime_allocations_where(
                "WHERE generation_id = ?1 ORDER BY component_id, resource_name",
                [first_generation.generation_id.as_str()],
            )
            .unwrap();
        db.cleanup_retired_workspace_environment_runtime_with(&provider, &retired)
            .unwrap();

        let state = provider.state.lock().unwrap();
        assert_eq!(state.remove_network_count, 1);
        assert_eq!(state.remove_volume_count, 0);
        drop(state);
        let retired_status = db
            .conn
            .query_row(
                "SELECT status FROM environment_generation_runtime_resources WHERE generation_id = ?1",
                params![first_generation.generation_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(retired_status, "stopped");
    }

    #[test]
    fn provider_diagnostics_are_bounded_and_redact_common_secret_forms() {
        let input = format!(
            "token=abc password=hunter2 secret=value authorization:Bearer-value {}",
            "x".repeat(MAX_PROVIDER_DIAGNOSTIC_BYTES * 2)
        );
        let sanitized = sanitize_provider_text(&input);
        assert!(!sanitized.contains("abc"));
        assert!(!sanitized.contains("hunter2"));
        assert!(!sanitized.contains("Bearer-value"));
        assert!(sanitized.len() <= MAX_PROVIDER_DIAGNOSTIC_BYTES + '…'.len_utf8());
    }

    #[test]
    fn secret_file_validation_rejects_empty_broad_and_symlink_handles() {
        let directory = tempfile::tempdir().unwrap();
        let empty = directory.path().join("empty");
        fs::write(&empty, b"").unwrap();
        assert!(validate_runtime_secret_file(&empty).is_err());

        let broad = directory.path().join("broad");
        fs::write(&broad, b"not-a-real-secret").unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(&broad, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(validate_runtime_secret_file(&broad).is_err());
            fs::set_permissions(&broad, fs::Permissions::from_mode(0o600)).unwrap();
            let symlink = directory.path().join("symlink");
            std::os::unix::fs::symlink(&broad, &symlink).unwrap();
            assert!(validate_runtime_secret_file(&symlink).is_err());
        }
    }

    #[test]
    fn secret_status_and_access_audit_commit_or_rollback_together() {
        let secret = tempfile::NamedTempFile::new().unwrap();
        fs::write(secret.path(), b"audit-rollback-canary").unwrap();
        #[cfg(unix)]
        fs::set_permissions(secret.path(), fs::Permissions::from_mode(0o600)).unwrap();
        let lane = "runtime-secret-audit";
        let (_workspace, db) = runtime_workspace_with_secret(lane, secret.path());
        let generation = db.active_environment_generation(lane).unwrap().unwrap();
        let allocations = db
            .runtime_allocations_for_generation(&generation.generation_id)
            .unwrap();
        db.conn
            .execute_batch(
                "CREATE TRIGGER reject_secret_access_audit
                 BEFORE INSERT ON environment_secret_access_audit
                 BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;",
            )
            .unwrap();
        let provider = FakeRuntimeProvider::new(false);

        assert!(db
            .reconcile_workspace_environment_runtime_with(&provider, &allocations)
            .is_err());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT status FROM environment_generation_runtime_secrets
                     WHERE generation_id = ?1",
                    params![generation.generation_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "pending"
        );
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM environment_secret_access_audit",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        assert_eq!(provider.create_count(), 0);
    }
}
