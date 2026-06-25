use serde_json::{json, Value};

use crate::{Error, Result};

use super::{types::*, utils::from_arguments};

pub(crate) fn handle_prompt_get(params: Value) -> Result<Value> {
    let args: PromptGetArgs = from_arguments(params)?;
    match args.name.as_str() {
        PROMPT_AGENT_TASK => {
            let agent = prompt_arg(&args.arguments, "agent")?;
            let task = prompt_arg(&args.arguments, "task")?;
            let branch = prompt_arg_optional(&args.arguments, "branch")?
                .unwrap_or_else(|| "main".to_string());
            prompt_result(
                "Safe CrabDB agent task workflow",
                format!(
                    "Run this CrabDB task using the MCP tools and resources.\n\n\
Agent: `{agent}`\nBase branch: `{branch}`\nTask:\n{task}\n\n\
Workflow:\n\
1. Read `crabdb://workspace/status` and `crabdb://docs/agent-workflows` before mutating anything.\n\
2. Use `crabdb.agent_spawn` for `{agent}` from `{branch}` if it does not already exist.\n\
3. Start a turn with `crabdb.begin_turn`, then attach the user request with `crabdb.add_message`.\n\
4. Claim busy paths with `crabdb.agent_claim` when multiple agents may edit the same files, then prefer structured patches through `crabdb.apply_patch`; preflight risky shell, network, deploy, destructive, or ignored-path work with `crabdb.guardrail_check`.\n\
5. Record trace spans/events for tool calls, guardrails, and handoffs.\n\
6. Request human approval with `crabdb.approval_request` when `crabdb.guardrail_check` returns `approval_required`; keep the returned run checkpoint and resume it with `crabdb.run_resume` after approval.\n\
7. Run `crabdb.run_test` and, when model/policy quality matters, `crabdb.run_eval`.\n\
8. End the turn with `crabdb.end_turn`, inspect `crabdb.agent_status`, `crabdb.agent_handoff`, and `crabdb.diff_agent`, then queue or merge only after review.\n\
9. If merge conflicts appear, use `crabdb.conflict_show` and `crabdb.conflict_resolve`; do not overwrite target changes silently."
                ),
                Some((RESOURCE_AGENT_WORKFLOWS, "text/markdown", AGENT_WORKFLOWS_MD)),
            )
        }
        PROMPT_REVIEW_AGENT => {
            let agent = prompt_arg(&args.arguments, "agent")?;
            prompt_result(
                "CrabDB agent review checklist",
                format!(
                    "Review CrabDB agent `{agent}` before accepting its work.\n\n\
Checklist:\n\
1. Read `crabdb://workspace/doctor`, `crabdb://workspace/agents`, and `crabdb://workspace/conflicts`.\n\
2. Call `crabdb.agent_contribution` for `{agent}` and inspect changed paths, operations, sessions, events, approvals, and latest gates.\n\
3. Call `crabdb.agent_handoff` for `{agent}` and use its current session context, trace spans, events, and next steps as the transfer packet.\n\
4. Call `crabdb.agent_readiness` for `{agent}` and treat blockers as stop conditions before merge.\n\
5. Call `crabdb.agent_status` for `{agent}` and confirm the branch/workdir state is clean enough to review.\n\
6. Call `crabdb.diff_agent` with patches and line ids; inspect provenance with `crabdb.why`, `crabdb.history`, and `crabdb.code_from` when a change is unclear.\n\
7. Confirm latest tests and evals passed or explain why warnings are acceptable.\n\
8. Use `crabdb.approval_request` for any unresolved human decision and inspect linked paused runs with `crabdb.run_list`.\n\
9. Prefer `crabdb.merge_queue_add` plus `crabdb.merge_queue_run` for shared target branches; use direct `merge-agent` only for one-off merges.\n\
10. If conflicts exist, stop review and switch to the `{PROMPT_RESOLVE_CONFLICT}` prompt."
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
1. Call `crabdb.conflict_show` with `conflict_set_id = {conflict_set_id}` and inspect every path in the conflict set.\n\
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
