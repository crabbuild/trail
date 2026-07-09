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
pub struct MergeQueueExplainReport {
    pub entry: MergeQueueEntry,
    pub readiness: Option<LaneReadinessReport>,
    pub dry_run: Option<MergeReport>,
    pub blockers: Vec<LaneReadinessIssue>,
    pub warnings: Vec<LaneReadinessIssue>,
    pub error: Option<String>,
    pub next_steps: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_root: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_root: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_root: Option<ObjectId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictPathExplanation {
    pub path: String,
    pub conflict_class: String,
    pub summary: String,
    pub reason: String,
    pub target: Option<ConflictSideProvenance>,
    pub source: Option<ConflictSideProvenance>,
    pub lines: Vec<ConflictLineExplanation>,
    pub recommendation: ConflictResolutionCandidate,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_resolutions: Vec<ConflictKnownResolution>,
    #[serde(default, skip_serializing)]
    pub signature: String,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictKnownResolution {
    pub resolution: String,
    pub confidence: String,
    pub reason: String,
    pub conflict_set_id: String,
    pub operation: ChangeId,
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConflictManualResolution {
    #[serde(default)]
    pub files: std::collections::BTreeMap<String, ConflictManualFile>,
}

#[derive(Clone, Debug, Serialize)]
pub enum ConflictManualFile {
    Text(String),
    Spec(ConflictManualFileSpec),
}

impl<'de> serde::Deserialize<'de> for ConflictManualFile {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(text) => Ok(Self::Text(text)),
            serde_json::Value::Object(_) => {
                serde_json::from_value::<ConflictManualFileSpec>(value)
                    .map(Self::Spec)
                    .map_err(serde::de::Error::custom)
            }
            other => Err(serde::de::Error::custom(format!(
                "invalid manual conflict file value {other}; expected string content or object spec"
            ))),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
