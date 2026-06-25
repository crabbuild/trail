#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub version: u16,
    pub change_id: ChangeId,
    pub kind: OperationKind,
    pub parents: Vec<ChangeId>,
    pub before_root: Option<ObjectId>,
    pub after_root: ObjectId,
    pub branch: String,
    pub actor: Actor,
    pub session_id: Option<String>,
    pub message: Option<String>,
    pub changes: Vec<FileChange>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperationKind {
    Init,
    GitImport,
    FileEdit,
    MultiFileEdit,
    Format,
    ManualCheckpoint,
    ManualRecord,
    WatchRecord,
    Checkout,
    Branch,
    Merge,
    AgentSpawn,
    AgentPatch,
    AgentRecord,
    AgentMerge,
    GitExport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ActorKind {
    Human,
    Agent,
    System,
}

impl Actor {
    pub fn human() -> Self {
        Self {
            kind: ActorKind::Human,
            id: std::env::var("USER").unwrap_or_else(|_| "human".to_string()),
        }
    }

    pub fn system() -> Self {
        Self {
            kind: ActorKind::System,
            id: "crabdb".to_string(),
        }
    }

    pub fn agent(id: impl Into<String>) -> Self {
        Self {
            kind: ActorKind::Agent,
            id: id.into(),
        }
    }
}
