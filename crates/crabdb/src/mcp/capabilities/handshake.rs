use super::*;

pub(crate) fn initialize_result() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "serverInfo": {
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {
            "tools": {
                "listChanged": false
            },
            "resources": {
                "listChanged": false
            },
            "prompts": {
                "listChanged": false
            },
            "completions": {}
        }
    })
}

pub(crate) fn discover_result() -> Value {
    json!({
        "supportedVersions": [MCP_PROTOCOL_VERSION],
        "serverInfo": {
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {
            "tools": {
                "listChanged": false
            },
            "resources": {
                "listChanged": false
            },
            "prompts": {
                "listChanged": false
            },
            "completions": {}
        }
    })
}
