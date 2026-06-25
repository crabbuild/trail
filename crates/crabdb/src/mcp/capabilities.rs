use serde_json::{json, Value};

mod handshake;
mod prompts;
mod resources;

use super::{tools::tools, types::*};

pub(crate) use handshake::*;
pub(crate) use prompts::*;
pub(crate) use resources::*;

pub(crate) fn tools_list_result() -> Value {
    json!({
        "resultType": "complete",
        "tools": tools(),
        "ttlMs": 300000,
        "cacheScope": "public"
    })
}
