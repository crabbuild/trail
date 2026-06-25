#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeReport {
    pub operation: ChangeId,
    pub source_ref: String,
    pub target_ref: String,
    pub root_id: ObjectId,
    #[serde(default)]
    pub dry_run: bool,
    pub changed_paths: Vec<FileDiffSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueEntry {
    pub queue_id: String,
    pub source_ref: String,
    pub target_ref: String,
    pub status: String,
    pub priority: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueAddReport {
    pub entry: MergeQueueEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueRemoveReport {
    pub entry: MergeQueueEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueRunReport {
    pub processed: Vec<MergeQueueRunItem>,
    pub stopped_on_conflict: bool,
    pub stopped_on_failure: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueRunItem {
    pub queue_id: String,
    pub source_ref: String,
    pub target_ref: String,
    pub status: String,
    pub operation: Option<ChangeId>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictSetSummary {
    pub conflict_set_id: String,
    pub merge_id: Option<String>,
    pub source_ref: Option<String>,
    pub target_ref: Option<String>,
    pub status: String,
    pub details: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConflictManualResolution {
    #[serde(default)]
    pub files: std::collections::BTreeMap<String, ConflictManualFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConflictManualFile {
    Text(String),
    Spec(ConflictManualFileSpec),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConflictManualFileSpec {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub delete: bool,
    #[serde(default)]
    pub executable: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictResolveReport {
    pub conflict_set_id: String,
    pub resolution: String,
    pub operation: ChangeId,
    pub target_ref: String,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}
