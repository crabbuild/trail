use super::*;

mod runs;
mod traces;
mod turns;
mod work;

pub(super) fn handle_lane_command(ctx: &RuntimeContext, lane: LaneCommand) -> Result<()> {
    match lane.command {
        LaneSubcommand::Spawn(args) => {
            let mut db = open_db(ctx)?;
            let materialize = if args.no_materialize {
                false
            } else if args.workdir.is_some() || !args.paths.is_empty() {
                args.materialize.unwrap_or(true)
            } else {
                args.materialize
                    .unwrap_or(db.default_lane_materialize_for_ref(args.from.as_deref())?)
            };
            let report = db.spawn_lane_with_workdir_paths_and_neighbors(
                &args.name,
                args.from.as_deref(),
                materialize,
                args.provider,
                args.model,
                args.workdir,
                &args.paths,
                args.include_neighbors,
            )?;
            render_lane_spawn(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::List => {
            let db = open_db(ctx)?;
            let lanes = db.list_lanes()?;
            render_lane_list(&lanes, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let details = db.lane_details(&args.name)?;
            render_lane_details(&details, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Status(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_status(&args.name)?;
            render_lane_status(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Review(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_review_packet(&args.name, args.limit)?;
            render_lane_review_packet(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Contribution(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_contribution(&args.name, args.limit)?;
            render_lane_contribution(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Gates(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_gate_history(&args.name, args.kind.as_deref(), args.limit)?;
            render_lane_gate_history(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Readiness(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_readiness(&args.name)?;
            render_lane_readiness(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::RefreshPreview(args) => {
            let db = open_db(ctx)?;
            let report = db.preview_lane_refresh(&args.name, &args.target)?;
            render_lane_refresh_preview(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Handoff(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_handoff(&args.name, args.limit)?;
            render_lane_handoff(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Claim(args) => {
            let mut db = open_db(ctx)?;
            let report = db.claim_lane_path(&args.name, &args.path, args.ttl_secs)?;
            render_lane_claim(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Message(args) => {
            let mut db = open_db(ctx)?;
            let report = db.add_lane_message(&args.name, &args.role, &args.text, args.session)?;
            render_lane_message(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Turn(turn) => turns::handle_turn_command(ctx, turn),
        LaneSubcommand::Run(run) => runs::handle_run_command(ctx, run),
        LaneSubcommand::Events(args) => {
            let db = open_db(ctx)?;
            let events = db.list_lane_events(
                args.lane.as_deref(),
                args.session.as_deref(),
                args.turn.as_deref(),
                args.event_type.as_deref(),
                args.limit,
            )?;
            render_lane_events(&events, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Trace(trace) => traces::handle_trace_command(ctx, trace),
        LaneSubcommand::Record(args) => {
            let mut db = open_db(ctx)?;
            if args.preview {
                let report = db.preview_lane_workdir_record(&args.name)?;
                render_lane_record_preview(&report, ctx.json, ctx.quiet)
            } else {
                let report = db.record_lane_workdir(&args.name, args.message)?;
                render_lane_record(&report, ctx.json, ctx.quiet)
            }
        }
        LaneSubcommand::Rewind(args) => {
            let mut db = open_db(ctx)?;
            let report = db.rewind_lane(
                &args.name,
                &args.target,
                args.record_current,
                args.sync_workdir,
            )?;
            render_lane_rewind(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Watch(args) => work::handle_watch_command(ctx, args),
        LaneSubcommand::Test(args) => {
            work::handle_gate_command(ctx, args, work::LaneGateKind::Test)
        }
        LaneSubcommand::Eval(args) => {
            work::handle_gate_command(ctx, args, work::LaneGateKind::Eval)
        }
        LaneSubcommand::Read(args) => {
            let mut db = open_db(ctx)?;
            let hydrate = if args.hydrate {
                Some(true)
            } else if args.no_hydrate {
                Some(false)
            } else {
                None
            };
            let report = db.read_lane_file_with_hydration(
                &args.name,
                &args.path,
                hydrate,
                args.force,
                args.include_neighbors,
            )?;
            render_lane_file_read(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Workdir(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_workdir(&args.name)?;
            render_lane_workdir(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::SyncWorkdir(args) => {
            let mut db = open_db(ctx)?;
            let report = db.sync_lane_workdir_with_paths_and_neighbors(
                &args.name,
                args.force,
                &args.paths,
                args.include_neighbors,
            )?;
            render_lane_workdir_sync(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::ApplyPatch(args) => {
            let mut db = open_db(ctx)?;
            let mut patch: PatchDocument =
                serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
            if args.allow_ignored {
                patch.allow_ignored = true;
            }
            if args.allow_stale {
                patch.allow_stale = true;
            }
            let report = db.apply_lane_patch(&args.name, patch)?;
            render_lane_patch(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Diff(args) => {
            let db = open_db(ctx)?;
            let summary = db.diff_lane_with_options(&args.name, args.patch, args.show_line_ids)?;
            render_diff(&summary, ctx.json, ctx.quiet, false)
        }
        LaneSubcommand::Timeline(args) => {
            let db = open_db(ctx)?;
            let entries = db.lane_timeline(&args.name, args.limit)?;
            render_timeline(&entries, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Checkout(args) => {
            let mut db = open_db(ctx)?;
            let report = db.checkout_lane_with_options(
                &args.name,
                args.force,
                args.dry_run,
                args.workdir.as_deref(),
            )?;
            render_checkout(&report, ctx.json, ctx.quiet)
        }
        LaneSubcommand::Rm(args) => {
            let mut db = open_db(ctx)?;
            let report = db.remove_lane(&args.name, args.force)?;
            render_lane_remove(&report, ctx.json, ctx.quiet)
        }
    }
}
