use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApiPatchRequest {
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
    pub(crate) edits: Option<Vec<crate::PatchEdit>>,
    pub(crate) files: Option<Vec<ApiPatchFile>>,
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
