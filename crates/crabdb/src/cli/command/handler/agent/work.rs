use super::super::*;
use std::thread;

pub(super) fn handle_watch_command(ctx: &RuntimeContext, args: AgentWatchArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let interval = watch_interval(args.interval_secs, args.debounce_ms)?;
    let _include_untracked = args.include_untracked;
    if args.once {
        let report = db.watch_agent_workdir(&args.name, args.message, interval, Some(1))?;
        render_agent_watch(&report, ctx.json, ctx.quiet)
    } else {
        loop {
            let report = db.record_agent_workdir(&args.name, args.message.clone())?;
            if matches!(ctx.format, OutputFormat::Ndjson) {
                println!("{}", serde_json::to_string(&report)?);
            } else if report.operation.is_some() {
                render_agent_record(&report, ctx.json, ctx.quiet)?;
            }
            thread::sleep(interval);
        }
    }
}

pub(super) enum AgentGateKind {
    Test,
    Eval,
}

pub(super) fn handle_gate_command(
    ctx: &RuntimeContext,
    args: AgentTestArgs,
    kind: AgentGateKind,
) -> Result<()> {
    let mut db = open_db(ctx)?;
    let options = AgentGateOptions {
        suite: args.suite,
        score: args.score,
        threshold: args.threshold,
    };
    let report = match kind {
        AgentGateKind::Test => db.run_agent_test_with_options(
            &args.name,
            args.command,
            args.turn.as_deref(),
            args.timeout_secs,
            options,
        )?,
        AgentGateKind::Eval => db.run_agent_eval_with_options(
            &args.name,
            args.command,
            args.turn.as_deref(),
            args.timeout_secs,
            options,
        )?,
    };
    let render_result = render_agent_test(&report, ctx.json, ctx.quiet);
    if render_result.is_ok() && !report.success {
        std::process::exit(command_failure_exit_code(report.exit_code));
    }
    render_result
}
