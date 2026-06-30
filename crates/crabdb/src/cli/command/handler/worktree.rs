use super::*;
use std::thread;

pub(super) fn handle_status_command(ctx: &RuntimeContext, args: StatusArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let branch = args.branch.as_deref().or(ctx.branch.as_deref());
    let report = db.status(branch)?;
    render_status(&report, ctx.json, ctx.quiet)
}

pub(super) fn handle_record_command(ctx: &RuntimeContext, args: RecordArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let kind = args
        .kind
        .as_deref()
        .map(parse_record_kind_arg)
        .transpose()?;
    let report = db.record_with_options(
        ctx.branch.as_deref(),
        args.message,
        Actor::human(),
        RecordOptions {
            paths: args.paths,
            kind,
            session_id: args.session,
            allow_ignored: args.allow_ignored,
        },
    )?;
    render_record(&report, ctx.json, ctx.quiet)
}

pub(super) fn handle_watch_command(ctx: &RuntimeContext, args: WatchArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let interval = watch_interval(args.interval_secs, args.debounce_ms)?;
    let _include_untracked = args.include_untracked;
    loop {
        let report = db.record_with_options(
            ctx.branch.as_deref(),
            args.message.clone(),
            Actor::human(),
            RecordOptions {
                kind: Some(OperationKind::WatchRecord),
                session_id: args.session.clone(),
                ..RecordOptions::default()
            },
        )?;
        if matches!(ctx.format, OutputFormat::Ndjson) {
            println!("{}", serde_json::to_string(&report)?);
        } else if report.operation.is_some() {
            render_record(&report, ctx.json, ctx.quiet)?;
        }
        if args.once {
            break;
        }
        thread::sleep(interval);
    }
    Ok(())
}

pub(super) fn handle_diff_command(ctx: &RuntimeContext, args: DiffArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let summary = diff_from_args(&mut db, &args)?;
    render_diff(&summary, ctx.json, ctx.quiet, args.stat, ctx.color)
}

pub(super) fn handle_checkout_command(ctx: &RuntimeContext, args: CheckoutArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.checkout_with_options(
        &args.target,
        args.force,
        args.dry_run,
        args.workdir.as_deref(),
        args.record_dirty,
    )?;
    render_checkout(&report, ctx.json, ctx.quiet)
}

pub(super) fn handle_branch_command(ctx: &RuntimeContext, args: BranchArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    match (
        args.name.as_deref(),
        args.delete.as_deref(),
        args.rename.as_deref(),
        args.to.as_deref(),
    ) {
        (Some(name), None, None, None) => {
            let report = db.create_branch(name, args.from.as_deref())?;
            render_branch(&report, ctx.json, ctx.quiet)
        }
        (None, Some(name), None, None) => {
            let report = db.delete_branch(name)?;
            render_branch_delete(&report, ctx.json, ctx.quiet)
        }
        (None, None, Some(old_name), Some(new_name)) => {
            let report = db.rename_branch(old_name, new_name)?;
            render_branch_rename(&report, ctx.json, ctx.quiet)
        }
        (None, None, None, None) => {
            let entries = db.list_branches()?;
            render_branch_list(&entries, ctx.json, ctx.quiet)
        }
        _ => Err(Error::InvalidInput(
            "branch accepts either NAME [--from REF], --delete NAME, --rename OLD --to NEW, or no arguments".to_string(),
        )),
    }
}

pub(super) fn handle_merge_command(ctx: &RuntimeContext, args: MergeArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    validate_merge_strategy(args.strategy.as_deref())?;
    let report = db.merge_branches_with_options(&args.source, &args.into, args.dry_run)?;
    render_merge(&report, ctx.json, ctx.quiet)
}
