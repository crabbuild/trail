use std::path::PathBuf;

use serde_json::Value;

use crate::{AgentGateOptions, CrabDb, Error, PatchDocument, PatchEdit, Result};

use super::{response::tool_result, types::*, utils::from_arguments};

pub(crate) fn handle_tool_call(db: &mut CrabDb, params: Value) -> Result<Value> {
    let call: ToolCall = serde_json::from_value(params)?;
    match call.name.as_str() {
        "crabdb.doctor" => tool_result(db.doctor()?),
        "crabdb.status" => {
            let args: StatusArgs = from_arguments(call.arguments)?;
            tool_result(db.status(args.branch.as_deref())?)
        }
        "crabdb.diff" => {
            let args: DiffArgs = from_arguments(call.arguments)?;
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
            let args: TimelineArgs = from_arguments(call.arguments)?;
            tool_result(db.timeline_query(
                args.branch.as_deref(),
                args.session.as_deref(),
                args.agent.as_deref(),
                args.limit.unwrap_or(30),
            )?)
        }
        "crabdb.why" => {
            let args: WhyArgs = from_arguments(call.arguments)?;
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
            let args: HistoryArgs = from_arguments(call.arguments)?;
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
            let args: CodeFromArgs = from_arguments(call.arguments)?;
            tool_result(db.code_from(&args.selector)?)
        }
        "crabdb.agent_spawn" => {
            let args: AgentSpawnArgs = from_arguments(call.arguments)?;
            tool_result(db.spawn_agent_with_workdir(
                &args.name,
                args.from_ref.as_deref(),
                args.materialize,
                args.provider,
                args.model,
                args.workdir.map(PathBuf::from),
            )?)
        }
        "crabdb.agent_claim" => {
            let args: AgentClaimArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.claim_agent_path(&agent, &args.path, args.ttl_secs.unwrap_or(600))?)
        }
        "crabdb.agent_list" => tool_result(db.list_agents()?),
        "crabdb.agent_show" => {
            let args: AgentHandleArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_details(&agent)?)
        }
        "crabdb.agent_status" => {
            let args: AgentHandleArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_status(&agent)?)
        }
        "crabdb.agent_contribution" => {
            let args: AgentContributionArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_contribution(&agent, args.limit.unwrap_or(50))?)
        }
        "crabdb.agent_readiness" => {
            let args: AgentHandleArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_readiness(&agent)?)
        }
        "crabdb.agent_handoff" => {
            let args: AgentContributionArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_handoff(&agent, args.limit.unwrap_or(50))?)
        }
        "crabdb.agent_remove" => {
            let args: AgentRemoveArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.remove_agent(&agent, args.force)?)
        }
        "crabdb.config_list" => tool_result(db.config_entries()),
        "crabdb.config_get" => {
            let args: ConfigKeyArgs = from_arguments(call.arguments)?;
            tool_result(db.config_get(&args.key)?)
        }
        "crabdb.config_set" => {
            let args: ConfigSetArgs = from_arguments(call.arguments)?;
            tool_result(db.config_set(&args.key, &args.value)?)
        }
        "crabdb.session_start" => {
            let args: SessionStartArgs = from_arguments(call.arguments)?;
            tool_result(db.start_agent_session(&args.agent, args.title, args.id)?)
        }
        "crabdb.session_list" => {
            let args: SessionListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_agent_sessions(args.agent.as_deref())?)
        }
        "crabdb.session_current" => {
            let args: SessionCurrentArgs = from_arguments(call.arguments)?;
            tool_result(db.current_agent_sessions(args.agent.as_deref())?)
        }
        "crabdb.session_show" => {
            let args: SessionIdArgs = from_arguments(call.arguments)?;
            tool_result(db.show_agent_session(&args.session_id)?)
        }
        "crabdb.session_context" => {
            let args: SessionContextArgs = from_arguments(call.arguments)?;
            tool_result(db.agent_session_context(&args.session_id, args.limit.unwrap_or(50))?)
        }
        "crabdb.session_end" => {
            let args: SessionEndArgs = from_arguments(call.arguments)?;
            tool_result(db.end_agent_session(&args.session_id, &args.status)?)
        }
        "crabdb.approval_request" => {
            let args: ApprovalRequestArgs = from_arguments(call.arguments)?;
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
            let args: ApprovalListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_agent_approvals(args.agent.as_deref(), args.status.as_deref())?)
        }
        "crabdb.approval_show" => {
            let args: ApprovalShowArgs = from_arguments(call.arguments)?;
            tool_result(db.show_agent_approval(&args.approval_id)?)
        }
        "crabdb.approval_decide" => {
            let args: ApprovalDecideArgs = from_arguments(call.arguments)?;
            tool_result(db.decide_agent_approval(
                &args.approval_id,
                &args.decision,
                args.reviewer,
                args.note,
            )?)
        }
        "crabdb.run_pause" => {
            let args: AgentRunPauseArgs = from_arguments(call.arguments)?;
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
            let args: AgentRunListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_agent_run_states(args.agent.as_deref(), args.status.as_deref())?)
        }
        "crabdb.run_show" => {
            let args: AgentRunShowArgs = from_arguments(call.arguments)?;
            tool_result(db.show_agent_run_state(&args.run_id)?)
        }
        "crabdb.run_resume" => {
            let args: AgentRunResumeArgs = from_arguments(call.arguments)?;
            tool_result(db.resume_agent_run(&args.run_id, args.reviewer, args.note)?)
        }
        "crabdb.lease_acquire" => {
            let args: LeaseAcquireArgs = from_arguments(call.arguments)?;
            let mode = args.mode.unwrap_or_else(default_lease_mode);
            tool_result(db.acquire_lease(
                &args.agent,
                args.path.as_deref(),
                &mode,
                args.ttl_secs.unwrap_or(3600),
            )?)
        }
        "crabdb.lease_list" => {
            let args: LeaseListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_leases(args.all)?)
        }
        "crabdb.lease_release" => {
            let args: LeaseReleaseArgs = from_arguments(call.arguments)?;
            tool_result(db.release_lease(&args.lease_id)?)
        }
        "crabdb.anchor_create" => {
            let args: AnchorCreateArgs = from_arguments(call.arguments)?;
            tool_result(db.create_anchor(&args.path_line, args.label, args.branch.as_deref())?)
        }
        "crabdb.anchor_list" => tool_result(db.list_anchors()?),
        "crabdb.anchor_resolve" => {
            let args: AnchorIdArgs = from_arguments(call.arguments)?;
            tool_result(db.resolve_anchor(&args.anchor_id, args.branch.as_deref())?)
        }
        "crabdb.anchor_delete" => {
            let args: AnchorIdArgs = from_arguments(call.arguments)?;
            tool_result(db.delete_anchor(&args.anchor_id)?)
        }
        "crabdb.merge_queue_add" => {
            let args: MergeQueueAddArgs = from_arguments(call.arguments)?;
            tool_result(db.enqueue_merge(&args.source, &args.target, args.priority)?)
        }
        "crabdb.merge_queue_list" => tool_result(db.list_merge_queue()?),
        "crabdb.merge_queue_run" => {
            let args: MergeQueueRunArgs = from_arguments(call.arguments)?;
            tool_result(db.run_merge_queue(args.limit)?)
        }
        "crabdb.merge_queue_remove" => {
            let args: MergeQueueRemoveArgs = from_arguments(call.arguments)?;
            tool_result(db.remove_merge_queue(&args.selector)?)
        }
        "crabdb.conflict_list" => tool_result(db.list_conflicts()?),
        "crabdb.conflict_show" => {
            let args: ConflictIdArgs = from_arguments(call.arguments)?;
            tool_result(db.show_conflict(&args.conflict_set_id)?)
        }
        "crabdb.conflict_resolve" => {
            let args: ConflictResolveArgs = from_arguments(call.arguments)?;
            match (args.take, args.manual) {
                (Some(take), None) => {
                    tool_result(db.resolve_conflict(&args.conflict_set_id, &take)?)
                }
                (None, Some(manual)) => {
                    tool_result(db.resolve_conflict_manual(&args.conflict_set_id, manual)?)
                }
                (Some(_), Some(_)) => Err(Error::InvalidInput(
                    "conflict_resolve requires only one of `take` or `manual`".to_string(),
                )),
                (None, None) => Err(Error::InvalidInput(
                    "conflict_resolve requires `take` or `manual`".to_string(),
                )),
            }
        }
        "crabdb.begin_turn" => {
            let args: BeginTurnArgs = from_arguments(call.arguments)?;
            tool_result(db.begin_agent_turn(
                &args.agent,
                args.branch.as_deref(),
                args.session_title,
                args.base_change.as_deref(),
            )?)
        }
        "crabdb.add_message" => {
            let args: AddMessageArgs = from_arguments(call.arguments)?;
            let text = args.content.or(args.text).ok_or_else(|| {
                Error::InvalidInput("add_message requires `content` or `text`".to_string())
            })?;
            tool_result(db.add_agent_turn_message(&args.turn_id, &args.role, &text)?)
        }
        "crabdb.add_event" => {
            let args: AddEventArgs = from_arguments(call.arguments)?;
            tool_result(db.add_agent_turn_event(
                &args.turn_id,
                &args.event_type,
                args.payload,
                args.change_id.as_deref(),
                args.message_id.as_deref(),
            )?)
        }
        "crabdb.event_list" => {
            let args: EventListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_agent_events(
                args.agent.as_deref(),
                args.session.as_deref(),
                args.turn_id.as_deref(),
                args.event_type.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "crabdb.span_start" => {
            let args: SpanStartArgs = from_arguments(call.arguments)?;
            tool_result(db.start_agent_trace_span(
                &args.turn_id,
                &args.span_type,
                &args.name,
                args.parent.as_deref(),
                args.trace.as_deref(),
                args.attributes,
            )?)
        }
        "crabdb.span_end" => {
            let args: SpanEndArgs = from_arguments(call.arguments)?;
            tool_result(db.end_agent_trace_span(&args.span_id, &args.status, args.result)?)
        }
        "crabdb.span_list" => {
            let args: SpanListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_agent_trace_spans(
                args.agent.as_deref(),
                args.session.as_deref(),
                args.turn_id.as_deref(),
                args.trace_id.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "crabdb.span_summary" => {
            let args: SpanSummaryArgs = from_arguments(call.arguments)?;
            tool_result(db.summarize_agent_trace_spans(
                args.agent.as_deref(),
                args.session.as_deref(),
                args.turn_id.as_deref(),
                args.trace_id.as_deref(),
                args.slowest.unwrap_or(5),
            )?)
        }
        "crabdb.span_show" => {
            let args: SpanShowArgs = from_arguments(call.arguments)?;
            tool_result(db.show_agent_trace_span(&args.span_id)?)
        }
        "crabdb.apply_patch" => {
            let args: ApplyPatchArgs = from_arguments(call.arguments)?;
            let turn_id = args.turn_id.clone();
            tool_result(db.apply_agent_turn_patch(&turn_id, patch_document_from_args(args))?)
        }
        "crabdb.end_turn" => {
            let args: EndTurnArgs = from_arguments(call.arguments)?;
            tool_result(db.end_agent_turn(&args.turn_id, &args.status)?)
        }
        "crabdb.show_turn" => {
            let args: TurnIdArgs = from_arguments(call.arguments)?;
            tool_result(db.show_agent_turn(&args.turn_id)?)
        }
        "crabdb.diff_agent" => {
            let args: DiffAgentArgs = from_arguments(call.arguments)?;
            tool_result(db.diff_agent_with_options(&args.agent, args.patch, args.show_line_ids)?)
        }
        "crabdb.gate_history" => {
            let args: GateHistoryArgs = from_arguments(call.arguments)?;
            tool_result(db.agent_gate_history(
                &args.agent,
                args.kind.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "crabdb.run_test" => {
            let args: RunTestArgs = from_arguments(call.arguments)?;
            let options = AgentGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_agent_test_with_options(
                &args.agent,
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.run_eval" => {
            let args: RunTestArgs = from_arguments(call.arguments)?;
            let options = AgentGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_agent_eval_with_options(
                &args.agent,
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.sync_workdir" => {
            let args: SyncWorkdirArgs = from_arguments(call.arguments)?;
            tool_result(db.sync_agent_workdir(&args.agent, args.force)?)
        }
        "crabdb.ignore_list" => tool_result(db.ignore_list()?),
        "crabdb.ignore_add" => {
            let args: IgnorePatternArgs = from_arguments(call.arguments)?;
            tool_result(db.ignore_add(&args.pattern)?)
        }
        "crabdb.ignore_remove" => {
            let args: IgnorePatternArgs = from_arguments(call.arguments)?;
            tool_result(db.ignore_remove(&args.pattern)?)
        }
        "crabdb.ignore_check" => {
            let args: IgnoreCheckArgs = from_arguments(call.arguments)?;
            tool_result(db.ignore_check(&args.path)?)
        }
        "crabdb.guardrail_check" => {
            let args: GuardrailCheckArgs = from_arguments(call.arguments)?;
            tool_result(db.guardrail_check(
                args.agent.as_deref(),
                &args.action,
                args.summary.as_deref(),
                args.payload,
                &args.paths,
            )?)
        }
        _ => Err(Error::InvalidInput(format!(
            "unknown MCP tool `{}`",
            call.name
        ))),
    }
}

fn patch_document_from_args(args: ApplyPatchArgs) -> PatchDocument {
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
    PatchDocument {
        base_change: args.base_change,
        message: args.message,
        session_id: args.session_id,
        allow_ignored: args.allow_ignored,
        edits,
    }
}
