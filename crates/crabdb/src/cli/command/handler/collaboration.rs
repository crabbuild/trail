use super::*;

pub(super) fn handle_merge_lane_command(ctx: &RuntimeContext, args: MergeLaneArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    validate_merge_strategy(args.strategy.as_deref())?;
    let report = db.merge_lane_with_options(&args.name, &args.into, args.dry_run)?;
    render_merge(&report, ctx.json, ctx.quiet)
}

pub(super) fn handle_merge_queue_command(
    ctx: &RuntimeContext,
    queue: MergeQueueCommand,
) -> Result<()> {
    match queue.command {
        MergeQueueSubcommand::Add(args) => {
            let mut db = open_db(ctx)?;
            let report = db.enqueue_merge(&args.source, &args.into, args.priority)?;
            render_merge_queue_add(&report, ctx.json, ctx.quiet)
        }
        MergeQueueSubcommand::List => {
            let db = open_db(ctx)?;
            let entries = db.list_merge_queue()?;
            render_merge_queue_list(&entries, ctx.json, ctx.quiet)
        }
        MergeQueueSubcommand::Explain(args) => {
            let mut db = open_db(ctx)?;
            let report = db.explain_merge_queue(&args.selector)?;
            render_merge_queue_explain(&report, ctx.json, ctx.quiet)
        }
        MergeQueueSubcommand::Run(args) => {
            let mut db = open_db(ctx)?;
            let report = db.run_merge_queue(args.limit)?;
            render_merge_queue_run(&report, ctx.json, ctx.quiet)
        }
        MergeQueueSubcommand::Remove(args) => {
            let mut db = open_db(ctx)?;
            let report = db.remove_merge_queue(&args.selector)?;
            render_merge_queue_remove(&report, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_conflicts_command(
    ctx: &RuntimeContext,
    conflicts: ConflictsCommand,
) -> Result<()> {
    match conflicts.command {
        ConflictsSubcommand::List => {
            let db = open_db(ctx)?;
            let conflicts = db.list_conflicts()?;
            render_conflicts(&conflicts, ctx.json, ctx.quiet)
        }
        ConflictsSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let conflict = db.show_conflict_with_limit(&args.conflict_set_id, args.limit)?;
            render_conflict(&conflict, ctx.json, ctx.quiet)
        }
        ConflictsSubcommand::Resolve(args) => {
            let mut db = open_db(ctx)?;
            let report = if let Some(manual_path) = args.manual {
                let manual = read_manual_conflict_resolution(&manual_path)?;
                db.resolve_conflict_manual(&args.conflict_set_id, manual)?
            } else if let Some(take) = args.take {
                db.resolve_conflict(&args.conflict_set_id, take.as_str())?
            } else {
                return Err(Error::InvalidInput(
                    "conflicts resolve requires `--take` or `--manual`".to_string(),
                ));
            };
            render_conflict_resolve(&report, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_anchor_command(ctx: &RuntimeContext, anchor: AnchorCommand) -> Result<()> {
    match anchor.command {
        AnchorSubcommand::Create(args) => {
            let mut db = open_db(ctx)?;
            let report = db.create_anchor(&args.path_line, args.label, ctx.branch.as_deref())?;
            render_anchor_create(&report, ctx.json, ctx.quiet)
        }
        AnchorSubcommand::Resolve(args) => {
            let db = open_db(ctx)?;
            let report = db.resolve_anchor(&args.anchor_id, ctx.branch.as_deref())?;
            render_anchor_resolve(&report, ctx.json, ctx.quiet)
        }
        AnchorSubcommand::List => {
            let db = open_db(ctx)?;
            let anchors = db.list_anchors()?;
            render_anchor_list(&anchors, ctx.json, ctx.quiet)
        }
        AnchorSubcommand::Delete(args) => {
            let mut db = open_db(ctx)?;
            let report = db.delete_anchor(&args.anchor_id)?;
            render_anchor_delete(&report, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_lease_command(ctx: &RuntimeContext, lease: LeaseCommand) -> Result<()> {
    match lease.command {
        LeaseSubcommand::Acquire(args) => {
            let mut db = open_db(ctx)?;
            let report = db.acquire_lease(
                &args.lane,
                Some(&args.path),
                args.mode.as_str(),
                args.ttl_secs,
            )?;
            render_lease_acquire(&report, ctx.json, ctx.quiet)
        }
        LeaseSubcommand::List(args) => {
            let db = open_db(ctx)?;
            let leases = db.list_leases(args.all)?;
            render_lease_list(&leases, ctx.json, ctx.quiet)
        }
        LeaseSubcommand::Release(args) => {
            let mut db = open_db(ctx)?;
            let report = db.release_lease(&args.lease_id)?;
            render_lease_release(&report, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_session_command(ctx: &RuntimeContext, session: SessionCommand) -> Result<()> {
    match session.command {
        SessionSubcommand::Start(args) => {
            let mut db = open_db(ctx)?;
            let report = db.start_lane_session(&args.lane, args.title, args.id)?;
            render_session_start(&report, ctx.json, ctx.quiet)
        }
        SessionSubcommand::Current(args) => {
            let db = open_db(ctx)?;
            let reports = db.current_lane_sessions(args.lane.as_deref())?;
            render_session_current(&reports, ctx.json, ctx.quiet)
        }
        SessionSubcommand::List(args) => {
            let db = open_db(ctx)?;
            let sessions = db.list_lane_sessions(args.lane.as_deref())?;
            render_session_list(&sessions, ctx.json, ctx.quiet)
        }
        SessionSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let details = db.show_lane_session(&args.session_id)?;
            render_session_details(&details, ctx.json, ctx.quiet)
        }
        SessionSubcommand::Context(args) => {
            let db = open_db(ctx)?;
            let report = db.lane_session_context(&args.session_id, args.limit)?;
            render_session_context(&report, ctx.json, ctx.quiet)
        }
        SessionSubcommand::End(args) => {
            let mut db = open_db(ctx)?;
            let report = db.end_lane_session(&args.session_id, &args.status)?;
            render_session_end(&report, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_approvals_command(
    ctx: &RuntimeContext,
    approvals: ApprovalsCommand,
) -> Result<()> {
    match approvals.command {
        ApprovalsSubcommand::Request(args) => {
            let mut db = open_db(ctx)?;
            let payload = args
                .payload_json
                .map(|payload| serde_json::from_str::<serde_json::Value>(&payload))
                .transpose()?;
            let report = db.request_lane_approval(
                &args.lane,
                &args.action,
                &args.summary,
                payload,
                args.session.as_deref(),
                args.turn.as_deref(),
            )?;
            render_approval_request(&report, ctx.json, ctx.quiet)
        }
        ApprovalsSubcommand::List(args) => {
            let db = open_db(ctx)?;
            let approvals = db.list_lane_approvals(args.lane.as_deref(), args.status.as_deref())?;
            render_approval_list(&approvals, ctx.json, ctx.quiet)
        }
        ApprovalsSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let approval = db.show_lane_approval(&args.approval_id)?;
            render_approval(&approval, ctx.json, ctx.quiet)
        }
        ApprovalsSubcommand::Decide(args) => {
            let mut db = open_db(ctx)?;
            let report = db.decide_lane_approval(
                &args.approval_id,
                args.decision.as_str(),
                args.reviewer,
                args.note,
            )?;
            render_approval_decision(&report, ctx.json, ctx.quiet)
        }
    }
}
