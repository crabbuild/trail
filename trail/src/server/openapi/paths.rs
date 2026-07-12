use serde_json::{Map, Value};

mod agent_hooks;
mod collaboration;
mod core;
mod lanes;
mod turns;

use super::helpers::{
    openapi_operation, openapi_operation_with_response_schema, openapi_path_param, openapi_query,
    openapi_required_query,
};

pub(super) fn openapi_paths() -> Value {
    let mut paths = Map::new();
    append_paths(&mut paths, core::core_paths());
    append_paths(&mut paths, agent_hooks::agent_hook_paths());
    append_paths(&mut paths, collaboration::collaboration_paths());
    append_paths(&mut paths, lanes::lane_paths());
    append_paths(&mut paths, turns::turn_paths());
    Value::Object(paths)
}

fn append_paths(paths: &mut Map<String, Value>, group: Value) {
    let Value::Object(group) = group else {
        debug_assert!(false, "OpenAPI path groups must be JSON objects");
        return;
    };
    paths.extend(group);
}
