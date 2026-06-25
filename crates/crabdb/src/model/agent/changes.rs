#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub file_id: Option<FileId>,
    pub kind: FileChangeKind,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub line_changes: Vec<LineChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineChange {
    pub line_id: LineId,
    pub kind: LineChangeKind,
    pub old_line_number: Option<u64>,
    pub new_line_number: Option<u64>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LineChangeKind {
    Added,
    Modified,
    Deleted,
    Moved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RefRecord {
    pub name: String,
    pub change_id: ChangeId,
    pub root_id: ObjectId,
    pub operation_id: ObjectId,
    pub generation: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub version: u16,
    pub id: MessageId,
    pub role: String,
    pub body: String,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub change_id: Option<ChangeId>,
    pub created_at: i64,
}
