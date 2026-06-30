use std::path::PathBuf;

use clap::Parser;

use super::{render::*, *};

use crabdb::{
    acp::AcpRelayOptions, Actor, CrabDb, Error, InitImportMode, LaneGateOptions, OperationKind,
    PatchDocument, RecordOptions, Result,
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
        .or_else(|| std::env::var_os("CRABDB_WORKSPACE").map(PathBuf::from));
    let db_dir = cli
        .db
        .clone()
        .or_else(|| std::env::var_os("CRABDB_DIR").map(PathBuf::from));
    let branch = cli
        .branch
        .clone()
        .or_else(|| std::env::var("CRABDB_BRANCH").ok());
    let daemon_url = cli
        .daemon_url
        .clone()
        .or_else(|| std::env::var("CRABDB_DAEMON_URL").ok())
        .filter(|value| !value.trim().is_empty());
    let daemon_token = cli
        .daemon_token
        .clone()
        .or_else(|| std::env::var("CRABDB_DAEMON_TOKEN").ok())
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
            let report = CrabDb::init_with_text_policy_and_prolly_backend(
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
        Command::Acp(acp_command) => acp::handle_acp_command(&ctx, acp_command),
        Command::Agent(agent_command) => agent::handle_agent_command(&ctx, agent_command),
        Command::Transcript(args) => acp::handle_transcript_command(&ctx, args),
        Command::Turn(turn) => acp::handle_top_turn_command(&ctx, turn),
        Command::Demo(demo) => acp::handle_demo_command(&ctx, demo),
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
