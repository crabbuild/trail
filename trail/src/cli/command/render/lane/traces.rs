use crate::cli::command::render::*;

use trail;
use trail::model::*;
use trail::Result;

pub(crate) fn render_lane_trace_span_start(
    report: &LaneTraceSpanStartReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Started {} span", report.span.span_type),
            UiTone::Success,
        )
        .block(span_metadata(&report.span)),
        options,
    )
}

pub(crate) fn render_lane_trace_span_end(
    report: &LaneTraceSpanEndReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Ended span {}", report.span.span_id),
            UiTone::Success,
        )
        .block(span_metadata(&report.span)),
        options,
    )
}

pub(crate) fn render_lane_trace_spans(
    spans: &[LaneTraceSpan],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(spans);
    }
    if spans.is_empty() {
        return render_document(
            &TerminalDocument::new("No trace spans", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(format!("{} trace span(s)", spans.len()), UiTone::Neutral)
            .block(UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("STATUS", 0, 8),
                    UiColumn::left("TYPE", 0, 8),
                    UiColumn::left("NAME", 0, 14),
                    UiColumn::left("TIME", 1, 7),
                    UiColumn::left("TRACE", 2, 12),
                ],
                spans
                    .iter()
                    .map(|span| {
                        vec![
                            span.status.clone(),
                            span.span_type.clone(),
                            span.name.clone(),
                            duration(span.duration_ms),
                            span.trace_id.clone(),
                        ]
                    })
                    .collect(),
            )))
            .pager_eligible(),
        options,
    )
}

pub(crate) fn render_lane_trace_summary(
    report: &LaneTraceSummaryReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut metadata = vec![
        ("Spans".to_string(), report.span_count.to_string()),
        ("Open".to_string(), report.open_span_count.to_string()),
        ("Ended".to_string(), report.ended_span_count.to_string()),
        ("Failed".to_string(), report.failed_span_count.to_string()),
        (
            "Total duration".to_string(),
            duration(Some(report.total_duration_ms)),
        ),
        (
            "Max duration".to_string(),
            duration(Some(report.max_duration_ms)),
        ),
    ];
    if let Some(avg) = report.average_duration_ms {
        metadata.push(("Average duration".to_string(), format!("{avg:.1} ms")));
    }
    if let Some(trace) = &report.trace_id {
        metadata.push(("Trace".to_string(), trace.clone()));
    }
    if let Some(lane) = &report.lane_id {
        metadata.push(("Lane".to_string(), lane.clone()));
    }
    let mut document = TerminalDocument::new(
        "Trace summary",
        if report.failed_span_count > 0 {
            UiTone::Attention
        } else {
            UiTone::Success
        },
    )
    .block(UiBlock::Metadata(metadata));
    if !report.slowest_spans.is_empty() {
        document = document.block(UiBlock::section(
            "Slowest spans:",
            vec![UiBlock::Table(span_table(&report.slowest_spans))],
        ));
    }
    if !report.open_spans.is_empty() {
        document = document.block(UiBlock::section(
            "Open spans:",
            vec![UiBlock::Table(span_table(&report.open_spans))],
        ));
    }
    render_document(&document, options)
}

pub(crate) fn render_lane_trace_span(
    span: &LaneTraceSpan,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(span);
    }
    render_document(
        &TerminalDocument::new(format!("Span {}", span.name), UiTone::Neutral)
            .block(span_metadata(span)),
        options,
    )
}

fn span_metadata(span: &LaneTraceSpan) -> UiBlock {
    let mut metadata = vec![
        ("Span".to_string(), span.span_id.clone()),
        ("Trace".to_string(), span.trace_id.clone()),
        ("Type".to_string(), span.span_type.clone()),
        ("Status".to_string(), span.status.clone()),
        ("Duration".to_string(), duration(span.duration_ms)),
    ];
    if let Some(parent) = &span.parent_span_id {
        metadata.push(("Parent".to_string(), parent.clone()));
    }
    if let Some(turn) = &span.turn_id {
        metadata.push(("Turn".to_string(), turn.clone()));
    }
    UiBlock::Metadata(metadata)
}

fn span_table(spans: &[LaneTraceSpan]) -> UiTable {
    UiTable::new(
        vec![
            UiColumn::left("STATUS", 0, 8),
            UiColumn::left("TYPE", 0, 8),
            UiColumn::left("NAME", 0, 14),
            UiColumn::left("TIME", 1, 7),
        ],
        spans
            .iter()
            .map(|span| {
                vec![
                    span.status.clone(),
                    span.span_type.clone(),
                    span.name.clone(),
                    duration(span.duration_ms),
                ]
            })
            .collect(),
    )
}

fn duration(duration: Option<u64>) -> String {
    duration
        .map(|value| format!("{value} ms"))
        .unwrap_or_else(|| "—".to_string())
}
