use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::{Result, Trail};

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

const MAX_MCP_STDIO_LINE_BYTES: usize = 16 * 1024 * 1024;

pub fn serve_stdio<R: BufRead, W: Write>(db: &mut Trail, input: R, output: &mut W) -> Result<()> {
    serve_stdio_with_line_limit(db, input, output, MAX_MCP_STDIO_LINE_BYTES)
}

fn serve_stdio_with_line_limit<R: BufRead, W: Write>(
    db: &mut Trail,
    input: R,
    output: &mut W,
    max_line_bytes: usize,
) -> Result<()> {
    let mut input = input;
    while let Some(line) = read_mcp_stdio_line_limited(&mut input, max_line_bytes)? {
        let line = match line {
            McpStdioLine::Text(line) => line,
            McpStdioLine::ParseError(message) => {
                let response = json_rpc_error(Value::Null, -32700, &message);
                serde_json::to_writer(&mut *output, &response)?;
                output.write_all(b"\n")?;
                output.flush()?;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(err) => {
                let response = json_rpc_error(Value::Null, -32700, &format!("parse error: {err}"));
                serde_json::to_writer(&mut *output, &response)?;
                output.write_all(b"\n")?;
                output.flush()?;
                continue;
            }
        };
        if let Some(response) = handle_json_rpc(db, request) {
            serde_json::to_writer(&mut *output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

enum McpStdioLine {
    Text(String),
    ParseError(String),
}

fn read_mcp_stdio_line_limited<R: BufRead>(
    input: &mut R,
    max_line_bytes: usize,
) -> Result<Option<McpStdioLine>> {
    let mut line = Vec::new();
    let mut bytes_read = 0usize;
    let mut oversized = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            if bytes_read == 0 {
                return Ok(None);
            }
            break;
        }
        let take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |idx| idx + 1);
        if !oversized {
            let allowed = max_line_bytes.saturating_sub(line.len());
            if take > allowed {
                line.extend_from_slice(&buffer[..allowed]);
                oversized = true;
            } else {
                line.extend_from_slice(&buffer[..take]);
            }
        }
        let ended_line = buffer[take - 1] == b'\n';
        bytes_read = bytes_read.saturating_add(take);
        input.consume(take);
        if ended_line {
            break;
        }
    }
    if oversized || bytes_read > max_line_bytes {
        return Ok(Some(McpStdioLine::ParseError(format!(
            "MCP stdio message is {bytes_read} bytes, exceeding limit {max_line_bytes}"
        ))));
    }
    match String::from_utf8(line) {
        Ok(line) => Ok(Some(McpStdioLine::Text(line))),
        Err(err) => Ok(Some(McpStdioLine::ParseError(format!(
            "MCP stdio message must be valid UTF-8: {err}"
        )))),
    }
}

pub fn handle_json_rpc(db: &mut Trail, request: Value) -> Option<Value> {
    let Some(request) = request.as_object() else {
        return Some(json_rpc_error(
            Value::Null,
            -32600,
            "invalid JSON-RPC request",
        ));
    };
    let id = request.get("id").cloned();
    if id.as_ref().is_some_and(|id| !is_valid_json_rpc_id(id)) {
        return Some(json_rpc_error(
            Value::Null,
            -32600,
            "invalid JSON-RPC request",
        ));
    }
    if let Some(unknown) = unknown_json_rpc_request_field(request) {
        return Some(json_rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            &format!("invalid JSON-RPC request: unknown field `{unknown}`"),
        ));
    }
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(json_rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "invalid JSON-RPC request",
        ));
    }
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Some(json_rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "invalid JSON-RPC request",
        ));
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    if !is_valid_json_rpc_params(&params) {
        return Some(json_rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "invalid JSON-RPC request",
        ));
    }

    id.as_ref()?;
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

fn is_valid_json_rpc_id(id: &Value) -> bool {
    id.is_null() || id.is_string() || id.is_number()
}

fn is_valid_json_rpc_params(params: &Value) -> bool {
    params.is_null() || params.is_object() || params.is_array()
}

fn unknown_json_rpc_request_field(request: &serde_json::Map<String, Value>) -> Option<&str> {
    request
        .keys()
        .find(|field| {
            !matches!(
                field.as_str(),
                "jsonrpc" | "id" | "method" | "params" | "_meta"
            )
        })
        .map(String::as_str)
}

fn handle_tool_call_response(db: &mut Trail, id: Value, params: Value) -> Value {
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

#[cfg(test)]
mod tests {
    use super::super::{types::*, utils::from_arguments};
    use super::*;
    use crate::InitImportMode;
    use std::io::Cursor;

    #[test]
    fn stdio_rejects_oversized_line_and_continues() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let input = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"{}\"}}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}}\n",
            "x".repeat(80)
        );
        let mut output = Vec::new();

        serve_stdio_with_line_limit(&mut db, Cursor::new(input), &mut output, 64).unwrap();

        let output = String::from_utf8(output).unwrap();
        let responses = output
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["error"]["code"], -32700);
        assert!(responses[0]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("exceeding limit")));
        assert_eq!(responses[1]["id"], 2);
        assert_eq!(responses[1]["result"], json!({}));
    }

    #[test]
    fn stdio_rejects_non_utf8_line_and_continues() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mut input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"".to_vec();
        input.push(0xff);
        input.extend_from_slice(b"\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n");
        let mut output = Vec::new();

        serve_stdio_with_line_limit(&mut db, Cursor::new(input), &mut output, 1024).unwrap();

        let output = String::from_utf8(output).unwrap();
        let responses = output
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["error"]["code"], -32700);
        assert!(responses[0]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("valid UTF-8")));
        assert_eq!(responses[1]["id"], 2);
        assert_eq!(responses[1]["result"], json!({}));
    }

    #[test]
    fn json_rpc_request_rejects_unknown_envelope_fields_but_accepts_meta() {
        let (_temp, mut db) = test_db();
        let accepted = handle_json_rpc(
            &mut db,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "ping",
                "_meta": { "client": "test" }
            }),
        )
        .unwrap();
        assert_eq!(accepted["result"], json!({}));

        let rejected = handle_json_rpc(
            &mut db,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "ping",
                "unexpected": true
            }),
        )
        .unwrap();
        assert_eq!(rejected["error"]["code"], -32600);
        assert!(rejected["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unknown field `unexpected`")));
    }

    #[test]
    fn tools_call_rejects_unknown_protocol_param_fields_but_accepts_meta() {
        let (_temp, mut db) = test_db();
        let accepted = handle_json_rpc(
            &mut db,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "trail.status",
                    "arguments": {},
                    "_meta": { "client": "test" }
                }
            }),
        )
        .unwrap();
        assert_eq!(accepted["result"]["isError"], false);

        let rejected = handle_json_rpc(
            &mut db,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "trail.status",
                    "arguments": {},
                    "unexpected": true
                }
            }),
        )
        .unwrap();
        assert_eq!(rejected["result"]["isError"], true);
        assert!(rejected["result"]["structuredContent"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unknown field")));
    }

    #[test]
    fn resource_read_rejects_unknown_protocol_param_fields() {
        let (_temp, mut db) = test_db();
        let response = handle_json_rpc(
            &mut db,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "resources/read",
                "params": {
                    "uri": RESOURCE_STATUS,
                    "unexpected": true
                }
            }),
        )
        .unwrap();

        assert_eq!(response["error"]["code"], -32602);
        assert!(response["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unknown field")));
    }

    #[test]
    fn mcp_protocol_param_types_reject_unknown_fields_and_accept_meta() {
        from_arguments::<PromptGetArgs>(json!({
            "name": PROMPT_LANE_TASK,
            "arguments": { "lane": "doc-bot", "task": "update docs" },
            "_meta": { "client": "test" }
        }))
        .unwrap();

        assert_unknown_field(from_arguments::<PromptGetArgs>(json!({
            "name": PROMPT_LANE_TASK,
            "arguments": {},
            "unexpected": true
        })));
        assert_unknown_field(from_arguments::<CompletionArgs>(json!({
            "ref": {
                "type": "ref/prompt",
                "name": PROMPT_LANE_TASK,
                "unexpected": true
            },
            "argument": { "name": "lane" }
        })));
        assert_unknown_field(from_arguments::<CompletionArgs>(json!({
            "ref": {
                "type": "ref/prompt",
                "name": PROMPT_LANE_TASK
            },
            "argument": {
                "name": "lane",
                "unexpected": true
            }
        })));
    }

    fn assert_unknown_field<T>(result: Result<T>) {
        match result {
            Ok(_) => panic!("expected unknown field error"),
            Err(err) => assert!(
                err.to_string().contains("unknown field"),
                "expected unknown field error, got {err}"
            ),
        }
    }

    fn test_db() -> (tempfile::TempDir, Trail) {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        (temp, db)
    }
}
