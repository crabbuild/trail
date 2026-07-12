use serde_json::{Map, Value};

mod agent_hooks;
mod collaboration;
mod core;
mod lane;
mod patches;

pub(super) fn openapi_schemas() -> Value {
    let mut schemas = Map::new();
    append_schemas(&mut schemas, core::core_schemas());
    append_schemas(&mut schemas, agent_hooks::agent_hook_schemas());
    append_schemas(&mut schemas, lane::lane_schemas());
    append_schemas(&mut schemas, collaboration::collaboration_schemas());
    append_schemas(&mut schemas, patches::patch_schemas());
    mark_request_schemas_strict(&mut schemas);
    Value::Object(schemas)
}

fn append_schemas(schemas: &mut Map<String, Value>, group: Value) {
    let Value::Object(group) = group else {
        debug_assert!(false, "OpenAPI schema groups must be JSON objects");
        return;
    };
    schemas.extend(group);
}

fn mark_request_schemas_strict(schemas: &mut Map<String, Value>) {
    for (name, schema) in schemas {
        if !name.ends_with("Request") {
            continue;
        }
        let Value::Object(schema) = schema else {
            continue;
        };
        if schema.get("type").and_then(Value::as_str) == Some("object")
            && !schema.contains_key("additionalProperties")
        {
            schema.insert("additionalProperties".to_string(), Value::Bool(false));
        }
    }
}
