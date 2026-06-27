use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::{CrabDb, Result};

use super::{
    audit::McpMutationAudit,
    capabilities::{
        discover_result, initialize_result, prompts_list_result, resources_list_result,
        resources_templates_list_result, tools_list_result,
    },
    completion::handle_completion_complete,
    prompt::handle_prompt_get,
    resource::handle_resource_read,
    response::{
        completion_error_response, json_rpc_error, json_rpc_result, prompt_error_response,
        resource_error_response, tool_error_result,
    },
    tool_call::handle_tool_call,
};

pub fn serve_stdio<R: BufRead, W: Write>(db: &mut CrabDb, input: R, output: &mut W) -> Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = serde_json::from_str(&line)?;
        if let Some(response) = handle_json_rpc(db, request) {
            serde_json::to_writer(&mut *output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

pub fn handle_json_rpc(db: &mut CrabDb, request: Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return id.map(|id| json_rpc_error(id, -32600, "invalid JSON-RPC request"));
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    if id.is_none() {
        return None;
    }
    let id = id.unwrap();

    match method {
        "initialize" => Some(json_rpc_result(id, initialize_result())),
        "server/discover" => Some(json_rpc_result(id, discover_result())),
        "ping" => Some(json_rpc_result(id, json!({}))),
        "tools/list" => Some(json_rpc_result(id, tools_list_result())),
        "resources/list" => Some(json_rpc_result(id, resources_list_result())),
        "resources/templates/list" => Some(json_rpc_result(id, resources_templates_list_result())),
        "prompts/list" => Some(json_rpc_result(id, prompts_list_result())),
        "prompts/get" => Some(match handle_prompt_get(params) {
            Ok(result) => json_rpc_result(id, result),
            Err(err) => prompt_error_response(id, &err),
        }),
        "completion/complete" => Some(match handle_completion_complete(db, params) {
            Ok(result) => json_rpc_result(id, result),
            Err(err) => completion_error_response(id, &err),
        }),
        "resources/read" => Some(match handle_resource_read(db, params) {
            Ok(result) => json_rpc_result(id, result),
            Err(err) => resource_error_response(id, &err),
        }),
        "tools/call" => Some(handle_tool_call_response(db, id, params)),
        _ => Some(json_rpc_error(
            id,
            -32601,
            &format!("method not found: {method}"),
        )),
    }
}

fn handle_tool_call_response(db: &mut CrabDb, id: Value, params: Value) -> Value {
    let audit = McpMutationAudit::from_tool_call_params(&params);
    let result = match handle_tool_call(db, params) {
        Ok(result) => result,
        Err(err) => tool_error_result(&err),
    };
    if let Some(audit) = audit {
        audit.record(db, &result);
    }
    json_rpc_result(id, result)
}
