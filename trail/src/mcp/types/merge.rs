use serde::Deserialize;

use crate::model::ConflictManualResolution;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneMergeQueueAddArgs {
    pub(crate) lane: String,
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) priority: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneMergeQueueRunArgs {
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneMergeQueueExplainArgs {
    pub(crate) selector: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneMergeQueueRemoveArgs {
    pub(crate) selector: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConflictIdArgs {
    pub(crate) conflict_set_id: String,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConflictResolveArgs {
    pub(crate) conflict_set_id: String,
    #[serde(default)]
    pub(crate) take: Option<String>,
    #[serde(default)]
    pub(crate) manual: Option<ConflictManualResolution>,
}
