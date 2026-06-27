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
            "title": "CrabDB Local API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Loopback JSON API for CrabDB editor integrations, lane runners, and local coordinators."
        },
        "servers": [
            {
                "url": "http://127.0.0.1:8765",
                "description": "Default local CrabDB daemon"
            }
        ],
        "security": [
            { "bearerAuth": [] },
            { "crabdbToken": [] }
        ],
        "paths": openapi_paths(),
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Send Authorization: Bearer <token>."
                },
                "crabdbToken": {
                    "type": "apiKey",
                    "in": "header",
                    "name": "X-CrabDB-Token"
                }
            },
            "responses": {
                "Error": {
                    "description": "CrabDB error response",
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
