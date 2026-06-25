use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::{CrabDb, Error, Result};

use super::{types::*, utils::from_arguments};

pub(crate) fn handle_completion_complete(db: &mut CrabDb, params: Value) -> Result<Value> {
    let args: CompletionArgs = from_arguments(params)?;
    let candidates = match args.reference.reference_type.as_str() {
        "ref/prompt" => {
            let name = args.reference.name.as_deref().ok_or_else(|| {
                Error::InvalidInput("completion ref/prompt requires `name`".to_string())
            })?;
            prompt_completion_candidates(db, name, &args.argument.name)?
        }
        "ref/resource" => {
            let uri = args.reference.uri.as_deref().ok_or_else(|| {
                Error::InvalidInput("completion ref/resource requires `uri`".to_string())
            })?;
            resource_completion_candidates(db, uri, &args.argument.name)?
        }
        other => {
            return Err(Error::InvalidInput(format!(
                "unsupported MCP completion reference type `{other}`"
            )));
        }
    };
    Ok(completion_result(candidates, &args.argument.value))
}

fn prompt_completion_candidates(
    db: &CrabDb,
    prompt_name: &str,
    argument_name: &str,
) -> Result<Vec<String>> {
    match (prompt_name, argument_name) {
        (PROMPT_AGENT_TASK | PROMPT_REVIEW_AGENT, "agent") => agent_completion_candidates(db),
        (PROMPT_AGENT_TASK, "branch") => branch_completion_candidates(db),
        (PROMPT_RESOLVE_CONFLICT, "conflict_set_id") => conflict_completion_candidates(db),
        (PROMPT_AGENT_TASK, "task") => Ok(Vec::new()),
        (PROMPT_AGENT_TASK | PROMPT_REVIEW_AGENT | PROMPT_RESOLVE_CONFLICT, _) => Ok(Vec::new()),
        (other, _) => Err(Error::InvalidInput(format!(
            "MCP prompt `{other}` not found"
        ))),
    }
}

fn resource_completion_candidates(
    db: &CrabDb,
    uri_template: &str,
    argument_name: &str,
) -> Result<Vec<String>> {
    match (uri_template, argument_name) {
        (
            RESOURCE_AGENT_TEMPLATE
            | RESOURCE_AGENT_STATUS_TEMPLATE
            | RESOURCE_AGENT_CONTRIBUTION_TEMPLATE
            | RESOURCE_AGENT_GATES_TEMPLATE
            | RESOURCE_AGENT_READINESS_TEMPLATE
            | RESOURCE_AGENT_HANDOFF_TEMPLATE
            | RESOURCE_AGENT_DIFF_TEMPLATE,
            "agent",
        ) => agent_completion_candidates(db),
        (RESOURCE_SESSION_TEMPLATE, "session_id") => session_completion_candidates(db),
        (RESOURCE_TURN_TEMPLATE, "turn_id") => turn_completion_candidates(db),
        (RESOURCE_CONFLICT_TEMPLATE, "conflict_set_id") => conflict_completion_candidates(db),
        (RESOURCE_APPROVAL_TEMPLATE, "approval_id") => approval_completion_candidates(db),
        (RESOURCE_RUN_TEMPLATE, "run_id") => run_completion_candidates(db),
        (RESOURCE_SPAN_TEMPLATE, "span_id") => span_completion_candidates(db),
        (
            RESOURCE_AGENT_TEMPLATE
            | RESOURCE_AGENT_STATUS_TEMPLATE
            | RESOURCE_AGENT_CONTRIBUTION_TEMPLATE
            | RESOURCE_AGENT_GATES_TEMPLATE
            | RESOURCE_AGENT_READINESS_TEMPLATE
            | RESOURCE_AGENT_HANDOFF_TEMPLATE
            | RESOURCE_AGENT_DIFF_TEMPLATE
            | RESOURCE_SESSION_TEMPLATE
            | RESOURCE_TURN_TEMPLATE
            | RESOURCE_CONFLICT_TEMPLATE
            | RESOURCE_APPROVAL_TEMPLATE
            | RESOURCE_RUN_TEMPLATE
            | RESOURCE_SPAN_TEMPLATE,
            _,
        ) => Ok(Vec::new()),
        (other, _) => Err(Error::InvalidInput(format!(
            "MCP resource template `{other}` not found"
        ))),
    }
}

fn agent_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    let mut values = BTreeSet::new();
    for agent in db.list_agents()? {
        values.insert(agent.record.name);
        values.insert(agent.record.agent_id);
    }
    Ok(values.into_iter().collect())
}

fn branch_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_branches()?
        .into_iter()
        .map(|branch| branch.name)
        .collect())
}

fn session_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_agent_sessions(None)?
        .into_iter()
        .map(|session| session.session_id)
        .collect())
}

fn turn_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    let mut values = BTreeSet::new();
    for session in db.list_agent_sessions(None)? {
        for turn in db.show_agent_session(&session.session_id)?.turns {
            values.insert(turn.turn_id);
        }
    }
    for event in db.list_agent_events(None, None, None, None, 1000)? {
        if let Some(turn_id) = event.turn_id {
            values.insert(turn_id);
        }
    }
    Ok(values.into_iter().collect())
}

fn conflict_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_conflicts()?
        .into_iter()
        .map(|conflict| conflict.conflict_set_id)
        .collect())
}

fn approval_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_agent_approvals(None, None)?
        .into_iter()
        .map(|approval| approval.approval_id)
        .collect())
}

fn run_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_agent_run_states(None, None)?
        .into_iter()
        .map(|run_state| run_state.run_id)
        .collect())
}

fn span_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_agent_trace_spans(None, None, None, None, 1000)?
        .into_iter()
        .map(|span| span.span_id)
        .collect())
}

fn completion_result(candidates: Vec<String>, value: &str) -> Value {
    let needle = value.to_ascii_lowercase();
    let mut starts_with = Vec::new();
    let mut contains = Vec::new();
    let mut seen = BTreeSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.clone()) {
            continue;
        }
        let candidate_lower = candidate.to_ascii_lowercase();
        if needle.is_empty() || candidate_lower.starts_with(&needle) {
            starts_with.push(candidate);
        } else if candidate_lower.contains(&needle) {
            contains.push(candidate);
        }
    }
    starts_with.sort();
    contains.sort();
    starts_with.extend(contains);
    let total = starts_with.len();
    let values = starts_with.into_iter().take(100).collect::<Vec<_>>();
    json!({
        "completion": {
            "values": values,
            "total": total,
            "hasMore": total > 100
        }
    })
}
