use serde::Deserialize;

use crate::model::ConflictManualResolution;

#[derive(Debug, Deserialize)]
pub(crate) struct MergeQueueAddArgs {
    pub(crate) source: String,
    #[serde(alias = "into", alias = "target_branch")]
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) priority: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MergeQueueRunArgs {
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MergeQueueRemoveArgs {
    pub(crate) selector: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConflictIdArgs {
    pub(crate) conflict_set_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConflictResolveArgs {
    pub(crate) conflict_set_id: String,
    #[serde(default)]
    pub(crate) take: Option<String>,
    #[serde(default)]
    pub(crate) manual: Option<ConflictManualResolution>,
}
