#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSpawnReport {
    pub agent_id: String,
    pub ref_name: String,
    pub base_change: ChangeId,
    pub workdir: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentPatchReport {
    pub agent_id: String,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRecordReport {
    pub agent_id: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRewindReport {
    pub agent_id: String,
    pub ref_name: String,
    pub target: String,
    pub previous_change: ChangeId,
    pub previous_root: ObjectId,
    pub target_change: ChangeId,
    pub target_root: ObjectId,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_current: Option<ChangeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    pub workdir_synced: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentWorkdirReport {
    pub agent_id: String,
    pub workdir: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentWorkdirSyncReport {
    pub agent_id: String,
    pub workdir: String,
    pub head_change: ChangeId,
    pub root_id: ObjectId,
    pub forced: bool,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentWatchReport {
    pub agent_id: String,
    pub iterations: u64,
    pub recorded_operations: Vec<ChangeId>,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTestReport {
    pub agent_id: String,
    pub turn_id: String,
    pub session_id: Option<String>,
    pub workdir: String,
    pub command: Vec<String>,
    #[serde(default = "default_agent_gate_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    pub status: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub stdout_object: ObjectId,
    pub stderr_object: ObjectId,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_preview: String,
    pub stderr_preview: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub started_event_id: String,
    pub finished_event_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTestSummary {
    pub event_id: String,
    pub turn_id: Option<String>,
    #[serde(default = "default_agent_gate_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    pub status: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub command: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentGateHistoryReport {
    pub agent: AgentDetails,
    pub kind: String,
    pub limit: usize,
    pub gates: Vec<AgentTestSummary>,
}

fn default_agent_gate_kind() -> String {
    "test".to_string()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentGateOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
}
