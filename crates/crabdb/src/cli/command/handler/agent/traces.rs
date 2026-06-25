use super::super::*;

pub(super) fn handle_trace_command(ctx: &RuntimeContext, trace: AgentTraceCommand) -> Result<()> {
    match trace.command {
        AgentTraceSubcommand::Start(args) => {
            let mut db = open_db(ctx)?;
            let attributes = parse_optional_json(args.attributes_json.as_deref())?;
            let report = db.start_agent_trace_span(
                &args.turn_id,
                &args.span_type,
                &args.name,
                args.parent.as_deref(),
                args.trace_id.as_deref(),
                attributes,
            )?;
            render_agent_trace_span_start(&report, ctx.json, ctx.quiet)
        }
        AgentTraceSubcommand::End(args) => {
            let mut db = open_db(ctx)?;
            let result = parse_optional_json(args.result_json.as_deref())?;
            let report = db.end_agent_trace_span(&args.span_id, &args.status, result)?;
            render_agent_trace_span_end(&report, ctx.json, ctx.quiet)
        }
        AgentTraceSubcommand::List(args) => {
            let db = open_db(ctx)?;
            let spans = db.list_agent_trace_spans(
                args.agent.as_deref(),
                args.session.as_deref(),
                args.turn.as_deref(),
                args.trace_id.as_deref(),
                args.limit,
            )?;
            render_agent_trace_spans(&spans, ctx.json, ctx.quiet)
        }
        AgentTraceSubcommand::Summary(args) => {
            let db = open_db(ctx)?;
            let report = db.summarize_agent_trace_spans(
                args.agent.as_deref(),
                args.session.as_deref(),
                args.turn.as_deref(),
                args.trace_id.as_deref(),
                args.slowest_limit,
            )?;
            render_agent_trace_summary(&report, ctx.json, ctx.quiet)
        }
        AgentTraceSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let span = db.show_agent_trace_span(&args.span_id)?;
            render_agent_trace_span(&span, ctx.json, ctx.quiet)
        }
    }
}
