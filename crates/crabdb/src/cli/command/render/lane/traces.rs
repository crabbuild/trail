use super::render_json;

use crabdb;
use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_lane_trace_span_start(
    report: &LaneTraceSpanStartReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Started span {} {} {}",
            report.span.span_id, report.span.span_type, report.span.name
        );
        println!("Trace: {}", report.span.trace_id);
    }
    Ok(())
}

pub(crate) fn render_lane_trace_span_end(
    report: &LaneTraceSpanEndReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Ended span {} {}", report.span.span_id, report.span.status);
        if let Some(duration_ms) = report.span.duration_ms {
            println!("Duration: {duration_ms} ms");
        }
    }
    Ok(())
}

pub(crate) fn render_lane_trace_spans(
    spans: &[LaneTraceSpan],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(spans);
    }
    if !quiet {
        for span in spans {
            let parent = span.parent_span_id.as_deref().unwrap_or("-");
            let turn = span.turn_id.as_deref().unwrap_or("-");
            let duration = span
                .duration_ms
                .map(|duration_ms| format!("{duration_ms}ms"))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "{} {} {} status={} trace={} parent={} turn={} duration={}",
                span.span_id,
                span.span_type,
                span.name,
                span.status,
                span.trace_id,
                parent,
                turn,
                duration
            );
        }
    }
    Ok(())
}

pub(crate) fn render_lane_trace_summary(
    report: &LaneTraceSummaryReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Trace summary: {} spans ({} open, {} ended, {} failed)",
            report.span_count,
            report.open_span_count,
            report.ended_span_count,
            report.failed_span_count
        );
        if let Some(trace_id) = &report.trace_id {
            println!("Trace: {trace_id}");
        }
        if let Some(lane_id) = &report.lane_id {
            println!("Lane: {lane_id}");
        }
        if let Some(turn_id) = &report.turn_id {
            println!("Turn: {turn_id}");
        }
        if report.total_duration_ms > 0 {
            let average = report
                .average_duration_ms
                .map(|duration| format!("{duration:.1}"))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "Duration: total={}ms max={}ms avg={}ms",
                report.total_duration_ms, report.max_duration_ms, average
            );
        }
        println!("Statuses: {}", render_named_counts(&report.status_counts));
        println!("Types: {}", render_named_counts(&report.span_type_counts));
        println!("Traces: {}", render_named_counts(&report.trace_counts));
        if !report.slowest_spans.is_empty() {
            println!("Slowest spans:");
            for span in &report.slowest_spans {
                println!(
                    "  {} {} {} {}ms",
                    span.span_id,
                    span.span_type,
                    span.status,
                    span.duration_ms.unwrap_or(0)
                );
            }
        }
        if !report.open_spans.is_empty() {
            println!("Open spans:");
            for span in &report.open_spans {
                println!("  {} {} {}", span.span_id, span.span_type, span.name);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_named_counts(counts: &[crabdb::model::NamedCount]) -> String {
    if counts.is_empty() {
        return "-".to_string();
    }
    counts
        .iter()
        .map(|count| format!("{}={}", count.name, count.count))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn render_lane_trace_span(span: &LaneTraceSpan, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(span);
    }
    if !quiet {
        println!("Span: {}", span.span_id);
        println!("Trace: {}", span.trace_id);
        println!("Type: {}", span.span_type);
        println!("Name: {}", span.name);
        println!("Status: {}", span.status);
        if let Some(parent) = &span.parent_span_id {
            println!("Parent: {parent}");
        }
        if let Some(turn) = &span.turn_id {
            println!("Turn: {turn}");
        }
        if let Some(duration_ms) = span.duration_ms {
            println!("Duration: {duration_ms} ms");
        }
    }
    Ok(())
}
