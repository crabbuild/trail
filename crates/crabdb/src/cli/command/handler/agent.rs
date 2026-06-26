use super::*;

mod runs;
mod traces;
mod turns;
mod work;

pub(super) fn handle_agent_command(ctx: &RuntimeContext, agent: AgentCommand) -> Result<()> {
    match agent.command {
        AgentSubcommand::Spawn(args) => {
            let mut db = open_db(ctx)?;
            let materialize = if args.no_materialize {
                false
            } else if args.workdir.is_some() || !args.paths.is_empty() {
                args.materialize.unwrap_or(true)
            } else {
                args.materialize
                    .unwrap_or(db.default_agent_materialize_for_ref(args.from.as_deref())?)
            };
            let report = db.spawn_agent_with_workdir_paths_and_neighbors(
                &args.name,
                args.from.as_deref(),
                materialize,
                args.provider,
                args.model,
                args.workdir,
                &args.paths,
                args.include_neighbors,
            )?;
            render_agent_spawn(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::List => {
            let db = open_db(ctx)?;
            let agents = db.list_agents()?;
            render_agent_list(&agents, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let details = db.agent_details(&args.name)?;
            render_agent_details(&details, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Status(args) => {
            let db = open_db(ctx)?;
            let report = db.agent_status(&args.name)?;
            render_agent_status(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Review(args) => {
            let db = open_db(ctx)?;
            let report = db.agent_review_packet(&args.name, args.limit)?;
            render_agent_review_packet(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Contribution(args) => {
            let db = open_db(ctx)?;
            let report = db.agent_contribution(&args.name, args.limit)?;
            render_agent_contribution(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Gates(args) => {
            let db = open_db(ctx)?;
            let report = db.agent_gate_history(&args.name, args.kind.as_deref(), args.limit)?;
            render_agent_gate_history(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Readiness(args) => {
            let db = open_db(ctx)?;
            let report = db.agent_readiness(&args.name)?;
            render_agent_readiness(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Handoff(args) => {
            let db = open_db(ctx)?;
            let report = db.agent_handoff(&args.name, args.limit)?;
            render_agent_handoff(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Claim(args) => {
            let mut db = open_db(ctx)?;
            let report = db.claim_agent_path(&args.name, &args.path, args.ttl_secs)?;
            render_agent_claim(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Message(args) => {
            let mut db = open_db(ctx)?;
            let report = db.add_agent_message(&args.name, &args.role, &args.text, args.session)?;
            render_agent_message(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Turn(turn) => turns::handle_turn_command(ctx, turn),
        AgentSubcommand::Run(run) => runs::handle_run_command(ctx, run),
        AgentSubcommand::Events(args) => {
            let db = open_db(ctx)?;
            let events = db.list_agent_events(
                args.agent.as_deref(),
                args.session.as_deref(),
                args.turn.as_deref(),
                args.event_type.as_deref(),
                args.limit,
            )?;
            render_agent_events(&events, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Trace(trace) => traces::handle_trace_command(ctx, trace),
        AgentSubcommand::Record(args) => {
            let mut db = open_db(ctx)?;
            let report = db.record_agent_workdir(&args.name, args.message)?;
            render_agent_record(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Rewind(args) => {
            let mut db = open_db(ctx)?;
            let report = db.rewind_agent(
                &args.name,
                &args.target,
                args.record_current,
                args.sync_workdir,
            )?;
            render_agent_rewind(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Watch(args) => work::handle_watch_command(ctx, args),
        AgentSubcommand::Test(args) => {
            work::handle_gate_command(ctx, args, work::AgentGateKind::Test)
        }
        AgentSubcommand::Eval(args) => {
            work::handle_gate_command(ctx, args, work::AgentGateKind::Eval)
        }
        AgentSubcommand::Read(args) => {
            let mut db = open_db(ctx)?;
            let hydrate = if args.hydrate {
                Some(true)
            } else if args.no_hydrate {
                Some(false)
            } else {
                None
            };
            let report = db.read_agent_file_with_hydration(
                &args.name,
                &args.path,
                hydrate,
                args.force,
                args.include_neighbors,
            )?;
            render_agent_file_read(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Workdir(args) => {
            let db = open_db(ctx)?;
            let report = db.agent_workdir(&args.name)?;
            render_agent_workdir(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::SyncWorkdir(args) => {
            let mut db = open_db(ctx)?;
            let report = db.sync_agent_workdir_with_paths_and_neighbors(
                &args.name,
                args.force,
                &args.paths,
                args.include_neighbors,
            )?;
            render_agent_workdir_sync(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::ApplyPatch(args) => {
            let mut db = open_db(ctx)?;
            let mut patch: PatchDocument =
                serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
            if args.allow_ignored {
                patch.allow_ignored = true;
            }
            let report = db.apply_agent_patch(&args.name, patch)?;
            render_agent_patch(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Diff(args) => {
            let db = open_db(ctx)?;
            let summary = db.diff_agent_with_options(&args.name, args.patch, args.show_line_ids)?;
            render_diff(&summary, ctx.json, ctx.quiet, false)
        }
        AgentSubcommand::Timeline(args) => {
            let db = open_db(ctx)?;
            let entries = db.agent_timeline(&args.name, args.limit)?;
            render_timeline(&entries, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Checkout(args) => {
            let mut db = open_db(ctx)?;
            let report = db.checkout_agent_with_options(
                &args.name,
                args.force,
                args.dry_run,
                args.workdir.as_deref(),
            )?;
            render_checkout(&report, ctx.json, ctx.quiet)
        }
        AgentSubcommand::Rm(args) => {
            let mut db = open_db(ctx)?;
            let report = db.remove_agent(&args.name, args.force)?;
            render_agent_remove(&report, ctx.json, ctx.quiet)
        }
    }
}
