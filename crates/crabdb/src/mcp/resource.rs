use serde_json::{json, Value};

use crate::{CrabDb, Error, Result};

use super::{response::pretty_json, types::*, utils::from_arguments};

pub(crate) fn handle_resource_read(db: &mut CrabDb, params: Value) -> Result<Value> {
    let args: ResourceReadArgs = from_arguments(params)?;
    let (mime_type, text) = match args.uri.as_str() {
        RESOURCE_STATUS => ("application/json", pretty_json(&db.status(None)?)?),
        RESOURCE_DOCTOR => ("application/json", pretty_json(&db.doctor()?)?),
        RESOURCE_AGENTS => ("application/json", pretty_json(&db.list_agents()?)?),
        RESOURCE_MERGE_QUEUE => ("application/json", pretty_json(&db.list_merge_queue()?)?),
        RESOURCE_CONFLICTS => ("application/json", pretty_json(&db.list_conflicts()?)?),
        RESOURCE_OPENAPI => (
            "application/json",
            serde_json::to_string_pretty(&crate::server::openapi_spec())?,
        ),
        RESOURCE_USER_GUIDE => ("text/markdown", USER_GUIDE_MD.to_string()),
        RESOURCE_AGENT_WORKFLOWS => ("text/markdown", AGENT_WORKFLOWS_MD.to_string()),
        RESOURCE_CLI_REFERENCE => ("text/markdown", CLI_REFERENCE_MD.to_string()),
        other => templated_resource(db, other)?,
    };
    Ok(json!({
        "contents": [
            {
                "uri": args.uri,
                "mimeType": mime_type,
                "text": text
            }
        ]
    }))
}

fn templated_resource(db: &mut CrabDb, uri: &str) -> Result<(&'static str, String)> {
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/status",
        RESOURCE_AGENT_STATUS_TEMPLATE,
    )? {
        return Ok(("application/json", pretty_json(&db.agent_status(&agent)?)?));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/contribution",
        RESOURCE_AGENT_CONTRIBUTION_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_contribution(&agent, 50)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/gates",
        RESOURCE_AGENT_GATES_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_gate_history(&agent, None, 50)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/readiness",
        RESOURCE_AGENT_READINESS_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_readiness(&agent)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/handoff",
        RESOURCE_AGENT_HANDOFF_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_handoff(&agent, 50)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/diff",
        RESOURCE_AGENT_DIFF_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.diff_agent_with_options(&agent, false, false)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "",
        RESOURCE_AGENT_TEMPLATE,
    )? {
        return Ok(("application/json", pretty_json(&db.agent_details(&agent)?)?));
    }
    if let Some(session_id) = template_uri_argument(
        uri,
        "crabdb://workspace/sessions/",
        "",
        RESOURCE_SESSION_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.show_agent_session(&session_id)?)?,
        ));
    }
    if let Some(turn_id) =
        template_uri_argument(uri, "crabdb://workspace/turns/", "", RESOURCE_TURN_TEMPLATE)?
    {
        return Ok((
            "application/json",
            pretty_json(&db.show_agent_turn(&turn_id)?)?,
        ));
    }
    if let Some(conflict_set_id) = template_uri_argument(
        uri,
        "crabdb://workspace/conflicts/",
        "",
        RESOURCE_CONFLICT_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.show_conflict(&conflict_set_id)?)?,
        ));
    }
    if let Some(approval_id) = template_uri_argument(
        uri,
        "crabdb://workspace/approvals/",
        "",
        RESOURCE_APPROVAL_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.show_agent_approval(&approval_id)?)?,
        ));
    }
    if let Some(run_id) =
        template_uri_argument(uri, "crabdb://workspace/runs/", "", RESOURCE_RUN_TEMPLATE)?
    {
        return Ok((
            "application/json",
            pretty_json(&db.show_agent_run_state(&run_id)?)?,
        ));
    }
    if let Some(span_id) =
        template_uri_argument(uri, "crabdb://workspace/spans/", "", RESOURCE_SPAN_TEMPLATE)?
    {
        return Ok((
            "application/json",
            pretty_json(&db.show_agent_trace_span(&span_id)?)?,
        ));
    }
    Err(Error::InvalidInput(format!(
        "MCP resource `{uri}` not found"
    )))
}

fn template_uri_argument(
    uri: &str,
    prefix: &str,
    suffix: &str,
    uri_template: &str,
) -> Result<Option<String>> {
    let Some(remainder) = uri.strip_prefix(prefix) else {
        return Ok(None);
    };
    if !remainder.ends_with(suffix) {
        return Ok(None);
    }
    let raw = &remainder[..remainder.len() - suffix.len()];
    if raw.is_empty() || raw.contains('/') {
        return Err(Error::InvalidInput(format!(
            "MCP resource `{uri}` does not match template `{uri_template}`"
        )));
    }
    let decoded = decode_uri_segment(raw)?;
    if decoded.trim().is_empty() || decoded.contains('/') {
        return Err(Error::InvalidInput(format!(
            "MCP resource `{uri}` does not match template `{uri_template}`"
        )));
    }
    Ok(Some(decoded))
}

fn decode_uri_segment(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(Error::InvalidInput(format!(
                    "invalid percent-encoding in URI segment `{value}`"
                )));
            }
            let high = hex_value(bytes[index + 1]).ok_or_else(|| {
                Error::InvalidInput(format!("invalid percent-encoding in URI segment `{value}`"))
            })?;
            let low = hex_value(bytes[index + 2]).ok_or_else(|| {
                Error::InvalidInput(format!("invalid percent-encoding in URI segment `{value}`"))
            })?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| Error::InvalidInput(format!("URI segment `{value}` is not valid UTF-8")))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
