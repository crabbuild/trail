use super::super::*;
use std::thread;

pub(super) fn handle_watch_command(ctx: &RuntimeContext, args: LaneWatchArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let interval = watch_interval(args.interval_secs, args.debounce_ms)?;
    let _include_untracked = args.include_untracked;
    if args.once {
        let report = db.watch_lane_workdir(&args.name, args.message, interval, Some(1))?;
        render_lane_watch(&report, ctx.json, ctx.quiet)
    } else {
        loop {
            let report = db.record_lane_workdir(&args.name, args.message.clone())?;
            if matches!(ctx.format, OutputFormat::Ndjson) {
                println!("{}", serde_json::to_string(&report)?);
            } else if report.operation.is_some() {
                render_lane_record(&report, ctx.json, ctx.quiet)?;
            }
            thread::sleep(interval);
        }
    }
}

pub(super) enum LaneGateKind {
    Test,
    Eval,
}

pub(super) fn handle_gate_command(
    ctx: &RuntimeContext,
    args: LaneTestArgs,
    kind: LaneGateKind,
) -> Result<()> {
    let mut db = open_db(ctx)?;
    let options = LaneGateOptions {
        suite: args.suite,
        score: args.score,
        threshold: args.threshold,
    };
    let report = match kind {
        LaneGateKind::Test => db.run_lane_test_with_options(
            &args.name,
            args.command,
            args.turn.as_deref(),
            args.timeout_secs,
            options,
        )?,
        LaneGateKind::Eval => db.run_lane_eval_with_options(
            &args.name,
            args.command,
            args.turn.as_deref(),
            args.timeout_secs,
            options,
        )?,
    };
    let render_result = render_lane_test(&report, ctx.json, ctx.quiet);
    if render_result.is_ok() && !report.success {
        std::process::exit(command_failure_exit_code(report.exit_code));
    }
    render_result
}
