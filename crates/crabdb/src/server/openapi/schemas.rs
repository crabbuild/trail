use serde_json::{Map, Value};

mod collaboration;
mod core;
mod lane;
mod patches;

pub(super) fn openapi_schemas() -> Value {
    let mut schemas = Map::new();
    append_schemas(&mut schemas, core::core_schemas());
    append_schemas(&mut schemas, lane::lane_schemas());
    append_schemas(&mut schemas, collaboration::collaboration_schemas());
    append_schemas(&mut schemas, patches::patch_schemas());
    Value::Object(schemas)
}

fn append_schemas(schemas: &mut Map<String, Value>, group: Value) {
    let Value::Object(group) = group else {
        debug_assert!(false, "OpenAPI schema groups must be JSON objects");
        return;
    };
    schemas.extend(group);
}
