use serde::Deserialize;
use serde_json::Value;

use crate::PatchEdit;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BeginTurnArgs {
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) session_title: Option<String>,
    #[serde(default)]
    pub(crate) base_change: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TurnIdArgs {
    pub(crate) turn_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddMessageArgs {
    pub(crate) turn_id: String,
    pub(crate) role: String,
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddEventArgs {
    pub(crate) turn_id: String,
    #[serde(alias = "type")]
    pub(crate) event_type: String,
    #[serde(default)]
    pub(crate) payload: Option<Value>,
    #[serde(default)]
    pub(crate) change_id: Option<String>,
    #[serde(default)]
    pub(crate) message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventListArgs {
    #[serde(default)]
    pub(crate) lane: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
    #[serde(default, alias = "type")]
    pub(crate) event_type: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanStartArgs {
    pub(crate) turn_id: String,
    #[serde(alias = "type")]
    pub(crate) span_type: String,
    pub(crate) name: String,
    #[serde(default, alias = "parent_span_id")]
    pub(crate) parent: Option<String>,
    #[serde(default, alias = "trace_id")]
    pub(crate) trace: Option<String>,
    #[serde(default, alias = "attributes_json")]
    pub(crate) attributes: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanEndArgs {
    pub(crate) span_id: String,
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
    #[serde(default, alias = "result_json")]
    pub(crate) result: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanListArgs {
    #[serde(default)]
    pub(crate) lane: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
    #[serde(default, alias = "trace")]
    pub(crate) trace_id: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanSummaryArgs {
    #[serde(default)]
    pub(crate) lane: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
    #[serde(default, alias = "trace")]
    pub(crate) trace_id: Option<String>,
    #[serde(default, alias = "slowest_limit")]
    pub(crate) slowest: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanShowArgs {
    pub(crate) span_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EndTurnArgs {
    pub(crate) turn_id: String,
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyPatchArgs {
    pub(crate) turn_id: String,
    #[serde(default)]
    pub(crate) base_change: Option<String>,
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) allow_ignored: bool,
    #[serde(default)]
    pub(crate) allow_stale: bool,
    #[serde(default)]
    pub(crate) edits: Vec<PatchEdit>,
    #[serde(default)]
    pub(crate) files: Vec<ApiPatchFile>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ApiPatchFile {
    AddText {
        path: String,
        content: String,
        #[serde(default)]
        executable: bool,
    },
    ModifyText {
        path: String,
        edits: Vec<ApiTextEdit>,
    },
    WriteBytes {
        path: String,
        bytes_hex: String,
        #[serde(default)]
        executable: bool,
    },
    Delete {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ApiTextEdit {
    ModifyLine {
        line_id: String,
        #[serde(default)]
        expected_text: Option<String>,
        new_text: String,
    },
}

pub(crate) fn default_completed_status() -> String {
    "completed".to_string()
}
