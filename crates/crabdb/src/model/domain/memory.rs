#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryVersionSource {
    pub source_ref: Option<String>,
    pub source_change: Option<ChangeId>,
    pub source_root: Option<ObjectId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryEmbeddingInput {
    pub provider: String,
    pub model: String,
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEmbeddingInfo {
    pub provider: String,
    pub model: String,
    pub dims: usize,
    pub embedding_hash: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryPut {
    pub memory_id: Option<String>,
    pub scope_type: String,
    pub scope_id: String,
    pub kind: String,
    pub path: Option<String>,
    pub title: Option<String>,
    pub body: String,
    pub actor_id: String,
    pub source: MemoryVersionSource,
    pub metadata: serde_json::Value,
    pub embedding: Option<MemoryEmbeddingInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryItem {
    pub memory_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub kind: String,
    pub path: Option<String>,
    pub title: Option<String>,
    pub body: String,
    pub status: String,
    pub source: MemoryVersionSource,
    pub metadata: serde_json::Value,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub embedding: Option<MemoryEmbeddingInfo>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemorySearchBackend {
    #[default]
    Auto,
    SqliteVec,
    Exact,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemorySearch {
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub kind: Option<String>,
    pub path_prefix: Option<String>,
    pub source_ref: Option<String>,
    pub source_change: Option<ChangeId>,
    pub status: Option<String>,
    pub query_embedding: Option<Vec<f32>>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub top_k: usize,
    pub backend: MemorySearchBackend,
}

impl Default for MemorySearch {
    fn default() -> Self {
        Self {
            scope_type: None,
            scope_id: None,
            kind: None,
            path_prefix: None,
            source_ref: None,
            source_change: None,
            status: Some("active".to_string()),
            query_embedding: None,
            embedding_provider: None,
            embedding_model: None,
            top_k: 20,
            backend: MemorySearchBackend::Auto,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemorySearchResult {
    pub item: MemoryItem,
    pub distance: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryContextEntry {
    pub memory_id: String,
    pub title: Option<String>,
    pub path: Option<String>,
    pub body: String,
    pub distance: Option<f32>,
    pub citation: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryContextPacket {
    pub backend: MemorySearchBackend,
    pub entries: Vec<MemoryContextEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRevision {
    pub revision_id: String,
    pub memory_id: String,
    pub version: i64,
    pub operation: String,
    pub scope_type: String,
    pub scope_id: String,
    pub kind: String,
    pub path: Option<String>,
    pub title: Option<String>,
    pub body: String,
    pub status: String,
    pub source: MemoryVersionSource,
    pub metadata: serde_json::Value,
    pub embedding_hash: Option<String>,
    pub actor_id: String,
    pub created_at: i64,
}
