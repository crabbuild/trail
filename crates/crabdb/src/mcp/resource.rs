use serde_json::{json, Value};

use crate::{CrabDb, Error, Result};

use super::{response::pretty_json, types::*, utils::from_arguments};

pub(crate) fn handle_resource_read(db: &mut CrabDb, params: Value) -> Result<Value> {
    let args: ResourceReadArgs = from_arguments(params)?;
    let label = format!("resources/read {}", args.uri);
    db.enforce_read_only_mcp_call(&label, |db| resource_read_response(db, args))
}

fn resource_read_response(db: &mut CrabDb, args: ResourceReadArgs) -> Result<Value> {
    let (mime_type, text) = match args.uri.as_str() {
        RESOURCE_STATUS => (
            "application/json",
            pretty_json(&db.status_read_only(None)?)?,
        ),
        RESOURCE_DOCTOR => ("application/json", pretty_json(&db.doctor()?)?),
        RESOURCE_LANES => ("application/json", pretty_json(&db.list_lanes()?)?),
        RESOURCE_MERGE_QUEUE => ("application/json", pretty_json(&db.list_merge_queue()?)?),
        RESOURCE_CONFLICTS => ("application/json", pretty_json(&db.list_conflicts()?)?),
        RESOURCE_OPENAPI => (
            "application/json",
            serde_json::to_string_pretty(&crate::server::openapi_spec())?,
        ),
        RESOURCE_AGENT_INBOX => ("application/json", pretty_json(&db.agent_inbox()?)?),
        RESOURCE_AGENT_LATEST_SUMMARY => (
            "application/json",
            pretty_json(&db.agent_summary("latest")?)?,
        ),
        RESOURCE_AGENT_LATEST_DIAGNOSE => (
            "application/json",
            pretty_json(&db.agent_diagnose("latest")?)?,
        ),
        RESOURCE_AGENT_LATEST_REVIEW => (
            "application/json",
            pretty_json(&db.agent_review("latest")?)?,
        ),
        RESOURCE_AGENT_LATEST_CHANGES => (
            "application/json",
            pretty_json(&db.agent_changes("latest", false)?)?,
        ),
        RESOURCE_AGENT_LATEST_TIMELINE => (
            "application/json",
            pretty_json(&db.agent_timeline("latest", false)?)?,
        ),
        RESOURCE_AGENT_LATEST_FILES => {
            ("application/json", pretty_json(&db.agent_files("latest")?)?)
        }
        RESOURCE_AGENT_LATEST_FOCUS => (
            "application/json",
            pretty_json(&db.agent_focus("latest", None, false)?)?,
        ),
        RESOURCE_AGENT_LATEST_RECEIPT => (
            "application/json",
            pretty_json(&db.agent_receipt("latest")?)?,
        ),
        RESOURCE_AGENT_LATEST_PR => (
            "application/json",
            pretty_json(&db.agent_pr_draft("latest")?)?,
        ),
        RESOURCE_USER_GUIDE => ("text/markdown", USER_GUIDE_MD.to_string()),
        RESOURCE_LANE_WORKFLOWS => ("text/markdown", LANE_WORKFLOWS_MD.to_string()),
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
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/summary",
        RESOURCE_AGENT_SUMMARY_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_summary(&selector)?)?,
        ));
    }
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/diagnose",
        RESOURCE_AGENT_DIAGNOSE_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_diagnose(&selector)?)?,
        ));
    }
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/review",
        RESOURCE_AGENT_REVIEW_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_review(&selector)?)?,
        ));
    }
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/changes",
        RESOURCE_AGENT_CHANGES_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_changes(&selector, false)?)?,
        ));
    }
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/timeline",
        RESOURCE_AGENT_TIMELINE_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_timeline(&selector, false)?)?,
        ));
    }
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/files",
        RESOURCE_AGENT_FILES_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_files(&selector)?)?,
        ));
    }
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/report",
        RESOURCE_AGENT_REPORT_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_report(&selector)?)?,
        ));
    }
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/receipt",
        RESOURCE_AGENT_RECEIPT_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_receipt(&selector)?)?,
        ));
    }
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/pr",
        RESOURCE_AGENT_PR_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_pr_draft(&selector)?)?,
        ));
    }
    if let Some(selector) = template_uri_argument(
        uri,
        "crabdb://workspace/agent-tasks/",
        "/focus",
        RESOURCE_AGENT_FOCUS_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_focus(&selector, None, false)?)?,
        ));
    }
    if let Some(lane) = template_uri_argument(
        uri,
        "crabdb://workspace/lanes/",
        "/status",
        RESOURCE_LANE_STATUS_TEMPLATE,
    )? {
        return Ok(("application/json", pretty_json(&db.lane_status(&lane)?)?));
    }
    if let Some(lane) = template_uri_argument(
        uri,
        "crabdb://workspace/lanes/",
        "/review",
        RESOURCE_LANE_REVIEW_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.lane_review_packet(&lane, 50)?)?,
        ));
    }
    if let Some(lane) = template_uri_argument(
        uri,
        "crabdb://workspace/lanes/",
        "/contribution",
        RESOURCE_LANE_CONTRIBUTION_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.lane_contribution(&lane, 50)?)?,
        ));
    }
    if let Some(lane) = template_uri_argument(
        uri,
        "crabdb://workspace/lanes/",
        "/gates",
        RESOURCE_LANE_GATES_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.lane_gate_history(&lane, None, 50)?)?,
        ));
    }
    if let Some(lane) = template_uri_argument(
        uri,
        "crabdb://workspace/lanes/",
        "/readiness",
        RESOURCE_LANE_READINESS_TEMPLATE,
    )? {
        return Ok(("application/json", pretty_json(&db.lane_readiness(&lane)?)?));
    }
    if let Some(lane) = template_uri_argument(
        uri,
        "crabdb://workspace/lanes/",
        "/handoff",
        RESOURCE_LANE_HANDOFF_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.lane_handoff(&lane, 50)?)?,
        ));
    }
    if let Some(lane) = template_uri_argument(
        uri,
        "crabdb://workspace/lanes/",
        "/diff",
        RESOURCE_LANE_DIFF_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.diff_lane_with_options(&lane, false, false)?)?,
        ));
    }
    if let Some(lane) =
        template_uri_argument(uri, "crabdb://workspace/lanes/", "", RESOURCE_LANE_TEMPLATE)?
    {
        return Ok(("application/json", pretty_json(&db.lane_details(&lane)?)?));
    }
    if let Some(session_id) = template_uri_argument(
        uri,
        "crabdb://workspace/sessions/",
        "",
        RESOURCE_SESSION_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.show_lane_session(&session_id)?)?,
        ));
    }
    if let Some(turn_id) =
        template_uri_argument(uri, "crabdb://workspace/turns/", "", RESOURCE_TURN_TEMPLATE)?
    {
        return Ok((
            "application/json",
            pretty_json(&db.show_lane_turn(&turn_id)?)?,
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
            pretty_json(&db.show_lane_approval(&approval_id)?)?,
        ));
    }
    if let Some(run_id) =
        template_uri_argument(uri, "crabdb://workspace/runs/", "", RESOURCE_RUN_TEMPLATE)?
    {
        return Ok((
            "application/json",
            pretty_json(&db.show_lane_run_state(&run_id)?)?,
        ));
    }
    if let Some(span_id) =
        template_uri_argument(uri, "crabdb://workspace/spans/", "", RESOURCE_SPAN_TEMPLATE)?
    {
        return Ok((
            "application/json",
            pretty_json(&db.show_lane_trace_span(&span_id)?)?,
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
