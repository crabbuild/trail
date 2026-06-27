use super::super::*;

pub(super) fn handle_turn_command(ctx: &RuntimeContext, turn: LaneTurnCommand) -> Result<()> {
    match turn.command {
        LaneTurnSubcommand::Start(args) => {
            let mut db = open_db(ctx)?;
            let report = db.begin_lane_turn(
                &args.name,
                args.from.as_deref(),
                args.title,
                args.base_change.as_deref(),
            )?;
            render_lane_turn_start(&report, ctx.json, ctx.quiet)
        }
        LaneTurnSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let details = db.show_lane_turn(&args.turn_id)?;
            render_lane_turn_details(&details, ctx.json, ctx.quiet)
        }
        LaneTurnSubcommand::Message(args) => {
            let mut db = open_db(ctx)?;
            let report = db.add_lane_turn_message(&args.turn_id, &args.role, &args.text)?;
            render_lane_message(&report, ctx.json, ctx.quiet)
        }
        LaneTurnSubcommand::Event(args) => {
            let mut db = open_db(ctx)?;
            let payload = parse_optional_json(args.payload_json.as_deref())?;
            let report = db.add_lane_turn_event(
                &args.turn_id,
                &args.event_type,
                payload,
                args.change.as_deref(),
                args.message.as_deref(),
            )?;
            render_lane_turn_event(&report, ctx.json, ctx.quiet)
        }
        LaneTurnSubcommand::ApplyPatch(args) => {
            let mut db = open_db(ctx)?;
            let mut patch: PatchDocument =
                serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
            if args.allow_ignored {
                patch.allow_ignored = true;
            }
            let report = db.apply_lane_turn_patch(&args.turn_id, patch)?;
            render_lane_patch(&report, ctx.json, ctx.quiet)
        }
        LaneTurnSubcommand::End(args) => {
            let mut db = open_db(ctx)?;
            let report = db.end_lane_turn(&args.turn_id, &args.status)?;
            render_lane_turn_end(&report, ctx.json, ctx.quiet)
        }
    }
}
