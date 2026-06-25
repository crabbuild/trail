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
