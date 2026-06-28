use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneSpawnArgs {
    pub(crate) name: String,
    #[serde(default, alias = "from", alias = "branch")]
    pub(crate) from_ref: Option<String>,
    #[serde(default)]
    pub(crate) materialize: Option<bool>,
    #[serde(default)]
    pub(crate) workdir_mode: Option<String>,
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
#[serde(deny_unknown_fields)]
pub(crate) struct LaneClaimArgs {
    pub(crate) lane: String,
    pub(crate) path: String,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneHandleArgs {
    #[serde(alias = "lane_or_id", alias = "name")]
    pub(crate) lane: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneContributionArgs {
    #[serde(alias = "lane_or_id", alias = "name")]
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneRefreshPreviewArgs {
    #[serde(alias = "lane_or_id", alias = "name")]
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) target: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneRemoveArgs {
    #[serde(alias = "lane_or_id", alias = "name")]
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) force: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneRewindArgs {
    #[serde(alias = "lane_or_id", alias = "name")]
    pub(crate) lane: String,
    #[serde(alias = "target")]
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) record_current: bool,
    #[serde(default)]
    pub(crate) sync_workdir: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LeaseAcquireArgs {
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LeaseListArgs {
    #[serde(default)]
    pub(crate) all: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LeaseReleaseArgs {
    pub(crate) lease_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiffLaneArgs {
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) patch: bool,
    #[serde(default, alias = "show-line-ids")]
    pub(crate) show_line_ids: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateHistoryArgs {
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunTestArgs {
    pub(crate) lane: String,
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
#[serde(deny_unknown_fields)]
pub(crate) struct ReadFileArgs {
    pub(crate) lane: String,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) hydrate: Option<bool>,
    #[serde(default)]
    pub(crate) force: bool,
    #[serde(default, alias = "include_neighborhood")]
    pub(crate) include_neighbors: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SyncWorkdirArgs {
    pub(crate) lane: String,
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
