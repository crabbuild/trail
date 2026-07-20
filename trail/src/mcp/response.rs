use crate::{Error, Result};
use serde_json::json;
use serde_json::Value;

pub(crate) fn object_schema(properties: Value, required: Vec<&str>) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

pub(crate) fn tool_result<T: serde::Serialize>(value: T) -> Result<Value> {
    let structured = serde_json::to_value(value)?;
    Ok(json!({
        "resultType": "complete",
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&structured)?
            }
        ],
        "structuredContent": structured,
        "isError": false
    }))
}

pub(crate) fn tool_error_result(err: &Error) -> Value {
    let mut structured =
        serde_json::to_value(crate::model::StructuredErrorEnvelope::from_error(err))
            .unwrap_or_else(|_| json!({ "error": { "message": err.to_string() } }));
    if let Some(object) = structured.as_object_mut() {
        object.insert("message".to_string(), Value::String(err.to_string()));
    }
    json!({
        "resultType": "complete",
        "content": [
            {
                "type": "text",
                "text": err.to_string()
            }
        ],
        "structuredContent": structured,
        "isError": true
    })
}

pub(crate) fn pretty_json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).map_err(Error::from)
}

pub(crate) fn resource_error_response(id: Value, err: &Error) -> Value {
    let code = match err {
        Error::Json(_) => -32602,
        Error::InvalidInput(_) => -32002,
        _ => -32603,
    };
    json_rpc_error(id, code, &err.to_string())
}

pub(crate) fn prompt_error_response(id: Value, err: &Error) -> Value {
    let code = match err {
        Error::InvalidInput(_) | Error::Json(_) => -32602,
        _ => -32603,
    };
    json_rpc_error(id, code, &err.to_string())
}

pub(crate) fn completion_error_response(id: Value, err: &Error) -> Value {
    let code = match err {
        Error::InvalidInput(_) | Error::Json(_) => -32602,
        _ => -32603,
    };
    json_rpc_error(id, code, &err.to_string())
}

pub(crate) fn json_rpc_result(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

pub(crate) fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_initialization_conflict_tool_error_uses_shared_conflict_contract() {
        let error = Error::LaneInitializationConflict {
            lane: "agent-1".into(),
            existing_fingerprint: "sha256:existing".into(),
            requested_fingerprint: "sha256:requested".into(),
        };
        let value = tool_error_result(&error);

        assert_eq!(value["isError"], true);
        assert_eq!(
            value["structuredContent"]["error"]["code"],
            "LANE_INITIALIZATION_CONFLICT"
        );
        assert_eq!(value["structuredContent"]["error"]["status"], 409);
        assert_eq!(
            value["structuredContent"]["error"]["details"]["lane"],
            "agent-1"
        );
        assert_eq!(
            value["structuredContent"]["error"]["details"]["existing_fingerprint"],
            "sha256:existing"
        );
        assert_eq!(
            value["structuredContent"]["error"]["details"]["requested_fingerprint"],
            "sha256:requested"
        );
    }

    #[test]
    fn lane_initialization_in_progress_tool_error_uses_shared_conflict_contract() {
        let error = Error::LaneInitializationInProgress {
            lane: "agent-1".into(),
            initialization_id: "init_123".into(),
            owner_pid: 42,
            phase: crate::model::LaneInitializationPhase::RepairRequired,
            retry_command: "trail lane repair-initialization agent-1".into(),
        };
        let value = tool_error_result(&error);

        assert_eq!(value["isError"], true);
        assert_eq!(
            value["structuredContent"]["error"]["code"],
            "LANE_INITIALIZATION_IN_PROGRESS"
        );
        assert_eq!(value["structuredContent"]["error"]["status"], 409);
        assert_eq!(
            value["structuredContent"]["error"]["details"]["owner_pid"],
            42
        );
    }
}
