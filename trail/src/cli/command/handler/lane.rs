use super::*;

mod runs;
mod traces;
mod turns;
mod work;

pub(super) fn handle_lane_command(ctx: &RuntimeContext, lane: LaneCommand) -> Result<()> {
    match lane.command {
        LaneSubcommand::Spawn(args) => {
            let materialization_workspace = (!args.no_materialize
                && args.materialize != Some(false))
            .then(|| daemon_start::workspace_from_context(ctx))
            .transpose()?;
            // A materialized lane changes the workspace generation. Retire a
            // verified workspace observer before the first local schema
            // handoff, rather than after a long-lived observer has kept the
            // SQLite WAL moving throughout a concurrent spawn burst.
            if let Some(workspace) = materialization_workspace.as_deref() {
                daemon_start::retire_workspace_daemon_after_external_generation_change(workspace)?;
            }
            // Serialize only the mutable database-open handoff. Dropping the
            // guard before lane initialization preserves the separate 16-way
            // native-COW admission for the expensive filesystem work.
            let mut db = {
                let _open_admission = materialization_workspace
                    .as_deref()
                    .map(trail::LaneSpawnMaterializationAdmission::acquire_for_workspace)
                    .transpose()?;
                open_db(ctx)?
            };
            let workdir_mode = db.resolve_lane_spawn_workdir_mode(
                args.from.as_deref(),
                args.workdir_mode.as_deref(),
                args.materialize,
                args.no_materialize,
                args.workdir.is_some(),
                &args.paths,
            )?;
            let report = db.spawn_lane_with_deferred_initial_ledger(
                &args.name,
                args.from.as_deref(),
                workdir_mode,
                args.provider,
                args.model,
                args.workdir,
                &args.paths,
                args.include_neighbors,
            )?;
            let report = if report.workdir.is_some() && !report.workdir_mode.is_transparent_cow() {
                drop(db);
                let mut reopened = {
                    let _open_admission = materialization_workspace
                        .as_deref()
                        .map(trail::LaneSpawnMaterializationAdmission::acquire_for_workspace)
                        .transpose()?;
                    open_db(ctx)?
                };
                let report = reopened.resume_deferred_initial_lane_ledger(&args.name)?;
                let workspace = reopened.workspace_root().to_path_buf();
                drop(reopened);
                daemon_start::retire_workspace_daemon_after_external_generation_change(&workspace)?;
                report
            } else {
                report
            };
            render_lane_spawn(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::RepairInitialization(args) => {
            let mut db = open_db(ctx)?;
            let report = db.repair_lane_initialization(&args.name)?;
            render_lane_spawn(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::List => {
            let db = open_db(ctx)?;
            let lanes = db.list_lanes()?;
            render_lane_list(&lanes, ctx.json, &ctx.render)
        }
        LaneSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let details = db.lane_details(&args.name)?;
            render_lane_details(&details, ctx.json, &ctx.render)
        }
        LaneSubcommand::Status(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_status(&args.name)?;
            render_lane_status(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Review(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_review_packet(&args.name, args.limit)?;
            render_lane_review_packet(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Contribution(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_contribution(&args.name, args.limit)?;
            render_lane_contribution(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Gates(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_gate_history(&args.name, args.kind.as_deref(), args.limit)?;
            render_lane_gate_history(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Readiness(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_readiness(&args.name)?;
            render_lane_readiness(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Merge(args) => {
            let mut db = open_db(ctx)?;
            validate_merge_strategy(args.strategy.as_deref())?;
            let report =
                db.merge_lane_user_with_options(&args.name, &args.into, args.dry_run, args.direct)?;
            render_merge(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::MergeQueue(queue) => {
            collaboration::handle_lane_merge_queue_command(ctx, queue)
        }
        LaneSubcommand::RefreshPreview(args) => {
            let db = open_db(ctx)?;
            let report = db.preview_lane_refresh(&args.name, &args.target)?;
            render_lane_refresh_preview(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Update(args) => {
            let mut db = open_db(ctx)?;
            let report = db.update_layered_lane_from(&args.name, &args.source, args.checkpoint)?;
            render_merge(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Handoff(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_handoff(&args.name, args.limit)?;
            render_lane_handoff(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Claim(args) => {
            let mut db = open_db(ctx)?;
            let report = db.claim_lane_path(&args.name, &args.path, args.ttl_secs)?;
            render_lane_claim(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Message(args) => {
            let mut db = open_db(ctx)?;
            let report = db.add_lane_message(&args.name, &args.role, &args.text, args.session)?;
            render_lane_message(&report, ctx.json, &ctx.render)
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
            render_lane_events(&events, ctx.json, &ctx.render)
        }
        LaneSubcommand::Trace(trace) => traces::handle_trace_command(ctx, trace),
        LaneSubcommand::Record(args) => {
            let mut db = open_db(ctx)?;
            if args.preview {
                let report = db.preview_lane_workdir_record(&args.name)?;
                render_lane_record_preview(&report, ctx.json, &ctx.render)
            } else {
                let report = db.record_lane_workdir(&args.name, args.message)?;
                render_lane_record(&report, ctx.json, &ctx.render)
            }
        }
        LaneSubcommand::Checkpoint(args) => {
            let mut db = open_db(ctx)?;
            let report = db.checkpoint_lane_workspace(&args.name, args.message)?;
            render_workspace_checkpoint(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Space(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_workspace_space(&args.name)?;
            render_workspace_space(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Exec(args) => {
            let db = open_db(ctx)?;
            let report = db.exec_lane_workspace(&args.name, &args.command)?;
            render_workspace_exec(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Mount(args) => {
            let db = open_db(ctx)?;
            let _foreground = args.foreground;
            let report = db.mount_lane_workspace_until_requested(&args.name)?;
            render_workspace_mount(&report, "mounted", ctx.json, &ctx.render)
        }
        LaneSubcommand::Unmount(args) => {
            let db = open_db(ctx)?;
            let report = db.request_lane_workspace_unmount(&args.name)?;
            render_workspace_mount(&report, "unmounted", ctx.json, &ctx.render)
        }
        LaneSubcommand::Rewind(args) => {
            let mut db = open_db(ctx)?;
            let report = db.rewind_lane(
                &args.name,
                &args.target,
                args.record_current,
                args.sync_workdir,
            )?;
            render_lane_rewind(&report, ctx.json, &ctx.render)
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
            render_lane_file_read(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Hydrate(args) => {
            let mut db = open_db(ctx)?;
            let report = db.sync_lane_workdir_with_paths_and_neighbors(
                &args.name,
                args.force,
                &args.paths,
                args.include_neighbors,
            )?;
            render_lane_workdir_sync(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Workdir(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_workdir(&args.name)?;
            render_lane_workdir(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::SyncWorkdir(args) => {
            let mut db = open_db(ctx)?;
            let report = db.sync_lane_workdir_with_paths_and_neighbors(
                &args.name,
                args.force,
                &args.paths,
                args.include_neighbors,
            )?;
            render_lane_workdir_sync(&report, ctx.json, &ctx.render)
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
            render_lane_patch(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Diff(args) => {
            let db = open_db(ctx)?;
            validate_diff_view(
                args.patch,
                args.stat,
                args.show_line_ids,
                args.name_only,
                args.name_status,
            )?;
            let summary = db.diff_lane_with_options(&args.name, args.patch, args.show_line_ids)?;
            let title = format!("Lane diff: {}", args.name);
            render_diff_with_title(
                &summary,
                ctx.json,
                &ctx.render,
                args.patch,
                args.stat,
                args.name_only,
                args.name_status,
                Some(&title),
            )
        }
        LaneSubcommand::Timeline(args) => {
            let db = open_db(ctx)?;
            let entries = db.lane_timeline(&args.name, args.limit)?;
            render_timeline(&entries, ctx.json, &ctx.render)
        }
        LaneSubcommand::Checkout(args) => {
            let mut db = open_db(ctx)?;
            let report = db.checkout_lane_with_options(
                &args.name,
                args.force,
                args.dry_run,
                args.workdir.as_deref(),
            )?;
            render_checkout(&report, ctx.json, &ctx.render)
        }
        LaneSubcommand::Rm(args) => {
            let mut db = open_db(ctx)?;
            let report = db.remove_lane(&args.name, args.force)?;
            render_lane_remove(&report, ctx.json, &ctx.render)
        }
    }
}
