use serde_json::Value;

use crate::{CrabDb, Error, LaneGateOptions, Result};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut CrabDb, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "crabdb.doctor" => tool_result(db.doctor()?),
        "crabdb.status" => {
            let args: StatusArgs = parse_args(arguments)?;
            tool_result(db.status_read_only(args.branch.as_deref())?)
        }
        "crabdb.diff" => {
            let args: DiffArgs = parse_args(arguments)?;
            let forms = usize::from(args.range.is_some())
                + usize::from(args.root.is_some())
                + usize::from(args.dirty);
            if forms != 1 {
                return Err(Error::InvalidInput(
                    "diff requires exactly one of `range`, `root`, or `dirty`".to_string(),
                ));
            }
            let diff = if args.dirty {
                db.diff_dirty(args.patch, args.show_line_ids)?
            } else if let Some(root) = args.root {
                db.diff_roots(&root, args.patch, args.show_line_ids)?
            } else {
                db.diff_range_with_options(
                    args.range.as_deref().unwrap_or_default(),
                    args.patch,
                    args.show_line_ids,
                )?
            };
            tool_result(diff)
        }
        "crabdb.timeline" => {
            let args: TimelineArgs = parse_args(arguments)?;
            tool_result(db.timeline_query(
                args.branch.as_deref(),
                args.session.as_deref(),
                args.lane.as_deref(),
                args.limit.unwrap_or(30),
            )?)
        }
        "crabdb.why" => {
            let args: WhyArgs = parse_args(arguments)?;
            let at = args.at.as_deref().or(args.branch.as_deref());
            let result = match (args.path_line.as_deref(), args.line_id.as_deref()) {
                (Some(path_line), None) => db.why(path_line, at)?,
                (None, Some(line_id)) => db.why_line_id(line_id, at)?,
                (Some(_), Some(_)) => {
                    return Err(Error::InvalidInput(
                        "crabdb.why accepts either path_line or line_id, not both".to_string(),
                    ));
                }
                (None, None) => {
                    return Err(Error::InvalidInput(
                        "crabdb.why requires path_line or line_id".to_string(),
                    ));
                }
            };
            tool_result(result)
        }
        "crabdb.history" => {
            let args: HistoryArgs = parse_args(arguments)?;
            let result = if let Some(line_id) = args.line_id {
                db.history_for_line_id(&line_id)?
            } else if let Some(file_id) = args.file_id {
                db.history_for_file_id(&file_id)?
            } else {
                let selector = args.path.or(args.selector).ok_or_else(|| {
                    Error::InvalidInput(
                        "history requires `path`, `selector`, `file_id`, or `line_id`".to_string(),
                    )
                })?;
                db.history_for_path(&selector)?
            };
            tool_result(result)
        }
        "crabdb.code_from" => {
            let args: CodeFromArgs = parse_args(arguments)?;
            tool_result(db.code_from(&args.selector)?)
        }
        "crabdb.agent_status" => tool_result(db.agent_status()?),
        "crabdb.agent_inbox" => tool_result(db.agent_inbox()?),
        "crabdb.agent_board" => {
            let args: AgentBoardArgs = parse_args(arguments)?;
            tool_result(db.agent_board_with_options(args.all)?)
        }
        "crabdb.agent_stack" => {
            let args: AgentBoardArgs = parse_args(arguments)?;
            tool_result(db.agent_stack_with_options(args.all)?)
        }
        "crabdb.agent_next" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_next(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_guide" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_guide(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_dashboard" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_dashboard(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_review_data" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_review_data(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_review_flow" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_review_flow(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_ask" => {
            let args: AgentAskArgs = parse_args(arguments)?;
            tool_result(db.agent_ask(args.selector.as_deref().unwrap_or("latest"), &args.question)?)
        }
        "crabdb.agent_view" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_task_view(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_brief" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_brief(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_summary" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_summary(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_validate" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_validate(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_test_plan" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_test_plan(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_report" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_report(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_handoff" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_handoff(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_receipt" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_receipt(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_pr" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_pr_draft(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_story" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_story(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_tools" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_tools(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_risk" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_risk(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_impact" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_impact(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_review_map" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_review_map(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_confidence" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_confidence(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_ready" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_ready(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_diagnose" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_diagnose(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_workdir" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_workdir(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_changes" => {
            let args: AgentChangesArgs = parse_args(arguments)?;
            let _ = args.by_turn;
            tool_result(db.agent_changes_with_options(
                args.selector.as_deref().unwrap_or("latest"),
                args.by_operation,
                args.by_file,
            )?)
        }
        "crabdb.agent_delta" => {
            let args: AgentDeltaArgs = parse_args(arguments)?;
            let _ = args.by_turn;
            tool_result(db.agent_delta(
                args.selector.as_deref().unwrap_or("latest"),
                args.by_operation,
                args.file.as_deref(),
                args.patch,
            )?)
        }
        "crabdb.agent_new" => {
            let args: AgentNewArgs = parse_args(arguments)?;
            tool_result(db.agent_new(
                args.selector.as_deref().unwrap_or("latest"),
                args.file.as_deref(),
                args.patch,
            )?)
        }
        "crabdb.agent_mark_reviewed" => {
            let args: AgentMarkReviewedArgs = parse_args(arguments)?;
            tool_result(
                db.agent_mark_reviewed(args.selector.as_deref().unwrap_or("latest"), args.note)?,
            )
        }
        "crabdb.agent_mark_file_reviewed" => {
            let args: AgentMarkFileReviewedArgs = parse_args(arguments)?;
            tool_result(db.agent_mark_file_reviewed(
                args.selector.as_deref().unwrap_or("latest"),
                &args.path,
                args.note,
            )?)
        }
        "crabdb.agent_archive" => {
            let args: AgentArchiveArgs = parse_args(arguments)?;
            tool_result(db.agent_archive(
                args.selector.as_deref().unwrap_or("latest"),
                true,
                args.note,
            )?)
        }
        "crabdb.agent_unarchive" => {
            let args: AgentArchiveArgs = parse_args(arguments)?;
            tool_result(db.agent_archive(
                args.selector.as_deref().unwrap_or("latest"),
                false,
                args.note,
            )?)
        }
        "crabdb.agent_change" => {
            let args: AgentChangeArgs = parse_args(arguments)?;
            tool_result(db.agent_change_set(
                args.selector.as_deref().unwrap_or("latest"),
                args.card.as_deref().unwrap_or("1"),
                args.patch,
            )?)
        }
        "crabdb.agent_timeline" => {
            let args: AgentChangesArgs = parse_args(arguments)?;
            let _ = args.by_turn;
            tool_result(db.agent_timeline(
                args.selector.as_deref().unwrap_or("latest"),
                args.by_operation,
            )?)
        }
        "crabdb.agent_files" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_files(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_file" => {
            let args: AgentFileArgs = parse_args(arguments)?;
            tool_result(db.agent_file(
                args.selector.as_deref().unwrap_or("latest"),
                &args.path,
                args.patch,
            )?)
        }
        "crabdb.agent_checkpoints" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_checkpoints(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_why" => {
            let args: AgentWhyArgs = parse_args(arguments)?;
            tool_result(db.agent_why(args.selector.as_deref().unwrap_or("latest"), &args.path)?)
        }
        "crabdb.agent_turn" => {
            let args: AgentTurnArgs = parse_args(arguments)?;
            tool_result(db.agent_turn(
                args.selector.as_deref().unwrap_or("latest"),
                args.turn.as_deref().unwrap_or("last"),
                args.file.as_deref(),
                args.patch,
            )?)
        }
        "crabdb.agent_compare" => {
            let args: AgentCompareArgs = parse_args(arguments)?;
            tool_result(db.agent_compare(&args.left, &args.right)?)
        }
        "crabdb.agent_test" => {
            let args: AgentGateArgs = parse_args(arguments)?;
            let options = LaneGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_agent_test_with_options(
                args.selector.as_deref().unwrap_or("latest"),
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.agent_eval" => {
            let args: AgentGateArgs = parse_args(arguments)?;
            let options = LaneGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_agent_eval_with_options(
                args.selector.as_deref().unwrap_or("latest"),
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.agent_diff" => {
            let args: AgentDiffArgs = parse_args(arguments)?;
            tool_result(db.agent_diff(
                args.selector.as_deref().unwrap_or("latest"),
                args.turn.as_deref(),
                args.operation.as_deref(),
                args.checkpoint.as_deref(),
                args.last_turn,
                args.file.as_deref(),
                args.patch,
            )?)
        }
        "crabdb.agent_review" => {
            let args: AgentSelectorArgs = parse_args(arguments)?;
            tool_result(db.agent_review(args.selector.as_deref().unwrap_or("latest"))?)
        }
        "crabdb.agent_focus" => {
            let args: AgentFocusArgs = parse_args(arguments)?;
            tool_result(db.agent_focus(
                args.selector.as_deref().unwrap_or("latest"),
                args.file.as_deref(),
                args.patch,
            )?)
        }
        "crabdb.agent_apply" => {
            let args: AgentApplyArgs = parse_args(arguments)?;
            tool_result(db.agent_apply(
                args.selector.as_deref().unwrap_or("latest"),
                args.dry_run,
                args.message,
            )?)
        }
        "crabdb.agent_finish" => {
            let args: AgentFinishArgs = parse_args(arguments)?;
            tool_result(db.agent_finish(
                args.selector.as_deref().unwrap_or("latest"),
                args.dry_run,
                args.message,
                args.note,
            )?)
        }
        "crabdb.agent_rewind" => {
            let args: AgentRewindArgs = parse_args(arguments)?;
            tool_result(db.agent_rewind(args.selector.as_deref().unwrap_or("latest"), &args.to)?)
        }
        "crabdb.agent_undo" => {
            let args: AgentUndoArgs = parse_args(arguments)?;
            tool_result(db.agent_undo(
                args.selector.as_deref().unwrap_or("latest"),
                args.last_turn,
                args.turn.as_deref(),
                args.prompt.as_deref(),
                args.last_operation,
            )?)
        }
        "crabdb.config_list" => tool_result(db.config_entries()),
        "crabdb.config_get" => {
            let args: ConfigKeyArgs = parse_args(arguments)?;
            tool_result(db.config_get(&args.key)?)
        }
        "crabdb.config_set" => {
            let args: ConfigSetArgs = parse_args(arguments)?;
            tool_result(db.config_set(&args.key, &args.value)?)
        }
        "crabdb.ignore_list" => tool_result(db.ignore_list()?),
        "crabdb.ignore_add" => {
            let args: IgnorePatternArgs = parse_args(arguments)?;
            tool_result(db.ignore_add(&args.pattern)?)
        }
        "crabdb.ignore_remove" => {
            let args: IgnorePatternArgs = parse_args(arguments)?;
            tool_result(db.ignore_remove(&args.pattern)?)
        }
        "crabdb.ignore_check" => {
            let args: IgnoreCheckArgs = parse_args(arguments)?;
            tool_result(db.ignore_check(&args.path)?)
        }
        "crabdb.guardrail_check" => {
            let args: GuardrailCheckArgs = parse_args(arguments)?;
            tool_result(db.guardrail_check(
                args.lane.as_deref(),
                &args.action,
                args.summary.as_deref(),
                args.payload,
                &args.paths,
            )?)
        }
        _ => return Ok(None),
    };
    Ok(Some(value?))
}
