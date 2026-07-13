use serde_json::{json, Value};

mod helpers;
mod paths;
mod schemas;

use paths::openapi_paths;
use schemas::openapi_schemas;

pub fn openapi_spec() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Trail Local API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Loopback JSON API for Trail editor integrations, lane runners, and local coordinators."
        },
        "servers": [
            {
                "url": "http://127.0.0.1:8765",
                "description": "Default local Trail daemon"
            }
        ],
        "security": [
            { "bearerAuth": [] },
            { "trailToken": [] }
        ],
        "paths": openapi_paths(),
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Send Authorization: Bearer <token>."
                },
                "trailToken": {
                    "type": "apiKey",
                    "in": "header",
                    "name": "X-Trail-Token"
                }
            },
            "responses": {
                "Error": {
                    "description": "Trail error response",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": "#/components/schemas/ErrorBody" }
                        }
                    }
                }
            },
            "schemas": openapi_schemas()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_and_record_reports_document_path_index_metrics() {
        let spec = openapi_spec();
        let schemas = &spec["components"]["schemas"];
        assert_eq!(
            schemas["LanePatchReport"]["properties"]["path_index"]["$ref"],
            "#/components/schemas/PathIndexMetricsReport"
        );
        assert_eq!(
            schemas["LaneRecordReport"]["properties"]["path_index"]["$ref"],
            "#/components/schemas/PathIndexMetricsReport"
        );
        assert_eq!(
            schemas["PathIndexMetricsReport"]["required"],
            serde_json::json!([
                "mode",
                "lookup_count",
                "full_root_path_load_count",
                "full_filesystem_path_scan_count"
            ])
        );
        assert_eq!(
            spec["paths"]["/v1/lanes/{lane_or_id}/patches"]["post"]["responses"]["200"]["content"]
                ["application/json"]["schema"]["$ref"],
            "#/components/schemas/LanePatchReport"
        );
        assert_eq!(
            spec["paths"]["/v1/lane/turns/{turn_id}/patches"]["post"]["responses"]["200"]
                ["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/LanePatchReport"
        );
    }
}
