use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{CrabDb, Error, Result};

use super::{tools::tool_is_read_only, types::*, utils::from_arguments};

mod collaboration;
mod core;
mod lane;
mod merge;
mod turns;

pub(crate) fn handle_tool_call(db: &mut CrabDb, params: Value) -> Result<Value> {
    let call: ToolCall = serde_json::from_value(params)?;
    if tool_is_read_only(&call.name) {
        return db.enforce_read_only_mcp_call(&call.name, |db| dispatch_tool_call(db, &call));
    }
    dispatch_tool_call(db, &call)
}

fn dispatch_tool_call(db: &mut CrabDb, call: &ToolCall) -> Result<Value> {
    if let Some(value) = core::handle(db, &call.name, &call.arguments)? {
        return Ok(value);
    }
    if let Some(value) = lane::handle(db, &call.name, &call.arguments)? {
        return Ok(value);
    }
    if let Some(value) = collaboration::handle(db, &call.name, &call.arguments)? {
        return Ok(value);
    }
    if let Some(value) = merge::handle(db, &call.name, &call.arguments)? {
        return Ok(value);
    }
    if let Some(value) = turns::handle(db, &call.name, &call.arguments)? {
        return Ok(value);
    }
    Err(Error::InvalidInput(format!(
        "unknown MCP tool `{}`",
        call.name
    )))
}

pub(super) fn parse_args<T: DeserializeOwned>(arguments: &Value) -> Result<T> {
    from_arguments(arguments.clone())
}
