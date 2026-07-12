use std::path::PathBuf;

use clap::Parser;

use super::{render::*, *};

use trail::{
    acp::AcpRelayOptions, Actor, Error, InitImportMode, LaneGateOptions, OperationKind,
    PatchDocument, RecordOptions, Result, Trail,
};

mod acp;
mod agent;
mod collaboration;
mod daemon_rpc;
mod errors;
mod inspect;
mod lane;
mod maintenance;
mod parsing;
mod runtime;
mod workspace;
mod worktree;

use errors::*;
use parsing::*;
use runtime::*;

pub(crate) fn run_cli() {
    let json_errors =
        args_request_json_errors(std::env::args_os().skip(1)) || env_requests_json_errors();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => handle_cli_parse_error(err, json_errors),
    };
    let json_errors = cli.json
        || matches!(cli.format.as_ref(), Some(OutputFormat::Json))
        || env_requests_json_errors();
    if let Err(err) = run(cli) {
        render_error(&err, json_errors);
        std::process::exit(err.exit_code());
    }
}

fn run(cli: Cli) -> Result<()> {
    let format = resolve_output_format(cli.format)?;
    let json = cli.json || matches!(format, OutputFormat::Json);
    let workspace = cli
        .workspace
        .clone()
        .or_else(|| std::env::var_os("TRAIL_WORKSPACE").map(PathBuf::from));
    let db_dir = cli
        .db
        .clone()
        .or_else(|| std::env::var_os("TRAIL_DIR").map(PathBuf::from));
    let branch = cli
        .branch
        .clone()
        .or_else(|| std::env::var("TRAIL_BRANCH").ok());
    let daemon_url = cli
        .daemon_url
        .clone()
        .or_else(|| std::env::var("TRAIL_DAEMON_URL").ok())
        .filter(|value| !value.trim().is_empty());
    let daemon_token = cli
        .daemon_token
        .clone()
        .or_else(|| std::env::var("TRAIL_DAEMON_TOKEN").ok())
        .filter(|value| !value.trim().is_empty());
    let ctx = RuntimeContext {
        workspace,
        db_dir,
        branch,
        json,
        quiet: cli.quiet,
        color: !cli.no_color && std::env::var_os("NO_COLOR").is_none(),
        format,
    };
    let command = cli.command;
    if let Some(daemon_url) = daemon_url {
        if daemon_rpc::try_handle_daemon_command(
            &ctx,
            Some(daemon_url),
            daemon_token.clone(),
            &command,
        )? {
            return Ok(());
        }
    } else if daemon_rpc::try_handle_auto_daemon_command(&ctx, daemon_token.clone(), &command)? {
        return Ok(());
    }
    match command {
        Command::Init(args) => {
            let workspace = ctx
                .workspace
                .clone()
                .unwrap_or(std::env::current_dir().map_err(Error::from)?);
            let mode = if args.from_git {
                InitImportMode::GitTracked
            } else if args.working_tree {
                InitImportMode::WorkingTree
            } else {
                InitImportMode::Empty
            };
            let report = Trail::init_with_text_policy_and_prolly_backend(
                workspace,
                args.branch,
                mode,
                args.force,
                args.text_policy.as_ref().map(TextPolicyArg::as_str),
                args.prolly_backend.as_ref().map(ProllyBackendArg::as_str),
            )?;
            render_init(&report, ctx.json, ctx.quiet)
        }
        Command::Config(config) => workspace::handle_config_command(&ctx, config),
        Command::Ignore(ignore) => workspace::handle_ignore_command(&ctx, ignore),
        Command::Guardrails(guardrails) => workspace::handle_guardrails_command(&ctx, guardrails),
        Command::Status(args) => worktree::handle_status_command(&ctx, args),
        Command::Record(args) => worktree::handle_record_command(&ctx, args),
        Command::Watch(args) => worktree::handle_watch_command(&ctx, args),
        Command::Timeline(args) => inspect::handle_timeline_command(&ctx, args),
        Command::Show(args) => inspect::handle_show_command(&ctx, args),
        Command::Object(object) => inspect::handle_object_command(&ctx, object),
        Command::Root(root) => inspect::handle_root_command(&ctx, root),
        Command::Text(text) => inspect::handle_text_command(&ctx, text),
        Command::Map(map) => inspect::handle_map_command(&ctx, map),
        Command::Diff(args) => worktree::handle_diff_command(&ctx, args),
        Command::Checkout(args) => worktree::handle_checkout_command(&ctx, args),
        Command::Branch(args) => worktree::handle_branch_command(&ctx, args),
        Command::Merge(args) => worktree::handle_merge_command(&ctx, args),
        Command::Why(args) => inspect::handle_why_command(&ctx, args),
        Command::History(args) => inspect::handle_history_command(&ctx, args),
        Command::CodeFrom(args) => inspect::handle_code_from_command(&ctx, args),
        Command::Lane(lane_command) => lane::handle_lane_command(&ctx, lane_command),
        Command::Deps(deps) => handle_deps_command(&ctx, deps),
        Command::Env(environment) => handle_environment_command(&ctx, environment),
        Command::Cache(cache) => handle_cache_command(&ctx, cache),
        Command::Acp(acp_command) => acp::handle_acp_command(&ctx, acp_command),
        Command::Agent(agent_command) => agent::handle_agent_command(&ctx, agent_command),
        Command::Transcript(args) => acp::handle_transcript_command(&ctx, args),
        Command::Turn(turn) => acp::handle_top_turn_command(&ctx, turn),
        Command::Session(session_command) => {
            collaboration::handle_session_command(&ctx, session_command)
        }
        Command::Approvals(approvals_command) => {
            collaboration::handle_approvals_command(&ctx, approvals_command)
        }
        Command::MergeLane(args) => collaboration::handle_merge_lane_command(&ctx, args),
        Command::MergeQueue(queue) => collaboration::handle_merge_queue_command(&ctx, queue),
        Command::Conflicts(conflicts) => collaboration::handle_conflicts_command(&ctx, conflicts),
        Command::Anchor(anchor) => collaboration::handle_anchor_command(&ctx, anchor),
        Command::Lease(lease) => collaboration::handle_lease_command(&ctx, lease),
        Command::Git(git) => maintenance::handle_git_command(&ctx, git),
        Command::Api(api) => maintenance::handle_api_command(&ctx, api),
        Command::Daemon(args) => maintenance::handle_daemon_command(&ctx, args),
        Command::Mcp => maintenance::handle_mcp_command(&ctx),
        Command::Doctor => maintenance::handle_doctor_command(&ctx),
        Command::Backup(backup) => maintenance::handle_backup_command(&ctx, backup),
        Command::Fsck => maintenance::handle_fsck_command(&ctx),
        Command::Index(index) => maintenance::handle_index_command(&ctx, index),
        Command::Gc(args) => maintenance::handle_gc_command(&ctx, args),
    }
}

fn handle_environment_command(ctx: &RuntimeContext, environment: EnvironmentCommand) -> Result<()> {
    match environment.command {
        EnvironmentSubcommand::Adapters => {
            let db = open_db(ctx)?;
            let report = db.workspace_environment_adapters()?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    for adapter in report.adapters {
                        println!(
                            "{} kind={} stability={} trust={} certification={} protocols={} selectors={} markers={} platforms={}/{}",
                            adapter.canonical_identity,
                            adapter.kind,
                            adapter.stability,
                            adapter.trust,
                            adapter.certification_tier,
                            if adapter.protocols.is_empty() {
                                "in-process".to_string()
                            } else {
                                adapter.protocols.join(",")
                            },
                            adapter.selectors.join(","),
                            adapter.discovery_markers.join(","),
                            adapter.supported_operating_systems.join(","),
                            adapter.supported_architectures.join(",")
                        );
                        println!("  {}", adapter.description);
                    }
                }
                Ok(())
            }
        }
        EnvironmentSubcommand::Plugin(plugin) => {
            let db = open_db(ctx)?;
            match plugin.command {
                EnvironmentPluginSubcommand::Inspect(args) => {
                    let report = db.inspect_environment_adapter_plugin_package(&args.package)?;
                    if ctx.json {
                        render_json(&report)
                    } else {
                        if !ctx.quiet {
                            println!(
                                "Adapter {} payload={} distribution={} executable={} signed={}",
                                report.canonical_identity,
                                report.payload_digest,
                                report.distribution_digest,
                                report.executable_digest,
                                report.signature_present
                            );
                        }
                        Ok(())
                    }
                }
                EnvironmentPluginSubcommand::Install(args) => {
                    let report = db.install_environment_adapter_plugin(&args.package)?;
                    if ctx.json {
                        render_json(&report)
                    } else {
                        if !ctx.quiet {
                            println!(
                                "Installed adapter {} ({}) trust={} certification={}",
                                report.canonical_identity,
                                report.distribution_digest,
                                report.trust,
                                report.certification_tier
                            );
                            if let (Some(publisher), Some(key_id)) =
                                (&report.publisher, &report.publisher_key_id)
                            {
                                println!("  publisher: {publisher} ({key_id})");
                            }
                            if let Some(previous) = report.replaced_distribution_digest {
                                println!("  replaced: {previous}");
                            }
                        }
                        Ok(())
                    }
                }
                EnvironmentPluginSubcommand::Remove(args) => {
                    let report = db.remove_environment_adapter_plugin(&args.identity)?;
                    if ctx.json {
                        render_json(&report)
                    } else {
                        if !ctx.quiet {
                            if let Some(digest) = report.removed_distribution_digest {
                                println!(
                                    "Removed adapter {} ({digest})",
                                    report.canonical_identity
                                );
                            } else {
                                println!("Adapter {} was not installed", report.canonical_identity);
                            }
                        }
                        Ok(())
                    }
                }
                EnvironmentPluginSubcommand::Trust(args) => match args.command {
                    EnvironmentPluginTrustSubcommand::Add(args) => {
                        let report = db.trust_environment_adapter_publisher_key(&args.key)?;
                        if ctx.json {
                            render_json(&report)
                        } else {
                            if !ctx.quiet {
                                println!(
                                    "Trusted publisher {} ({})",
                                    report.publisher.as_deref().unwrap_or("unknown"),
                                    report.key_id
                                );
                            }
                            Ok(())
                        }
                    }
                    EnvironmentPluginTrustSubcommand::List => {
                        let report = db.environment_adapter_publisher_trust()?;
                        if ctx.json {
                            render_json(&report)
                        } else {
                            if !ctx.quiet {
                                for key in report.keys {
                                    println!(
                                        "{} publisher={} trusted_at={}",
                                        key.key_id, key.publisher, key.trusted_at
                                    );
                                }
                            }
                            Ok(())
                        }
                    }
                    EnvironmentPluginTrustSubcommand::Remove(args) => {
                        let report = db.remove_environment_adapter_publisher_key(&args.key_id)?;
                        if ctx.json {
                            render_json(&report)
                        } else {
                            if !ctx.quiet {
                                println!("Removed publisher trust {}", report.key_id);
                            }
                            Ok(())
                        }
                    }
                },
            }
        }
        EnvironmentSubcommand::Discover(args) => {
            let db = open_db(ctx)?;
            let report = db.discover_workspace_environment(&args.lane, args.path.as_deref())?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    for component in report.components {
                        println!(
                            "{} root={} kind={} adapter={}",
                            component.component_id,
                            if component.component_root.is_empty() {
                                "."
                            } else {
                                &component.component_root
                            },
                            component.kind,
                            component.adapter_identity
                        );
                    }
                    for conflict in report.conflicts {
                        println!(
                            "conflict root={} adapters={}: {}",
                            if conflict.component_root.is_empty() {
                                "."
                            } else {
                                &conflict.component_root
                            },
                            conflict.adapter_identities.join(","),
                            conflict.reason
                        );
                    }
                }
                Ok(())
            }
        }
        EnvironmentSubcommand::Graph(args) => {
            let db = open_db(ctx)?;
            let report = db.workspace_environment_graph_page(
                &args.lane,
                args.path.as_deref(),
                args.offset,
                args.limit,
            )?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    println!(
                        "source={} components={} edges={}",
                        report.source_root.0, report.total_nodes, report.total_edges
                    );
                    for node in &report.nodes {
                        println!(
                            "[{}] {} root={} kind={} adapter={} key={}",
                            node.topological_index,
                            node.component_id,
                            if node.component_root.is_empty() {
                                "."
                            } else {
                                &node.component_root
                            },
                            node.kind,
                            node.adapter_identity,
                            node.component_key
                        );
                        for output in &node.outputs {
                            println!(
                                "  output {} [{}] -> {}",
                                output.name, output.policy, output.mount_path
                            );
                        }
                        for cache in &node.caches {
                            println!(
                                "  cache {} [{}; {}] namespace={}",
                                cache.name, cache.protocol, cache.access, cache.namespace_id
                            );
                        }
                        for artifact in &node.external_artifacts {
                            println!(
                                "  external {} [{}; {}] {} platform={}",
                                artifact.name,
                                artifact.artifact_type,
                                artifact.provider,
                                artifact.reference,
                                artifact.platform
                            );
                        }
                        for resource in &node.runtime_resources {
                            println!(
                                "  runtime {} [{}] image={} port={}/{} restart={} volume={}",
                                resource.name,
                                resource.runtime_type,
                                resource.artifact_name,
                                resource.container_port,
                                resource.protocol,
                                resource.restart_policy,
                                resource.volume_target.as_deref().unwrap_or("-")
                            );
                            for secret in &resource.secrets {
                                println!(
                                    "    secret-ref {} provider={} injection={} target={} required={}",
                                    secret.name,
                                    secret.provider,
                                    secret.injection,
                                    secret.target,
                                    secret.required
                                );
                            }
                        }
                    }
                    for edge in &report.edges {
                        println!(
                            "{} -> {} [{}] source-key={}",
                            edge.source_component_id,
                            edge.target_component_id,
                            edge.edge_type,
                            edge.source_component_key
                        );
                    }
                    if let Some(next) = report.next_offset {
                        println!("more nodes: rerun with --offset {next}");
                    }
                }
                Ok(())
            }
        }
        EnvironmentSubcommand::Status(args) => {
            let db = open_db(ctx)?;
            let report = db.environment_component_status(&args.lane)?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    for component in report {
                        println!(
                            "{} {} kind={} adapter={}/{}@{}:{} expected={} attached={}",
                            component.component.component_id,
                            component.status,
                            component.component.kind,
                            component.adapter.namespace,
                            component.adapter.name,
                            component.adapter.contract_major,
                            component.adapter.implementation_version,
                            component.expected_key,
                            component.attached_key.as_deref().unwrap_or("-")
                        );
                        if let Some(reason) = component.reason {
                            println!("  reason: {reason}");
                        }
                    }
                }
                Ok(())
            }
        }
        EnvironmentSubcommand::Generation(args) => {
            let db = open_db(ctx)?;
            let report = db.active_environment_generation(&args.lane)?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    if let Some(generation) = report {
                        println!(
                            "{} sequence={} state={} source={} components={}",
                            generation.generation_id,
                            generation.generation_sequence,
                            generation.state,
                            generation.source_root.0,
                            generation.components.len()
                        );
                        for component in generation.components {
                            println!(
                                "  {} {} key={} layer={} mount={}",
                                component.component_id,
                                component.adapter_identity,
                                component.component_key,
                                component.layer_id.as_deref().unwrap_or("-"),
                                component.mount_path.as_deref().unwrap_or("-")
                            );
                            for dependency in component.dependencies {
                                println!(
                                    "    {} {} key={}",
                                    dependency.edge_type,
                                    dependency.component_id,
                                    dependency.component_key
                                );
                            }
                            for cache in component.caches {
                                println!(
                                    "    cache {} [{}; {}] namespace={}",
                                    cache.name, cache.protocol, cache.access, cache.namespace_id
                                );
                            }
                            for artifact in component.external_artifacts {
                                println!(
                                    "    external {} [{}; {}] {} platform={}",
                                    artifact.name,
                                    artifact.artifact_type,
                                    artifact.provider,
                                    artifact.reference,
                                    artifact.platform
                                );
                            }
                            for resource in component.runtime_resources {
                                println!(
                                    "    runtime {} status={} health={} container={} network={} host-port={} volume={}",
                                    resource.declaration.name,
                                    resource.status,
                                    resource.health_status,
                                    resource.container_name,
                                    resource.network_name,
                                    resource
                                        .host_port
                                        .map(|port| port.to_string())
                                        .as_deref()
                                        .unwrap_or("-"),
                                    resource.volume_name.as_deref().unwrap_or("-")
                                );
                                if let Some(reason) = resource.reason {
                                    println!("      reason: {reason}");
                                }
                                for secret in resource.secret_statuses {
                                    println!(
                                        "      secret-ref {} provider={} status={} required={}",
                                        secret.reference.name,
                                        secret.reference.provider,
                                        secret.status,
                                        secret.reference.required
                                    );
                                    if let Some(reason) = secret.reason {
                                        println!("        reason: {reason}");
                                    }
                                }
                            }
                        }
                    } else {
                        println!("No active environment generation");
                    }
                }
                Ok(())
            }
        }
        EnvironmentSubcommand::Explain(args) => {
            let db = open_db(ctx)?;
            let report = db.explain_workspace_environment_staleness_page(
                &args.lane,
                &args.component,
                args.offset,
                args.limit,
            )?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    println!(
                        "{} status={} complete={} expected={} attached={}",
                        report.component_id,
                        report.status,
                        report.complete,
                        report.expected_key,
                        report.attached_key.as_deref().unwrap_or("-")
                    );
                    for change in report.changes {
                        println!("  {}:{} {}", change.dimension, change.name, change.change);
                    }
                    if let Some(next) = report.next_offset {
                        println!("  more changes: rerun with --offset {next}");
                    }
                }
                Ok(())
            }
        }
        EnvironmentSubcommand::Plan(args) => {
            let db = open_db(ctx)?;
            let report = db.plan_workspace_environment_component(
                &args.lane,
                &args.adapter,
                args.path.as_deref(),
                args.component.as_deref(),
            )?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    println!(
                        "{} kind={} adapter={} key={} mount={}",
                        report.component_id,
                        report.kind,
                        report.adapter_identity,
                        report.component_key,
                        report.mount_path
                    );
                    for output in &report.outputs {
                        println!(
                            "  output {} [{}]: {} -> {}",
                            output.name, output.policy, output.output_path, output.mount_path
                        );
                    }
                    for dependency in &report.dependency_edges {
                        println!(
                            "  {} {} key={}",
                            dependency.edge_type, dependency.component_id, dependency.component_key
                        );
                    }
                    for cache in &report.caches {
                        println!(
                            "  cache {} [{}; {}] namespace={}",
                            cache.name, cache.protocol, cache.access, cache.namespace_id
                        );
                    }
                    for artifact in &report.external_artifacts {
                        println!(
                            "  external {} [{}; {}] {} platform={}",
                            artifact.name,
                            artifact.artifact_type,
                            artifact.provider,
                            artifact.reference,
                            artifact.platform
                        );
                    }
                    for resource in &report.runtime_resources {
                        println!(
                            "  runtime {} [{}] image={} port={}/{} restart={} health={} volume={}",
                            resource.name,
                            resource.runtime_type,
                            resource.artifact_name,
                            resource.container_port,
                            resource.protocol,
                            resource.restart_policy,
                            resource.health_type,
                            resource.volume_target.as_deref().unwrap_or("-")
                        );
                        for secret in &resource.secrets {
                            println!(
                                "    secret-ref {} provider={} injection={} target={} required={}",
                                secret.name,
                                secret.provider,
                                secret.injection,
                                secret.target,
                                secret.required
                            );
                        }
                    }
                    println!(
                        "  capabilities: sandbox={} network={} shell={} scripts={} secrets={}",
                        report.capabilities.sandbox,
                        report.capabilities.network,
                        report.capabilities.shell,
                        report.capabilities.scripts,
                        report.capabilities.secrets
                    );
                    for (name, identity) in &report.tools {
                        println!("  tool {name}: {identity}");
                    }
                    if !report.external_artifacts.is_empty() {
                        println!(
                            "  action: metadata-only; Trail records provider-owned immutable identities without creating filesystem layers"
                        );
                    } else if report.commands.is_empty() {
                        println!(
                            "  action: provision private output; run path-sensitive tools inside the mounted lane"
                        );
                    }
                    for command in report.commands {
                        println!(
                            "  command [{}]: {} {} cwd={}",
                            command.phase,
                            command.program,
                            command.args.join(" "),
                            command.working_directory
                        );
                    }
                }
                Ok(())
            }
        }
        EnvironmentSubcommand::Sync(args) => {
            let db = open_db(ctx)?;
            let report = db.sync_workspace_environment_component_with_runtime(
                &args.lane,
                &args.adapter,
                args.path.as_deref(),
                args.component.as_deref(),
            )?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    let private_outputs = report
                        .generation
                        .components
                        .iter()
                        .flat_map(|component| &component.outputs)
                        .filter(|output| output.policy == "writable_private")
                        .count();
                    println!(
                        "Synchronized environment generation {} ({} shared layer(s), {} writable-private output(s))",
                        report.generation.generation_id,
                        report.layers.len(),
                        private_outputs
                    );
                }
                Ok(())
            }
        }
        EnvironmentSubcommand::SyncAll(args) => {
            let db = open_db(ctx)?;
            let report =
                db.sync_all_workspace_environments_with_runtime(&args.lane, args.path.as_deref())?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    println!(
                        "Synchronized {} components as generation {}",
                        report.generation.components.len(),
                        report.generation.generation_id
                    );
                }
                Ok(())
            }
        }
        EnvironmentSubcommand::Runtime(runtime) => {
            let db = open_db(ctx)?;
            let generation = match runtime.command {
                EnvironmentRuntimeSubcommand::Status(args) => db
                    .active_environment_generation(&args.lane)?
                    .ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "lane `{}` has no active environment generation",
                            args.lane
                        ))
                    })?,
                EnvironmentRuntimeSubcommand::Reconcile(args) => {
                    db.reconcile_workspace_environment_runtime(&args.lane)?
                }
                EnvironmentRuntimeSubcommand::Stop(args) => {
                    db.stop_workspace_environment_runtime(&args.lane)?
                }
            };
            if ctx.json {
                render_json(&generation)
            } else {
                if !ctx.quiet {
                    let mut count = 0usize;
                    for component in generation.components {
                        for resource in component.runtime_resources {
                            count += 1;
                            println!(
                                "{}:{} status={} health={} provider={} container={} endpoint={} network={} volume={}",
                                component.component_id,
                                resource.declaration.name,
                                resource.status,
                                resource.health_status,
                                resource.declaration.provider,
                                resource.container_name,
                                resource
                                    .host_port
                                    .map(|port| format!("127.0.0.1:{port}"))
                                    .unwrap_or_else(|| "-".to_string()),
                                resource.network_name,
                                resource.volume_name.as_deref().unwrap_or("-")
                            );
                            if let Some(reason) = resource.reason {
                                println!("  reason: {reason}");
                            }
                            for secret in resource.secret_statuses {
                                println!(
                                    "  secret-ref {} provider={} status={} required={}",
                                    secret.reference.name,
                                    secret.reference.provider,
                                    secret.status,
                                    secret.reference.required
                                );
                            }
                        }
                    }
                    if count == 0 {
                        println!("No runtime resources declared in the active generation");
                    }
                }
                Ok(())
            }
        }
    }
}

fn handle_deps_command(ctx: &RuntimeContext, deps: DepsCommand) -> Result<()> {
    match deps.command {
        DepsSubcommand::Status(args) => {
            let db = open_db(ctx)?;
            db.refresh_workspace_environment_staleness(&args.lane)?;
            let report = db.workspace_environment_status(&args.lane)?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    for environment in report {
                        println!(
                            "{} {} expected={} attached={}",
                            environment.adapter,
                            environment.status,
                            environment.expected_key,
                            environment.attached_key.as_deref().unwrap_or("-")
                        );
                    }
                }
                Ok(())
            }
        }
        DepsSubcommand::Sync(args) => {
            let db = open_db(ctx)?;
            let report = db.sync_node_dependencies(&args.lane, args.path.as_deref())?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    println!(
                        "Replaced dependency state with layer {} ({}, {} bytes)",
                        report.layer_id, report.adapter, report.logical_bytes
                    );
                }
                Ok(())
            }
        }
    }
}

fn handle_cache_command(ctx: &RuntimeContext, cache: CacheCommand) -> Result<()> {
    let db = open_db(ctx)?;
    match cache.command {
        CacheSubcommand::List => {
            let report = db.list_workspace_layers()?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    for layer in report {
                        println!(
                            "{} {} {} logical={} physical={}",
                            layer.layer_id,
                            layer.adapter,
                            layer.state,
                            layer.logical_bytes,
                            layer
                                .physical_bytes
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "unknown".to_string())
                        );
                    }
                }
                Ok(())
            }
        }
        CacheSubcommand::Verify(args) | CacheSubcommand::Inspect(args) => {
            let report = db.verify_workspace_layer(&args.layer)?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    println!(
                        "{} {} {} entries={} bytes={}",
                        report.layer_id,
                        report.adapter,
                        report.state,
                        report.entry_count,
                        report.logical_bytes
                    );
                }
                Ok(())
            }
        }
        CacheSubcommand::Gc(args) => {
            let report = db.workspace_cache_gc(args.dry_run, args.retention_secs)?;
            if ctx.json {
                render_json(&report)
            } else {
                if !ctx.quiet {
                    println!(
                        "cache gc {}: candidates={} reclaimable={} reclaimed={}",
                        if report.dry_run {
                            "dry-run"
                        } else {
                            "complete"
                        },
                        report.candidates.len(),
                        report.reclaimable_bytes,
                        report.reclaimed_bytes
                    );
                    for item in &report.candidates {
                        println!(
                            "{} {} bytes={} reason={}",
                            item.kind, item.id, item.physical_bytes, item.reason
                        );
                    }
                }
                Ok(())
            }
        }
    }
}
