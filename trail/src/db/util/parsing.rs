use super::*;

pub(crate) fn parse_line_change_kind(value: &str) -> LineChangeKind {
    match value {
        "Added" => LineChangeKind::Added,
        "Deleted" => LineChangeKind::Deleted,
        "Moved" => LineChangeKind::Moved,
        _ => LineChangeKind::Modified,
    }
}

pub(crate) fn parse_conflict_take(value: &str) -> Result<ConflictTake> {
    match value {
        "source" => Ok(ConflictTake::Source),
        "target" => Ok(ConflictTake::Target),
        other => Err(Error::InvalidInput(format!(
            "conflict resolution must take `source` or `target`, got `{other}`"
        ))),
    }
}

pub(crate) fn parse_lease_mode(value: &str) -> Result<&'static str> {
    match value {
        "read" => Ok("read"),
        "write" => Ok("write"),
        other => Err(Error::InvalidInput(format!(
            "lease mode must be `read` or `write`, got `{other}`"
        ))),
    }
}

pub(crate) fn parse_session_end_status(value: &str) -> Result<&'static str> {
    match value {
        "completed" => Ok("completed"),
        "failed" => Ok("failed"),
        "cancelled" => Ok("cancelled"),
        "interrupted" => Ok("interrupted"),
        "archived" => Ok("archived"),
        other => Err(Error::InvalidInput(format!(
            "session end status must be completed, failed, cancelled, interrupted, or archived, got `{other}`"
        ))),
    }
}

pub(crate) fn parse_approval_status_filter(value: &str) -> Result<Option<&'static str>> {
    match value {
        "all" => Ok(None),
        "pending" => Ok(Some("pending")),
        "approved" => Ok(Some("approved")),
        "rejected" => Ok(Some("rejected")),
        "cancelled" => Ok(Some("cancelled")),
        other => Err(Error::InvalidInput(format!(
            "approval status must be pending, approved, rejected, cancelled, or all, got `{other}`"
        ))),
    }
}

pub(crate) fn parse_lane_run_status_filter(value: &str) -> Result<Option<&'static str>> {
    match value {
        "all" => Ok(None),
        "paused" => Ok(Some("paused")),
        "resumed" => Ok(Some("resumed")),
        "blocked" => Ok(Some("blocked")),
        "cancelled" | "canceled" => Ok(Some("cancelled")),
        other => Err(Error::InvalidInput(format!(
            "lane run status must be paused, resumed, blocked, cancelled, or all, got `{other}`"
        ))),
    }
}

pub(crate) fn parse_approval_decision(value: &str) -> Result<&'static str> {
    match value {
        "approved" | "approve" => Ok("approved"),
        "rejected" | "reject" => Ok("rejected"),
        "cancelled" | "cancel" => Ok("cancelled"),
        other => Err(Error::InvalidInput(format!(
            "approval decision must be approved, rejected, or cancelled, got `{other}`"
        ))),
    }
}

pub(crate) fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.trim().is_empty() {
        return Err(Error::InvalidInput(
            "session id cannot be empty".to_string(),
        ));
    }
    if !session_id.starts_with("session_") && !session_id.starts_with("session-") {
        return Err(Error::InvalidInput(format!(
            "session id `{session_id}` must start with `session_` or `session-`"
        )));
    }
    if !session_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(Error::InvalidInput(format!(
            "session id `{session_id}` contains invalid characters"
        )));
    }
    Ok(())
}

pub(crate) fn parse_file_change_kind(value: &str) -> FileChangeKind {
    match value {
        "Added" => FileChangeKind::Added,
        "Deleted" => FileChangeKind::Deleted,
        "Renamed" => FileChangeKind::Renamed,
        "TypeChanged" => FileChangeKind::TypeChanged,
        _ => FileChangeKind::Modified,
    }
}

pub(crate) fn parse_operation_kind(value: &str) -> OperationKind {
    match value {
        "GitImport" => OperationKind::GitImport,
        "FileEdit" => OperationKind::FileEdit,
        "MultiFileEdit" => OperationKind::MultiFileEdit,
        "Format" => OperationKind::Format,
        "ManualCheckpoint" => OperationKind::ManualCheckpoint,
        "ManualRecord" => OperationKind::ManualRecord,
        "WatchRecord" => OperationKind::WatchRecord,
        "Checkout" => OperationKind::Checkout,
        "Branch" => OperationKind::Branch,
        "Merge" => OperationKind::Merge,
        "LaneSpawn" => OperationKind::LaneSpawn,
        "LanePatch" => OperationKind::LanePatch,
        "LaneRecord" => OperationKind::LaneRecord,
        "LaneRewind" => OperationKind::LaneRewind,
        "LaneMerge" => OperationKind::LaneMerge,
        "GitExport" => OperationKind::GitExport,
        _ => OperationKind::Init,
    }
}

pub(crate) fn parse_range(spec: &str) -> Result<(&str, &str)> {
    let Some((left, right)) = spec.split_once("..") else {
        return Err(Error::InvalidInput(format!(
            "range `{spec}` must look like left..right"
        )));
    };
    if left.is_empty() || right.is_empty() {
        return Err(Error::InvalidInput(format!(
            "range `{spec}` must include both endpoints"
        )));
    }
    Ok((left, right))
}

pub(crate) fn parse_path_line(spec: &str) -> Result<(String, u64)> {
    let Some((path, line)) = spec.rsplit_once(':') else {
        return Err(Error::InvalidInput(format!(
            "`{spec}` must look like path:line"
        )));
    };
    let line_number = line
        .parse::<u64>()
        .map_err(|_| Error::InvalidInput(format!("invalid line number `{line}`")))?;
    if line_number == 0 {
        return Err(Error::InvalidInput("line numbers are 1-based".to_string()));
    }
    Ok((normalize_relative_path(path)?, line_number))
}
