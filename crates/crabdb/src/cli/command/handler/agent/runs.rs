use super::super::*;

pub(super) fn handle_run_command(ctx: &RuntimeContext, run: AgentRunCommand) -> Result<()> {
    match run.command {
        AgentRunSubcommand::Pause(args) => {
            let mut db = open_db(ctx)?;
            let state = parse_optional_json(args.state_json.as_deref())?;
            let interruption = parse_optional_json(args.interruption_json.as_deref())?;
            let report = db.pause_agent_run(
                &args.name,
                &args.reason,
                &args.summary,
                state,
                interruption,
                args.session.as_deref(),
                args.turn.as_deref(),
            )?;
            render_agent_run_pause(&report, ctx.json, ctx.quiet)
        }
        AgentRunSubcommand::List(args) => {
            let db = open_db(ctx)?;
            let run_states =
                db.list_agent_run_states(args.agent.as_deref(), args.status.as_deref())?;
            render_agent_run_list(&run_states, ctx.json, ctx.quiet)
        }
        AgentRunSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let run_state = db.show_agent_run_state(&args.run_id)?;
            render_agent_run_state(&run_state, ctx.json, ctx.quiet)
        }
        AgentRunSubcommand::Resume(args) => {
            let mut db = open_db(ctx)?;
            let report = db.resume_agent_run(&args.run_id, args.reviewer, args.note)?;
            render_agent_run_resume(&report, ctx.json, ctx.quiet)
        }
    }
}
