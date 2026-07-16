use super::super::*;

pub(super) fn handle_run_command(ctx: &RuntimeContext, run: LaneRunCommand) -> Result<()> {
    match run.command {
        LaneRunSubcommand::Pause(args) => {
            let mut db = open_db(ctx)?;
            let state = parse_optional_json(args.state_json.as_deref())?;
            let interruption = parse_optional_json(args.interruption_json.as_deref())?;
            let report = db.pause_lane_run(
                &args.name,
                &args.reason,
                &args.summary,
                state,
                interruption,
                args.session.as_deref(),
                args.turn.as_deref(),
            )?;
            render_lane_run_pause(&report, ctx.json, &ctx.render)
        }
        LaneRunSubcommand::List(args) => {
            let db = open_db(ctx)?;
            let run_states =
                db.list_lane_run_states(args.lane.as_deref(), args.status.as_deref())?;
            render_lane_run_list(&run_states, ctx.json, &ctx.render)
        }
        LaneRunSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let run_state = db.show_lane_run_state(&args.run_id)?;
            render_lane_run_state(&run_state, ctx.json, &ctx.render)
        }
        LaneRunSubcommand::Resume(args) => {
            let mut db = open_db(ctx)?;
            let report = db.resume_lane_run(&args.run_id, args.reviewer, args.note)?;
            render_lane_run_resume(&report, ctx.json, &ctx.render)
        }
    }
}
