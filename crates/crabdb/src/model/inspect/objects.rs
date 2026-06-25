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
