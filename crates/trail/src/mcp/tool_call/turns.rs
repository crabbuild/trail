use serde_json::Value;

use crate::model::validate_external_patch_edit_sources;
use crate::{Error, PatchDocument, PatchEdit, Result, Trail};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut Trail, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "trail.begin_turn" => {
            let args: BeginTurnArgs = parse_args(arguments)?;
            tool_result(db.begin_lane_turn(
                &args.lane,
                args.branch.as_deref(),
                args.session_title,
                args.base_change.as_deref(),
            )?)
        }
        "trail.add_message" => {
            let args: AddMessageArgs = parse_args(arguments)?;
            let text = args.content.or(args.text).ok_or_else(|| {
                Error::InvalidInput("add_message requires `content` or `text`".to_string())
            })?;
            tool_result(db.add_lane_turn_message(&args.turn_id, &args.role, &text)?)
        }
        "trail.add_event" => {
            let args: AddEventArgs = parse_args(arguments)?;
            tool_result(db.add_lane_turn_event(
                &args.turn_id,
                &args.event_type,
                args.payload,
                args.change_id.as_deref(),
                args.message_id.as_deref(),
            )?)
        }
        "trail.event_list" => {
            let args: EventListArgs = parse_args(arguments)?;
            tool_result(db.list_lane_events(
                args.lane.as_deref(),
                args.session.as_deref(),
                args.turn_id.as_deref(),
                args.event_type.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "trail.span_start" => {
            let args: SpanStartArgs = parse_args(arguments)?;
            tool_result(db.start_lane_trace_span(
                &args.turn_id,
                &args.span_type,
                &args.name,
                args.parent.as_deref(),
                args.trace.as_deref(),
                args.attributes,
            )?)
        }
        "trail.span_end" => {
            let args: SpanEndArgs = parse_args(arguments)?;
            tool_result(db.end_lane_trace_span(&args.span_id, &args.status, args.result)?)
        }
        "trail.span_list" => {
            let args: SpanListArgs = parse_args(arguments)?;
            tool_result(db.list_lane_trace_spans(
                args.lane.as_deref(),
                args.session.as_deref(),
                args.turn_id.as_deref(),
                args.trace_id.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "trail.span_summary" => {
            let args: SpanSummaryArgs = parse_args(arguments)?;
            tool_result(db.summarize_lane_trace_spans(
                args.lane.as_deref(),
                args.session.as_deref(),
                args.turn_id.as_deref(),
                args.trace_id.as_deref(),
                args.slowest.unwrap_or(5),
            )?)
        }
        "trail.span_show" => {
            let args: SpanShowArgs = parse_args(arguments)?;
            tool_result(db.show_lane_trace_span(&args.span_id)?)
        }
        "trail.apply_patch" => {
            let args: ApplyPatchArgs = parse_args(arguments)?;
            let turn_id = args.turn_id.clone();
            let patch = patch_document_from_args(args)?;
            tool_result(db.apply_lane_turn_patch(&turn_id, patch)?)
        }
        "trail.end_turn" => {
            let args: EndTurnArgs = parse_args(arguments)?;
            tool_result(db.end_lane_turn(&args.turn_id, &args.status)?)
        }
        "trail.show_turn" => {
            let args: TurnIdArgs = parse_args(arguments)?;
            tool_result(db.show_lane_turn(&args.turn_id)?)
        }
        _ => return Ok(None),
    };
    Ok(Some(value?))
}

fn patch_document_from_args(args: ApplyPatchArgs) -> Result<PatchDocument> {
    validate_external_patch_edit_sources("patch request", args.edits.len(), args.files.len())?;
    let mut edits = args.edits;
    for file in args.files {
        match file {
            ApiPatchFile::AddText {
                path,
                content,
                executable,
            } => edits.push(PatchEdit::Write {
                path,
                content,
                executable,
            }),
            ApiPatchFile::ModifyText {
                path,
                edits: file_edits,
            } => {
                for edit in file_edits {
                    match edit {
                        ApiTextEdit::ModifyLine {
                            line_id,
                            expected_text,
                            new_text,
                        } => edits.push(PatchEdit::ReplaceLine {
                            path: path.clone(),
                            line_id,
                            expected_text,
                            new_text,
                        }),
                    }
                }
            }
            ApiPatchFile::WriteBytes {
                path,
                bytes_hex,
                executable,
            } => edits.push(PatchEdit::WriteBytes {
                path,
                bytes_hex,
                executable,
            }),
            ApiPatchFile::Delete { path } => edits.push(PatchEdit::Delete { path }),
            ApiPatchFile::Rename { from, to } => edits.push(PatchEdit::Rename { from, to }),
        }
    }
    Ok(PatchDocument {
        base_change: args.base_change,
        message: args.message,
        session_id: args.session_id,
        allow_ignored: args.allow_ignored,
        allow_stale: args.allow_stale,
        edits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_patch_args_reject_empty_or_ambiguous_edit_sources() {
        let empty: ApplyPatchArgs = serde_json::from_value(serde_json::json!({
            "turn_id": "turn_1",
            "message": "empty"
        }))
        .unwrap();
        let empty_err = patch_document_from_args(empty).unwrap_err();
        assert!(empty_err
            .to_string()
            .contains("requires at least one edit in `edits` or `files`"));

        let ambiguous: ApplyPatchArgs = serde_json::from_value(serde_json::json!({
            "turn_id": "turn_1",
            "message": "ambiguous",
            "edits": [{"op": "delete", "path": "old.md"}],
            "files": [{"type": "delete", "path": "new.md"}]
        }))
        .unwrap();
        let ambiguous_err = patch_document_from_args(ambiguous).unwrap_err();
        assert!(ambiguous_err
            .to_string()
            .contains("must use either `edits` or `files`, not both"));
    }

    #[test]
    fn apply_patch_args_convert_files_shape_to_native_edits() {
        let args: ApplyPatchArgs = serde_json::from_value(serde_json::json!({
            "turn_id": "turn_1",
            "message": "files shape",
            "files": [{
                "type": "modify_text",
                "path": "README.md",
                "edits": [{
                    "type": "modify_line",
                    "line_id": "ch_seed:1",
                    "expected_text": "old",
                    "new_text": "new"
                }]
            }]
        }))
        .unwrap();

        let patch = patch_document_from_args(args).unwrap();
        assert_eq!(patch.edits.len(), 1);
        match &patch.edits[0] {
            PatchEdit::ReplaceLine {
                path,
                line_id,
                expected_text,
                new_text,
            } => {
                assert_eq!(path, "README.md");
                assert_eq!(line_id, "ch_seed:1");
                assert_eq!(expected_text.as_deref(), Some("old"));
                assert_eq!(new_text, "new");
            }
            other => panic!("expected replace_line edit, got {other:?}"),
        }
    }
}
