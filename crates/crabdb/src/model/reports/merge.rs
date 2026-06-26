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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<ConflictExplanation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictExplanation {
    pub merge: ConflictMergeContext,
    pub paths: Vec<ConflictPathExplanation>,
    pub recommendations: Vec<ConflictResolutionCandidate>,
    pub next_steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictMergeContext {
    pub merge_id: String,
    pub queue_id: Option<String>,
    pub source_ref: String,
    pub target_ref: String,
    pub base_change: ChangeId,
    pub target_change: ChangeId,
    pub source_change: ChangeId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictPathExplanation {
    pub path: String,
    pub summary: String,
    pub reason: String,
    pub target: Option<ConflictSideProvenance>,
    pub source: Option<ConflictSideProvenance>,
    pub lines: Vec<ConflictLineExplanation>,
    pub recommendation: ConflictResolutionCandidate,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictSideProvenance {
    pub side: String,
    pub change_id: ChangeId,
    pub kind: String,
    pub branch: String,
    pub actor_id: String,
    pub session_id: Option<String>,
    pub message: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictLineExplanation {
    pub line_id: String,
    pub base: Option<String>,
    pub target: Option<String>,
    pub source: Option<String>,
    pub target_change: Option<ConflictSideProvenance>,
    pub source_change: Option<ConflictSideProvenance>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictResolutionCandidate {
    pub resolution: String,
    pub confidence: String,
    pub reason: String,
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
