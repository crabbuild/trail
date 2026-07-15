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
mod daemon_start;
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

fn resolve_agent_provider_argument(
    positional: Option<String>,
    named: Option<String>,
    fallback: Option<&str>,
) -> Result<String> {
    match (positional, named) {
        (Some(_), Some(_)) => Err(Error::InvalidInput(
            "provider may be supplied either positionally or with --provider, not both".to_string(),
        )),
        (Some(provider), None) | (None, Some(provider)) => Ok(provider),
        (None, None) => fallback.map(ToString::to_string).ok_or_else(|| {
            Error::InvalidInput(
                "choose a provider positionally or with --provider <PROVIDER>".to_string(),
            )
        }),
    }
}

pub(crate) fn run_cli() {
    let json_errors =
        args_request_json_errors(std::env::args_os().skip(1)) || env_requests_json_errors();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => handle_cli_parse_error(err, json_errors),
    };
    let json_errors = cli.json
        || matches!(
            cli.format.as_ref(),
            Some(OutputFormat::Json | OutputFormat::Ndjson)
        )
        || env_requests_json_errors();
    if let Err(err) = run(cli) {
        render_error(&err, json_errors);
        std::process::exit(err.exit_code());
    }
}

fn run(cli: Cli) -> Result<()> {
    let format = resolve_output_format(cli.format)?;
    let json = cli.json || matches!(format, OutputFormat::Json);
    let render_mode = if matches!(format, OutputFormat::Plain) {
        RenderMode::Plain
    } else {
        RenderMode::Human
    };
    let render = RenderOptions::from_environment(
        render_mode,
        cli.color.as_policy(),
        cli.pager.as_policy(),
        cli.verbose,
        cli.quiet,
    );
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
        format,
        render,
    };
    let command = cli.command;
    if matches!(ctx.format, OutputFormat::Ndjson) && !supports_ndjson(&command) {
        return Err(Error::InvalidInput(
            "--format ndjson is available only for streaming watch commands; use --format json for a single report"
                .to_string(),
        ));
    }
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
            render_init(&report, ctx.json, &ctx.render)
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

fn supports_ndjson(command: &Command) -> bool {
    match command {
        Command::Watch(_) => true,
        Command::Index(IndexCommand {
            command: IndexSubcommand::Watch(_),
        }) => true,
        Command::Lane(LaneCommand {
            command: LaneSubcommand::Watch(args),
        }) => !args.once,
        _ => false,
    }
}

fn render_specialist<T: serde::Serialize>(
    ctx: &RuntimeContext,
    title: &str,
    report: &T,
) -> Result<()> {
    render_semantic_report(title, report, ctx.json, &ctx.render)
}

fn handle_environment_command(ctx: &RuntimeContext, environment: EnvironmentCommand) -> Result<()> {
    let db = open_db(ctx)?;
    match environment.command {
        EnvironmentSubcommand::Adapters => render_specialist(
            ctx,
            "Environment adapters",
            &db.workspace_environment_adapters()?,
        ),
        EnvironmentSubcommand::Plugin(plugin) => match plugin.command {
            EnvironmentPluginSubcommand::Inspect(args) => render_specialist(
                ctx,
                "Environment adapter package",
                &db.inspect_environment_adapter_plugin_package(&args.package)?,
            ),
            EnvironmentPluginSubcommand::Install(args) => render_specialist(
                ctx,
                "Installed environment adapter",
                &db.install_environment_adapter_plugin(&args.package)?,
            ),
            EnvironmentPluginSubcommand::Remove(args) => render_specialist(
                ctx,
                "Updated environment adapter",
                &db.remove_environment_adapter_plugin(&args.identity)?,
            ),
            EnvironmentPluginSubcommand::Trust(args) => match args.command {
                EnvironmentPluginTrustSubcommand::Add(args) => render_specialist(
                    ctx,
                    "Trusted adapter publisher",
                    &db.trust_environment_adapter_publisher_key(&args.key)?,
                ),
                EnvironmentPluginTrustSubcommand::List => render_specialist(
                    ctx,
                    "Trusted adapter publishers",
                    &db.environment_adapter_publisher_trust()?,
                ),
                EnvironmentPluginTrustSubcommand::Remove(args) => render_specialist(
                    ctx,
                    "Removed adapter publisher trust",
                    &db.remove_environment_adapter_publisher_key(&args.key_id)?,
                ),
            },
        },
        EnvironmentSubcommand::Discover(args) => render_specialist(
            ctx,
            "Environment discovery",
            &db.discover_workspace_environment(&args.lane, args.path.as_deref())?,
        ),
        EnvironmentSubcommand::Graph(args) => render_specialist(
            ctx,
            "Environment graph",
            &db.workspace_environment_graph_page(
                &args.lane,
                args.path.as_deref(),
                args.offset,
                args.limit,
            )?,
        ),
        EnvironmentSubcommand::Status(args) => render_specialist(
            ctx,
            "Environment status",
            &db.environment_component_status(&args.lane)?,
        ),
        EnvironmentSubcommand::Generation(args) => render_specialist(
            ctx,
            "Environment generation",
            &db.active_environment_generation(&args.lane)?,
        ),
        EnvironmentSubcommand::Explain(args) => render_specialist(
            ctx,
            "Environment staleness",
            &db.explain_workspace_environment_staleness_page(
                &args.lane,
                &args.component,
                args.offset,
                args.limit,
            )?,
        ),
        EnvironmentSubcommand::Plan(args) => render_specialist(
            ctx,
            "Environment plan",
            &db.plan_workspace_environment_component(
                &args.lane,
                &args.adapter,
                args.path.as_deref(),
                args.component.as_deref(),
            )?,
        ),
        EnvironmentSubcommand::Sync(args) => render_specialist(
            ctx,
            "Synchronized environment",
            &db.sync_workspace_environment_component_with_runtime(
                &args.lane,
                &args.adapter,
                args.path.as_deref(),
                args.component.as_deref(),
            )?,
        ),
        EnvironmentSubcommand::SyncAll(args) => render_specialist(
            ctx,
            "Synchronized environments",
            &db.sync_all_workspace_environments_with_runtime(&args.lane, args.path.as_deref())?,
        ),
        EnvironmentSubcommand::Runtime(runtime) => {
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
            render_specialist(ctx, "Environment runtime", &generation)
        }
    }
}

fn handle_deps_command(ctx: &RuntimeContext, deps: DepsCommand) -> Result<()> {
    let db = open_db(ctx)?;
    match deps.command {
        DepsSubcommand::Status(args) => {
            db.refresh_workspace_environment_staleness(&args.lane)?;
            render_specialist(
                ctx,
                "Dependency status",
                &db.workspace_environment_status(&args.lane)?,
            )
        }
        DepsSubcommand::Sync(args) => render_specialist(
            ctx,
            "Synchronized dependencies",
            &db.sync_node_dependencies(&args.lane, args.path.as_deref())?,
        ),
    }
}

fn handle_cache_command(ctx: &RuntimeContext, cache: CacheCommand) -> Result<()> {
    let db = open_db(ctx)?;
    match cache.command {
        CacheSubcommand::List => {
            render_specialist(ctx, "Workspace cache", &db.list_workspace_layers()?)
        }
        CacheSubcommand::Verify(args) | CacheSubcommand::Inspect(args) => render_specialist(
            ctx,
            "Workspace cache layer",
            &db.verify_workspace_layer(&args.layer)?,
        ),
        CacheSubcommand::Gc(args) => render_specialist(
            ctx,
            if args.dry_run {
                "Workspace cache cleanup preview"
            } else {
                "Workspace cache cleanup"
            },
            &db.workspace_cache_gc(args.dry_run, args.retention_secs)?,
        ),
    }
}
