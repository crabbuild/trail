use serde_json::{json, Value};

use crate::{Error, Result};

use super::{types::*, utils::from_arguments};

pub(crate) fn handle_prompt_get(params: Value) -> Result<Value> {
    let args: PromptGetArgs = from_arguments(params)?;
    match args.name.as_str() {
        PROMPT_LANE_TASK => {
            let lane = prompt_arg(&args.arguments, "lane")?;
            let task = prompt_arg(&args.arguments, "task")?;
            let branch = prompt_arg_optional(&args.arguments, "branch")?
                .unwrap_or_else(|| "main".to_string());
            prompt_result(
                "Safe CrabDB lane task workflow",
                format!(
                    "Run this CrabDB task using the MCP tools and resources.\n\n\
Lane: `{lane}`\nBase branch: `{branch}`\nTask:\n{task}\n\n\
Workflow:\n\
1. Read `crabdb://workspace/status` and `crabdb://docs/lane-workflows` before mutating anything.\n\
2. Use `crabdb.lane_spawn` for `{lane}` from `{branch}` if it does not already exist.\n\
3. Start a turn with `crabdb.begin_turn`, then attach the user request with `crabdb.add_message`.\n\
4. Claim busy paths with `crabdb.lane_claim` when multiple lanes may edit the same files, then prefer structured patches through `crabdb.apply_patch`; preflight risky shell, network, deploy, destructive, or ignored-path work with `crabdb.guardrail_check`.\n\
5. Record trace spans/events for tool calls, guardrails, and handoffs.\n\
6. Request human approval with `crabdb.approval_request` when `crabdb.guardrail_check` returns `approval_required`; keep the returned run checkpoint and resume it with `crabdb.run_resume` after approval.\n\
7. Run `crabdb.run_test` and, when model/policy quality matters, `crabdb.run_eval`.\n\
8. End the turn with `crabdb.end_turn`, inspect `crabdb.lane_status`, `crabdb.lane_handoff`, and `crabdb.diff_lane`, then queue or merge only after review.\n\
9. Treat `crabdb.lane_claim` leases as advisory coordination unless `lane.claim_enforcement` is set to `warn` or `reject`; rely on readiness, conflict sets, and merge review before accepting work.\n\
10. If the lane goes sideways, use `crabdb.lane_rewind` with `record_current = true` before returning to a known-good change or root.\n\
11. If merge conflicts appear, use `crabdb.conflict_show` and `crabdb.conflict_resolve`; do not overwrite target changes silently."
                ),
                Some((RESOURCE_LANE_WORKFLOWS, "text/markdown", LANE_WORKFLOWS_MD)),
            )
        }
        PROMPT_REVIEW_LANE => {
            let lane = prompt_arg(&args.arguments, "lane")?;
            prompt_result(
                "CrabDB lane review checklist",
                format!(
                    "Review CrabDB lane `{lane}` before accepting its work.\n\n\
Checklist:\n\
1. Read `crabdb://workspace/doctor`, `crabdb://workspace/lanes`, and `crabdb://workspace/conflicts`.\n\
2. Call `crabdb.lane_review` for `{lane}` first and inspect readiness, evidence summary, gates, approvals, conflicts, changed paths, recent operations, and next steps.\n\
3. Treat `crabdb.lane_review` readiness blockers as stop conditions before merge.\n\
4. Call `crabdb.lane_handoff`, `crabdb.lane_contribution`, or `crabdb.lane_status` only when you need deeper session, trace, contribution, or branch/workdir detail.\n\
5. Call `crabdb.lane_status` for `{lane}` and confirm the branch/workdir state is clean enough to review.\n\
6. Call `crabdb.diff_lane` with patches and line ids; inspect provenance with `crabdb.why`, `crabdb.history`, and `crabdb.code_from` when a change is unclear.\n\
7. Confirm latest tests and evals passed or explain why warnings are acceptable.\n\
8. Use `crabdb.approval_request` for any unresolved human decision and inspect linked paused runs with `crabdb.run_list`.\n\
9. Prefer `crabdb.merge_queue_add` plus `crabdb.merge_queue_run` for shared target branches; use direct `merge-lane` only for one-off merges.\n\
10. If the work should be abandoned, use `crabdb.lane_rewind` with `record_current = true` instead of silently moving refs.\n\
11. If conflicts exist, stop review and switch to the `{PROMPT_RESOLVE_CONFLICT}` prompt."
                ),
                Some((RESOURCE_CLI_REFERENCE, "text/markdown", CLI_REFERENCE_MD)),
            )
        }
        PROMPT_RESOLVE_CONFLICT => {
            let conflict_set_id = prompt_arg(&args.arguments, "conflict_set_id")?;
            prompt_result(
                "CrabDB conflict resolution workflow",
                format!(
                    "Resolve CrabDB conflict `{conflict_set_id}` safely.\n\n\
Workflow:\n\
1. Call `crabdb.conflict_show` with `conflict_set_id = {conflict_set_id}` and inspect the explanation before reading raw path details.\n\
2. Read `crabdb://workspace/conflicts` and confirm this conflict is still open.\n\
3. Decide per conflicted path whether source, target, or manual content should win. Keep non-conflicting source changes merged.\n\
4. Use `crabdb.conflict_resolve` with either `take: source`, `take: target`, or `manual.files` covering every conflicted path and no unrelated paths.\n\
5. If manual content is used, preserve intended executable mode or set `delete: true` for intended deletions.\n\
6. After resolution, run status/diff plus the relevant `crabdb.run_test` or `crabdb.run_eval` gates before considering the merge complete.\n\
7. If CrabDB reports a stale branch, stop and re-run the merge from the current refs rather than forcing stale content."
                ),
                None,
            )
        }
        PROMPT_REVIEW_AGENT => {
            let selector = prompt_arg_optional(&args.arguments, "selector")?
                .unwrap_or_else(|| "latest".to_string());
            prompt_result(
                "CrabDB agent task review workflow",
                format!(
                    "Review CrabDB agent task `{selector}` using the high-level agent tools.\n\n\
Workflow:\n\
1. If the user asks a plain-language question such as show the agent board, show all agents, which task first, apply order, what needs attention, what changed, what should I do next, what did the agent do, what did the agent change, what files did it touch, where is the workdir, where did the agent edit, which prompt changed a file, last prompt, what changed in the last prompt, what changed in README.md in the last prompt, show transcript, walk me through review, review checklist, review map, review files, file checklist, test plan, validation plan, go/no-go, am I good, final check, should I ship, what should I review, what should I review first, what file should I review first, what file should I open, where should I look first, open review, review this task, what tools were used, handoff this to another agent, what should I put in the PR, give me a summary to share, what commit message should I use, what tests should I run, is it tested, how should I test this, can I merge, why can't I apply, what is blocking this task, why did it fail, what went wrong, any red flags, what should I worry about, which files are risky, is it safe to land, recover, explain a file, show the diff, or show a patch/diff, call `crabdb.agent_ask` for `{selector}` and present the routed report.\n\
2. Call `crabdb.agent_guide` for `{selector}` when the user asks how to use CrabDB from here, wants the shortest setup/review/apply/recovery workflow, or seems lost in command choices.\n\
3. Start with `crabdb.agent_board` when the user wants all agent tasks, multiple agents, or a low-noise board of what needs attention across tasks. Use `crabdb.agent_stack` when the user asks which task to apply first, asks for apply order, or wants overlap checks across tasks.\n\
4. Start with `crabdb.agent_dashboard` for `{selector}` when the user wants the compact cockpit for one task: next action, focus file, validation, changed files, risk, and apply readiness.\n\
5. Call `crabdb.agent_review_data` for `{selector}` when an editor, side panel, or UI needs one structured review packet with file review progress, focus file, changes by file, confidence, validation, risk, and readiness.\n\
6. Call `crabdb.agent_review_flow` for `{selector}` when the user asks to walk through review, wants a review checklist, or wants the review/validate/finish sequence in one place.\n\
7. Call `crabdb.agent_summary` for `{selector}` when you need the fuller shareable review cockpit: readiness, risk, validation, receipt Markdown, PR draft, and the one recommended next action.\n\
8. Call `crabdb.agent_next` for `{selector}` only when you need a smaller next-action payload.\n\
9. Call `crabdb.agent_handoff` for `{selector}` when the user asks to hand work to another human or agent. Call `crabdb.agent_receipt` for a shorter copyable after-action note; call `crabdb.agent_report` only when a deeper review bundle is needed.\n\
10. Call `crabdb.agent_focus` for `{selector}` when the user asks what to inspect first; it bundles review priority, why, and a focused diff summary.\n\
11. Call `crabdb.agent_new` for `{selector}` when the user asks what changed since they last reviewed the task; call `crabdb.agent_mark_file_reviewed` only when the user explicitly says one changed file has been reviewed, and call `crabdb.agent_mark_reviewed` only when the user explicitly says the current checkpoint has been reviewed.\n\
12. Call `crabdb.agent_finish` when the user asks to apply and clean up a task in one step; call `crabdb.agent_archive` when the user asks only to close, hide, dismiss, or clean up an already-finished task; call `crabdb.agent_unarchive` only when the user asks to restore it. Do not call low-level lane removal for inbox cleanup.\n\
13. Call `crabdb.agent_delta` for `{selector}` when the user asks what just changed in the latest prompt or wants the newest turn/operation patch.\n\
14. Call `crabdb.agent_changes` for `{selector}` when the user asks what changed across the whole task; present its `next` command and high-level change cards before exposing raw turns or operation ids. Pass `by-file` when the user wants a file-first review map.\n\
15. Call `crabdb.agent_change` for `{selector}` when the user wants one change card expanded into files, provenance, tools, commands, and optional focused patches.\n\
16. Call `crabdb.agent_impact` for `{selector}` when the user asks about impact, blast radius, touched areas, scope, surface area, or what areas need review/testing.\n\
17. Call `crabdb.agent_review_map` for `{selector}` when the user asks for a review map, file checklist, review by area, review all files, or a low-burden file-by-file checklist.\n\
18. Call `crabdb.agent_tools` for `{selector}` when the user asks which tools ran, what commands were available, or how tool calls relate to turns/checkpoints/files.\n\
19. Call `crabdb.agent_timeline` for `{selector}` when the user asks what happened when, which prompt caused a checkpoint, or how turns and operations connect.\n\
20. Call `crabdb.agent_files` for `{selector}` when the user wants the full file-by-file review. Call `crabdb.agent_file` when the user points at one open file or asks why one path changed.\n\
21. Call `crabdb.agent_turn` when the user asks what happened in one prompt; use `file` and `patch` only when patch detail is needed.\n\
22. Call `crabdb.agent_diff` with `last_turn`, a turn selector, or an operation selector only when raw patch detail is needed; include `file` when reviewing one specific path.\n\
23. Call `crabdb.agent_confidence` when the user asks for go/no-go, final check, or whether they should ship; call `crabdb.agent_ready` or `crabdb.agent_risk` when they specifically ask about apply readiness or risk.\n\
23. Call `crabdb.agent_test_plan` for `{selector}` when the user asks what tests to run, how to test, or wants a validation plan. Call `crabdb.agent_validate` when the user asks for validation status, whether tests passed, or latest gate results. Only call `crabdb.agent_test` or `crabdb.agent_eval` after the user explicitly asks to run validation.\n\
24. Call `crabdb.agent_pr` when the user asks for a pull request title/body; it is read-only and does not create a remote PR.\n\
25. Call `crabdb.agent_diagnose` before suggesting undo or rewind when the task appears stuck, blocked, or sideways.\n\
26. Do not call `crabdb.agent_apply`, `crabdb.agent_finish`, `crabdb.agent_rewind`, or `crabdb.agent_undo` unless the user explicitly asks for apply, finish, or recovery.\n\
27. End with one concrete next command from the board, handoff, receipt, report, diagnosis, impact, review-map, test-plan, new-changes, delta, tools, timeline, change-set, file, dashboard, review-flow, guide, or next-action output."
                ),
                Some((RESOURCE_CLI_REFERENCE, "text/markdown", CLI_REFERENCE_MD)),
            )
        }
        PROMPT_RECOVER_AGENT => {
            let selector = prompt_arg_optional(&args.arguments, "selector")?
                .unwrap_or_else(|| "latest".to_string());
            prompt_result(
                "CrabDB agent task recovery workflow",
                format!(
                    "Recover CrabDB agent task `{selector}` safely.\n\n\
Workflow:\n\
1. Call `crabdb.agent_diagnose` for `{selector}` first so the user can see likely issue, evidence, friendly recovery targets, and safe next commands.\n\
2. Call `crabdb.agent_report` only when the user needs deeper status, changed files, turns, risk, and transcript context.\n\
3. Call `crabdb.agent_checkpoints` for `{selector}` when the user wants the full target list; present friendly targets such as `before-last-turn`, `before-turn:N`, and `turn:N` with the exact checkpoint ids.\n\
4. If the user wants to undo a prompt, prefer `crabdb.agent_undo` with `last_turn`, `turn`, or `prompt` because it uses user-facing task language.\n\
5. If the user wants a specific checkpoint, use `crabdb.agent_rewind` with the friendly target from `crabdb.agent_checkpoints`.\n\
6. Before calling undo or rewind, clearly state that it is destructive because it moves task state and may refresh the materialized workdir.\n\
7. After recovery, call `crabdb.agent_diagnose`, `crabdb.agent_summary`, or `crabdb.agent_files` again to verify the task state and show the new next action.\n\
8. Never guess a checkpoint id when a friendly target is available; ask for confirmation if multiple targets could match."
                ),
                Some((RESOURCE_CLI_REFERENCE, "text/markdown", CLI_REFERENCE_MD)),
            )
        }
        PROMPT_APPLY_AGENT => {
            let selector = prompt_arg_optional(&args.arguments, "selector")?
                .unwrap_or_else(|| "latest".to_string());
            prompt_result(
                "CrabDB agent task apply workflow",
                format!(
                    "Apply CrabDB agent task `{selector}` safely.\n\n\
Workflow:\n\
1. Call `crabdb.agent_summary` and `crabdb.agent_ready` for `{selector}` before applying.\n\
2. If `crabdb.agent_ready` returns `ready = false`, stop and explain the exact blocker, Git preflight error, or recommended test/eval command.\n\
3. Use `crabdb.agent_files` and `crabdb.agent_diff` for patch review when the changed files or risk reasons need detail; pass `file` to keep the patch focused on one path.\n\
4. When the user asks to apply, call `crabdb.agent_apply` in dry-run mode first and summarize the plan, Git branch, changed files, generated commit message, and any safety failure. When the user asks to finish, call `crabdb.agent_finish` in dry-run mode first and summarize both the apply plan and whether it would archive the task.\n\
5. Only call non-dry-run `crabdb.agent_apply` or `crabdb.agent_finish` after the dry run is clean and the user confirms they want the Git fast-forward/apply action. Use finish only when the user also wants the task hidden from the default inbox after success.\n\
6. If apply or finish fails or reports conflicts, stop. Do not retry with force. Show `crabdb.agent_report`, `crabdb.agent_files`, or conflict/readiness details as the next step.\n\
7. After successful apply or finish, show the created Git commit/checkpoint and offer `crabdb.agent_pr` for a read-only PR title/body draft or the `markdown` field from `crabdb.agent_receipt` for archival review."
                ),
                Some((RESOURCE_CLI_REFERENCE, "text/markdown", CLI_REFERENCE_MD)),
            )
        }
        other => Err(Error::InvalidInput(format!(
            "MCP prompt `{other}` not found"
        ))),
    }
}

fn prompt_arg(arguments: &Value, name: &str) -> Result<String> {
    let object = arguments.as_object().ok_or_else(|| {
        Error::InvalidInput("prompts/get `arguments` must be an object".to_string())
    })?;
    let Some(value) = object.get(name) else {
        return Err(Error::InvalidInput(format!(
            "prompt requires argument `{name}`"
        )));
    };
    let Some(value) = value.as_str() else {
        return Err(Error::InvalidInput(format!(
            "prompt argument `{name}` must be a string"
        )));
    };
    if value.trim().is_empty() {
        return Err(Error::InvalidInput(format!(
            "prompt argument `{name}` must not be empty"
        )));
    }
    Ok(value.to_string())
}

fn prompt_arg_optional(arguments: &Value, name: &str) -> Result<Option<String>> {
    if arguments.is_null() {
        return Ok(None);
    }
    let object = arguments.as_object().ok_or_else(|| {
        Error::InvalidInput("prompts/get `arguments` must be an object".to_string())
    })?;
    let Some(value) = object.get(name) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(Error::InvalidInput(format!(
            "prompt argument `{name}` must be a string"
        )));
    };
    if value.trim().is_empty() {
        return Err(Error::InvalidInput(format!(
            "prompt argument `{name}` must not be empty"
        )));
    }
    Ok(Some(value.to_string()))
}

fn prompt_result(
    description: &str,
    text: String,
    embedded_resource: Option<(&str, &str, &str)>,
) -> Result<Value> {
    let mut messages = vec![prompt_text_message(text)];
    if let Some((uri, mime_type, text)) = embedded_resource {
        messages.push(json!({
            "role": "user",
            "content": {
                "type": "resource",
                "resource": {
                    "uri": uri,
                    "mimeType": mime_type,
                    "text": text
                }
            }
        }));
    }
    Ok(json!({
        "description": description,
        "messages": messages
    }))
}

fn prompt_text_message(text: String) -> Value {
    json!({
        "role": "user",
        "content": {
            "type": "text",
            "text": text
        }
    })
}
