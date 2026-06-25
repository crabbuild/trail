use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct AgentSpawnArgs {
    pub(crate) name: String,
    #[serde(default, alias = "from", alias = "branch")]
    pub(crate) from_ref: Option<String>,
    #[serde(default)]
    pub(crate) materialize: Option<bool>,
    #[serde(default, alias = "workdir_path")]
    pub(crate) workdir: Option<String>,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
    #[serde(default, alias = "include_neighborhood")]
    pub(crate) include_neighbors: bool,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentClaimArgs {
    pub(crate) agent: String,
    pub(crate) path: String,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentHandleArgs {
    #[serde(alias = "agent_or_id", alias = "name")]
    pub(crate) agent: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentContributionArgs {
    #[serde(alias = "agent_or_id", alias = "name")]
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentRemoveArgs {
    #[serde(alias = "agent_or_id", alias = "name")]
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) force: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LeaseAcquireArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LeaseListArgs {
    #[serde(default)]
    pub(crate) all: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LeaseReleaseArgs {
    pub(crate) lease_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiffAgentArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) patch: bool,
    #[serde(default, alias = "show-line-ids")]
    pub(crate) show_line_ids: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GateHistoryArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RunTestArgs {
    pub(crate) agent: String,
    pub(crate) command: Vec<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
    #[serde(default, alias = "timeout_seconds")]
    pub(crate) timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) suite: Option<String>,
    #[serde(default)]
    pub(crate) score: Option<f64>,
    #[serde(default)]
    pub(crate) threshold: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReadFileArgs {
    pub(crate) agent: String,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) hydrate: Option<bool>,
    #[serde(default)]
    pub(crate) force: bool,
    #[serde(default, alias = "include_neighborhood")]
    pub(crate) include_neighbors: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SyncWorkdirArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) force: bool,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
    #[serde(default, alias = "include_neighborhood")]
    pub(crate) include_neighbors: bool,
}

pub(crate) fn default_lease_mode() -> String {
    "write".to_string()
}
