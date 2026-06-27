use serde_json::Value;

use crate::{CrabDb, Result};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut CrabDb, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "crabdb.session_start" => {
            let args: SessionStartArgs = parse_args(arguments)?;
            tool_result(db.start_lane_session(&args.lane, args.title, args.id)?)
        }
        "crabdb.session_list" => {
            let args: SessionListArgs = parse_args(arguments)?;
            tool_result(db.list_lane_sessions(args.lane.as_deref())?)
        }
        "crabdb.session_current" => {
            let args: SessionCurrentArgs = parse_args(arguments)?;
            tool_result(db.current_lane_sessions(args.lane.as_deref())?)
        }
        "crabdb.session_show" => {
            let args: SessionIdArgs = parse_args(arguments)?;
            tool_result(db.show_lane_session(&args.session_id)?)
        }
        "crabdb.session_context" => {
            let args: SessionContextArgs = parse_args(arguments)?;
            tool_result(db.lane_session_context(&args.session_id, args.limit.unwrap_or(50))?)
        }
        "crabdb.session_end" => {
            let args: SessionEndArgs = parse_args(arguments)?;
            tool_result(db.end_lane_session(&args.session_id, &args.status)?)
        }
        "crabdb.approval_request" => {
            let args: ApprovalRequestArgs = parse_args(arguments)?;
            tool_result(db.request_lane_approval(
                &args.lane,
                &args.action,
                &args.summary,
                args.payload,
                args.session_id.as_deref(),
                args.turn_id.as_deref(),
            )?)
        }
        "crabdb.approval_list" => {
            let args: ApprovalListArgs = parse_args(arguments)?;
            tool_result(db.list_lane_approvals(args.lane.as_deref(), args.status.as_deref())?)
        }
        "crabdb.approval_show" => {
            let args: ApprovalShowArgs = parse_args(arguments)?;
            tool_result(db.show_lane_approval(&args.approval_id)?)
        }
        "crabdb.approval_decide" => {
            let args: ApprovalDecideArgs = parse_args(arguments)?;
            tool_result(db.decide_lane_approval(
                &args.approval_id,
                &args.decision,
                args.reviewer,
                args.note,
            )?)
        }
        "crabdb.run_pause" => {
            let args: LaneRunPauseArgs = parse_args(arguments)?;
            tool_result(db.pause_lane_run(
                &args.lane,
                &args.reason,
                &args.summary,
                args.state,
                args.interruption,
                args.session_id.as_deref(),
                args.turn_id.as_deref(),
            )?)
        }
        "crabdb.run_list" => {
            let args: LaneRunListArgs = parse_args(arguments)?;
            tool_result(db.list_lane_run_states(args.lane.as_deref(), args.status.as_deref())?)
        }
        "crabdb.run_show" => {
            let args: LaneRunShowArgs = parse_args(arguments)?;
            tool_result(db.show_lane_run_state(&args.run_id)?)
        }
        "crabdb.run_resume" => {
            let args: LaneRunResumeArgs = parse_args(arguments)?;
            tool_result(db.resume_lane_run(&args.run_id, args.reviewer, args.note)?)
        }
        "crabdb.anchor_create" => {
            let args: AnchorCreateArgs = parse_args(arguments)?;
            tool_result(db.create_anchor(&args.path_line, args.label, args.branch.as_deref())?)
        }
        "crabdb.anchor_list" => tool_result(db.list_anchors()?),
        "crabdb.anchor_resolve" => {
            let args: AnchorIdArgs = parse_args(arguments)?;
            tool_result(db.resolve_anchor(&args.anchor_id, args.branch.as_deref())?)
        }
        "crabdb.anchor_delete" => {
            let args: AnchorIdArgs = parse_args(arguments)?;
            tool_result(db.delete_anchor(&args.anchor_id)?)
        }
        _ => return Ok(None),
    };
    Ok(Some(value?))
}
