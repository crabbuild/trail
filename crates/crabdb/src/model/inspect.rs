#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub lease_id: String,
    pub agent_id: String,
    pub ref_name: String,
    pub path: Option<String>,
    pub file_id: Option<String>,
    pub mode: String,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentClaimReport {
    pub agent_id: String,
    pub ref_name: String,
    pub path: String,
    pub mode: String,
    pub ttl_secs: u64,
    pub claimed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<LeaseRecord>,
    #[serde(default)]
    pub conflicts: Vec<LeaseRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseAcquireReport {
    pub lease: LeaseRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseReleaseReport {
    pub lease_id: String,
    pub released: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatchDocument {
    pub base_change: Option<String>,
    pub message: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub allow_ignored: bool,
    pub edits: Vec<PatchEdit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffSummary {
    pub from: String,
    pub to: String,
    pub files: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileDiffSummary {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: FileChangeKind,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub line_changes: Vec<LineChange>,
    pub patch: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusReport {
    pub branch: String,
    pub head: RefRecord,
    pub worktree_state: WorktreeState,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorktreeState {
    Clean,
    DirtyTracked,
    DirtyUntracked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub change_id: ChangeId,
    pub kind: OperationKind,
    pub branch: String,
    pub actor_id: String,
    pub message: Option<String>,
    pub created_at: i64,
    pub path_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WhyResult {
    pub path: String,
    pub line_number: u64,
    pub file_id: FileId,
    pub line_id: LineId,
    pub current_text: String,
    pub introduced_by: ChangeId,
    pub last_content_change: ChangeId,
    pub last_move_change: Option<ChangeId>,
    pub history: Vec<LineHistoryEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineHistoryEntry {
    pub change_id: ChangeId,
    pub path: String,
    pub line_number: Option<u64>,
    pub kind: LineChangeKind,
    pub text_hash: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileHistoryEntry {
    pub file_id: String,
    pub change_id: ChangeId,
    pub path: String,
    pub old_path: Option<String>,
    pub kind: FileChangeKind,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryResult {
    pub selector: String,
    pub file_history: Vec<FileHistoryEntry>,
    pub line_history: Vec<LineHistoryEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub object_id: ObjectId,
    pub kind: String,
    pub version: u16,
    pub size_bytes: u64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectInspectReport {
    pub info: ObjectInfo,
    pub summary: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootInspectReport {
    pub root_id: ObjectId,
    pub root: WorktreeRoot,
    pub files: Vec<RootFileInspect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootFileInspect {
    pub path: String,
    pub file_id: String,
    pub kind: FileKind,
    pub mode: u32,
    pub executable: bool,
    pub size_bytes: u64,
    pub content_hash: String,
    pub content_object: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextInspectReport {
    pub text_id: ObjectId,
    pub content: TextContent,
    pub lines: Vec<TextLineInspect>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextLineInspect {
    pub line_number: u64,
    pub line_id: String,
    pub text_hash: String,
    pub text: String,
    pub newline: NewlineKind,
    pub introduced_by: ChangeId,
    pub last_content_change: ChangeId,
    pub last_move_change: Option<ChangeId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapRangeReport {
    pub map_id: String,
    pub map_type: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub entries: Vec<MapEntryInspect>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapDiffReport {
    pub left_map_id: String,
    pub right_map_id: String,
    pub map_type: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub changes: Vec<MapDiffInspect>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapEntryInspect {
    pub key: MapKeyInspect,
    pub value: MapValueInspect,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapDiffInspect {
    pub kind: String,
    pub key: MapKeyInspect,
    pub old_value: Option<MapValueInspect>,
    pub new_value: Option<MapValueInspect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapKeyInspect {
    pub hex: String,
    pub text: Option<String>,
    pub summary: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapValueInspect {
    pub bytes: usize,
    pub hex_preview: String,
    pub truncated: bool,
    pub text: Option<String>,
    pub summary: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationShow {
    pub operation: Operation,
    pub changed_paths: Vec<FileDiffSummary>,
    pub messages: Vec<Message>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShowResult {
    Operation { value: OperationShow },
    Message { value: Message },
    Ref { value: RefRecord },
    Agent { value: AgentBranch },
    Object { value: ObjectInfo },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeFromResult {
    pub selector: String,
    pub operations: Vec<CodeFromOperation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeFromOperation {
    pub change_id: ChangeId,
    pub kind: OperationKind,
    pub branch: String,
    pub actor_id: String,
    pub session_id: Option<String>,
    pub message: Option<String>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub version: u16,
    pub id: AnchorId,
    pub label: String,
    pub file_id: FileId,
    pub line_id: LineId,
    pub created_path: String,
    pub created_line: u64,
    pub created_change: ChangeId,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorCreateReport {
    pub anchor: Anchor,
    pub object_id: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorResolveReport {
    pub anchor: Anchor,
    pub branch: String,
    pub status: String,
    pub path: Option<String>,
    pub line_number: Option<u64>,
    pub text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorDeleteReport {
    pub anchor_id: AnchorId,
}
