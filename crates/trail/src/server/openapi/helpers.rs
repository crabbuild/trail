use serde_json::{json, Value};

pub(super) fn openapi_operation(
    operation_id: &str,
    summary: &str,
    description: &str,
    parameters: Vec<serde_json::Value>,
    request_schema: Option<&str>,
    authenticated: bool,
) -> Value {
    let mut operation = json!({
        "operationId": operation_id,
        "summary": summary,
        "description": description,
        "parameters": parameters,
        "responses": {
            "200": {
                "description": "Successful JSON response",
                "content": {
                    "application/json": {
                        "schema": { "$ref": "#/components/schemas/JsonValue" }
                    }
                }
            },
            "400": { "$ref": "#/components/responses/Error" },
            "401": { "$ref": "#/components/responses/Error" },
            "404": { "$ref": "#/components/responses/Error" }
        }
    });
    if let Some(schema) = request_schema {
        operation["requestBody"] = json!({
            "required": true,
            "content": {
                "application/json": {
                    "schema": { "$ref": format!("#/components/schemas/{schema}") }
                }
            }
        });
    }
    if !authenticated {
        operation["security"] = json!([]);
    }
    operation
}

pub(super) fn openapi_operation_with_response_schema(
    operation_id: &str,
    summary: &str,
    description: &str,
    parameters: Vec<serde_json::Value>,
    request_schema: Option<&str>,
    response_schema: &str,
    authenticated: bool,
) -> Value {
    let mut operation = openapi_operation(
        operation_id,
        summary,
        description,
        parameters,
        request_schema,
        authenticated,
    );
    operation["responses"]["200"]["content"]["application/json"]["schema"] =
        json!({ "$ref": format!("#/components/schemas/{response_schema}") });
    operation
}

pub(super) fn openapi_query(name: &str, value_type: &str) -> Value {
    openapi_parameter(name, "query", false, value_type)
}

pub(super) fn openapi_required_query(name: &str, value_type: &str) -> Value {
    openapi_parameter(name, "query", true, value_type)
}

pub(super) fn openapi_path_param(name: &str, value_type: &str) -> Value {
    openapi_parameter(name, "path", true, value_type)
}

fn openapi_parameter(name: &str, location: &str, required: bool, value_type: &str) -> Value {
    json!({
        "name": name,
        "in": location,
        "required": required,
        "schema": { "type": value_type }
    })
}
