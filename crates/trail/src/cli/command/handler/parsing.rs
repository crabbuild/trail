use std::collections::BTreeMap;
use std::fs;

use trail::model::{ConflictManualFile, ConflictManualResolution};

use super::*;

pub(super) fn parse_optional_json(value: Option<&str>) -> Result<Option<serde_json::Value>> {
    value
        .map(serde_json::from_str)
        .transpose()
        .map_err(Error::from)
}

pub(super) fn read_manual_conflict_resolution(path: &PathBuf) -> Result<ConflictManualResolution> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    if value.get("files").is_some() {
        return serde_json::from_value(value).map_err(Error::from);
    }
    let files: BTreeMap<String, ConflictManualFile> =
        serde_json::from_value(value).map_err(Error::from)?;
    Ok(ConflictManualResolution { files })
}

pub(super) fn parse_record_kind_arg(value: &str) -> Result<OperationKind> {
    match value {
        "file-edit" => Ok(OperationKind::FileEdit),
        "multi-file-edit" => Ok(OperationKind::MultiFileEdit),
        "format" => Ok(OperationKind::Format),
        "manual-checkpoint" => Ok(OperationKind::ManualCheckpoint),
        "manual-record" => Ok(OperationKind::ManualRecord),
        other => Err(Error::InvalidInput(format!(
            "record kind must be file-edit, multi-file-edit, format, manual-checkpoint, or manual-record, got `{other}`"
        ))),
    }
}

pub(super) fn validate_merge_strategy(value: Option<&str>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    match value {
        "conservative" | "line-id-aware" | "line_id_aware" => Ok(()),
        other => Err(Error::InvalidInput(format!(
            "merge strategy must be conservative, line-id-aware, or line_id_aware, got `{other}`"
        ))),
    }
}

pub(super) fn command_failure_exit_code(exit_code: Option<i32>) -> i32 {
    exit_code
        .filter(|code| *code != 0)
        .unwrap_or(1)
        .clamp(1, 255)
}
