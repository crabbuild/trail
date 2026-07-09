#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredObject {
    pub id: ObjectId,
    pub kind: String,
    pub version: u16,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorktreeRoot {
    pub version: u16,
    pub path_map_root: Option<String>,
    pub file_index_map_root: Option<String>,
    pub file_count: u64,
    pub total_text_bytes: u64,
    pub created_by: ChangeId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub file_id: FileId,
    pub kind: FileKind,
    pub mode: u32,
    pub executable: bool,
    pub content: FileContentRef,
    pub size_bytes: u64,
    pub content_hash: String,
    pub created_by: ChangeId,
    pub last_content_change: ChangeId,
    pub last_path_change: Option<ChangeId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileKind {
    Text,
    OpaqueText,
    Binary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileContentRef {
    Text(ObjectId),
    Opaque(ObjectId),
    Binary(ObjectId),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextContent {
    pub version: u16,
    pub content_hash: String,
    pub line_count: u64,
    pub byte_count: u64,
    #[serde(default)]
    pub full_bytes_blob_id: Option<ObjectId>,
    pub order_map_root: Option<String>,
    pub line_index_map_root: Option<String>,
    pub representation: TextRepresentation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TextRepresentation {
    TreeText,
    LazyText {
        blob_id: ObjectId,
        introduced_by: ChangeId,
    },
    OpaqueText {
        blob_id: ObjectId,
        reason: OpaqueReason,
    },
    SmallTextTable { table: Vec<u8> },
    SmallText { lines: Vec<LineEntry> },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OpaqueReason {
    TooLarge,
    LineTooLong,
    InvalidUtf8,
    BinaryLike,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineEntry {
    pub line_id: LineId,
    pub text: Vec<u8>,
    pub newline: NewlineKind,
    pub text_hash: String,
    pub introduced_by: ChangeId,
    pub last_content_change: ChangeId,
    pub last_move_change: Option<ChangeId>,
    pub flags: LineFlags,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NewlineKind {
    None,
    Lf,
    Crlf,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineFlags {
    pub generated: bool,
    pub redacted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Blob {
    pub version: u16,
    pub content_hash: String,
    pub bytes: Vec<u8>,
}
