use serde_json::Value;

use crate::{CrabDb, Result};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut CrabDb, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "crabdb.session_start" => {
            let args: SessionStartArgs = parse_args(arguments)?;
            tool_result(db.start_agent_session(&args.agent, args.title, args.id)?)
        }
        "crabdb.session_list" => {
            let args: SessionListArgs = parse_args(arguments)?;
            tool_result(db.list_agent_sessions(args.agent.as_deref())?)
        }
        "crabdb.session_current" => {
            let args: SessionCurrentArgs = parse_args(arguments)?;
            tool_result(db.current_agent_sessions(args.agent.as_deref())?)
        }
        "crabdb.session_show" => {
            let args: SessionIdArgs = parse_args(arguments)?;
            tool_result(db.show_agent_session(&args.session_id)?)
        }
        "crabdb.session_context" => {
            let args: SessionContextArgs = parse_args(arguments)?;
            tool_result(db.agent_session_context(&args.session_id, args.limit.unwrap_or(50))?)
        }
        "crabdb.session_end" => {
            let args: SessionEndArgs = parse_args(arguments)?;
            tool_result(db.end_agent_session(&args.session_id, &args.status)?)
        }
        "crabdb.approval_request" => {
            let args: ApprovalRequestArgs = parse_args(arguments)?;
            tool_result(db.request_agent_approval(
                &args.agent,
                &args.action,
                &args.summary,
                args.payload,
                args.session_id.as_deref(),
                args.turn_id.as_deref(),
            )?)
        }
        "crabdb.approval_list" => {
            let args: ApprovalListArgs = parse_args(arguments)?;
            tool_result(db.list_agent_approvals(args.agent.as_deref(), args.status.as_deref())?)
        }
        "crabdb.approval_show" => {
            let args: ApprovalShowArgs = parse_args(arguments)?;
            tool_result(db.show_agent_approval(&args.approval_id)?)
        }
        "crabdb.approval_decide" => {
            let args: ApprovalDecideArgs = parse_args(arguments)?;
            tool_result(db.decide_agent_approval(
                &args.approval_id,
                &args.decision,
                args.reviewer,
                args.note,
            )?)
        }
        "crabdb.run_pause" => {
            let args: AgentRunPauseArgs = parse_args(arguments)?;
            tool_result(db.pause_agent_run(
                &args.agent,
                &args.reason,
                &args.summary,
                args.state,
                args.interruption,
                args.session_id.as_deref(),
                args.turn_id.as_deref(),
            )?)
        }
        "crabdb.run_list" => {
            let args: AgentRunListArgs = parse_args(arguments)?;
            tool_result(db.list_agent_run_states(args.agent.as_deref(), args.status.as_deref())?)
        }
        "crabdb.run_show" => {
            let args: AgentRunShowArgs = parse_args(arguments)?;
            tool_result(db.show_agent_run_state(&args.run_id)?)
        }
        "crabdb.run_resume" => {
            let args: AgentRunResumeArgs = parse_args(arguments)?;
            tool_result(db.resume_agent_run(&args.run_id, args.reviewer, args.note)?)
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
