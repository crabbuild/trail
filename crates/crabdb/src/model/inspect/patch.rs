#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchDocument {
    pub base_change: Option<String>,
    pub message: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub allow_ignored: bool,
    #[serde(default)]
    pub allow_stale: bool,
    pub edits: Vec<PatchEdit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PatchEdit {
    Write {
        path: String,
        content: String,
        #[serde(default)]
        executable: bool,
    },
    WriteBytes {
        path: String,
        bytes_hex: String,
        #[serde(default)]
        executable: bool,
    },
    ReplaceLine {
        path: String,
        line_id: String,
        #[serde(default)]
        expected_text: Option<String>,
        new_text: String,
    },
    Delete {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
}

pub(crate) fn validate_external_patch_edit_sources(
    label: &str,
    edits_len: usize,
    files_len: usize,
) -> crate::Result<()> {
    match (edits_len > 0, files_len > 0) {
        (true, false) | (false, true) => Ok(()),
        (false, false) => Err(crate::Error::InvalidInput(format!(
            "{label} requires at least one edit in `edits` or `files`"
        ))),
        (true, true) => Err(crate::Error::InvalidInput(format!(
            "{label} must use either `edits` or `files`, not both"
        ))),
    }
}
