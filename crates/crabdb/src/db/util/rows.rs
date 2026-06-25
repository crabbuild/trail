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

#[derive(Debug)]
pub(crate) enum ManualConflictPayload {
    Text { content: String, executable: bool },
    Delete,
}

pub(crate) fn normalize_manual_conflict_files(
    manual: ConflictManualResolution,
    conflict_paths: &BTreeSet<String>,
) -> Result<BTreeMap<String, ConflictManualFile>> {
    if manual.files.is_empty() {
        return Err(Error::InvalidInput(
            "manual conflict resolution must include at least one file".to_string(),
        ));
    }

    let mut normalized = BTreeMap::new();
    for (path, file) in manual.files {
        let normalized_path = normalize_relative_path(&path)?;
        if normalized.insert(normalized_path.clone(), file).is_some() {
            return Err(Error::InvalidInput(format!(
                "manual conflict resolution includes duplicate path `{normalized_path}`"
            )));
        }
    }

    let provided = normalized.keys().cloned().collect::<BTreeSet<_>>();
    let missing = conflict_paths
        .difference(&provided)
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(Error::InvalidInput(format!(
            "manual conflict resolution is missing conflicted path(s): {}",
            missing.join(", ")
        )));
    }

    let extra = provided
        .difference(conflict_paths)
        .cloned()
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        return Err(Error::InvalidInput(format!(
            "manual conflict resolution includes non-conflicted path(s): {}",
            extra.join(", ")
        )));
    }

    Ok(normalized)
}

pub(crate) fn manual_conflict_file_payload(
    file: ConflictManualFile,
    default_executable: bool,
) -> Result<ManualConflictPayload> {
    match file {
        ConflictManualFile::Text(content) => Ok(ManualConflictPayload::Text {
            content,
            executable: default_executable,
        }),
        ConflictManualFile::Spec(spec) if spec.delete => {
            if spec.content.is_some() {
                return Err(Error::InvalidInput(
                    "manual conflict file cannot set both `delete` and `content`".to_string(),
                ));
            }
            Ok(ManualConflictPayload::Delete)
        }
        ConflictManualFile::Spec(spec) => {
            let Some(content) = spec.content else {
                return Err(Error::InvalidInput(
                    "manual conflict file must include `content` or set `delete` to true"
                        .to_string(),
                ));
            };
            Ok(ManualConflictPayload::Text {
                content,
                executable: spec.executable.unwrap_or(default_executable),
            })
        }
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
        "archived" => Ok("archived"),
        other => Err(Error::InvalidInput(format!(
            "session end status must be completed, failed, cancelled, or archived, got `{other}`"
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

pub(crate) fn parse_agent_run_status_filter(value: &str) -> Result<Option<&'static str>> {
    match value {
        "all" => Ok(None),
        "paused" => Ok(Some("paused")),
        "resumed" => Ok(Some("resumed")),
        "blocked" => Ok(Some("blocked")),
        "cancelled" | "canceled" => Ok(Some("cancelled")),
        other => Err(Error::InvalidInput(format!(
            "agent run status must be paused, resumed, blocked, cancelled, or all, got `{other}`"
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

pub(crate) fn conflict_paths_from_details(details: &[String]) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    for detail in details {
        let mut parts = detail.split('`');
        let _before = parts.next();
        if let Some(path) = parts.next() {
            paths.insert(normalize_relative_path(path)?);
        }
    }
    if paths.is_empty() {
        return Err(Error::InvalidInput(
            "conflict set does not include path details that can be resolved automatically"
                .to_string(),
        ));
    }
    Ok(paths)
}

pub(crate) fn build_agent_trace_spans(events: Vec<AgentEventRecord>) -> Vec<AgentTraceSpan> {
    let mut builders: BTreeMap<String, AgentTraceSpanBuilder> = BTreeMap::new();

    for event in events {
        let Some(payload) = event.payload.as_ref() else {
            continue;
        };
        let Some(span_id) = payload_string(payload, "span_id") else {
            continue;
        };

        match event.event_type.as_str() {
            "span_started" => {
                let trace_id = payload_string(payload, "trace_id").unwrap_or_else(|| {
                    event
                        .turn_id
                        .as_deref()
                        .map(default_trace_id_for_turn)
                        .unwrap_or_else(|| default_trace_id_for_turn(&event.event_id))
                });
                let builder = AgentTraceSpanBuilder {
                    span_id: span_id.clone(),
                    trace_id,
                    agent_id: event.agent_id.clone(),
                    session_id: event.session_id.clone(),
                    turn_id: event.turn_id.clone(),
                    parent_span_id: payload_string(payload, "parent_span_id"),
                    span_type: payload_string(payload, "span_type")
                        .unwrap_or_else(|| "custom".to_string()),
                    name: payload_string(payload, "name").unwrap_or_else(|| span_id.clone()),
                    started_event_id: event.event_id.clone(),
                    started_at: event.created_at,
                    attributes: payload_value(payload, "attributes"),
                    ended_event_id: None,
                    ended_at: None,
                    status: None,
                    result: None,
                };
                builders.entry(span_id).or_insert(builder);
            }
            "span_ended" => {
                if let Some(builder) = builders.get_mut(&span_id) {
                    builder.ended_event_id = Some(event.event_id.clone());
                    builder.ended_at = Some(event.created_at);
                    builder.status = payload_string(payload, "status");
                    builder.result = payload_value(payload, "result");
                }
            }
            _ => {}
        }
    }

    builders
        .into_values()
        .map(agent_trace_span_from_builder)
        .collect()
}

pub(crate) fn agent_trace_span_from_builder(builder: AgentTraceSpanBuilder) -> AgentTraceSpan {
    let duration_ms = builder
        .ended_at
        .and_then(|ended_at| ended_at.checked_sub(builder.started_at))
        .map(|seconds| seconds as u64 * 1000);
    AgentTraceSpan {
        span_id: builder.span_id,
        trace_id: builder.trace_id,
        agent_id: builder.agent_id,
        session_id: builder.session_id,
        turn_id: builder.turn_id,
        parent_span_id: builder.parent_span_id,
        span_type: builder.span_type,
        name: builder.name,
        status: builder.status.unwrap_or_else(|| {
            if builder.ended_at.is_some() {
                "completed".to_string()
            } else {
                "running".to_string()
            }
        }),
        started_event_id: builder.started_event_id,
        ended_event_id: builder.ended_event_id,
        started_at: builder.started_at,
        ended_at: builder.ended_at,
        duration_ms,
        attributes: builder.attributes,
        result: builder.result,
    }
}

pub(crate) fn named_counts(counts: BTreeMap<String, u64>) -> Vec<NamedCount> {
    counts
        .into_iter()
        .map(|(name, count)| NamedCount { name, count })
        .collect()
}

pub(crate) fn tail_limited<T: Clone>(values: &[T], limit: usize) -> Vec<T> {
    let start = values.len().saturating_sub(limit);
    values[start..].to_vec()
}

pub(crate) fn agent_trace_status_is_failed(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "failed" | "error" | "errored" | "cancelled" | "canceled" | "timeout" | "timed_out"
    )
}

pub(crate) fn payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

pub(crate) fn payload_value(payload: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    payload.get(key).filter(|value| !value.is_null()).cloned()
}

pub(crate) fn default_trace_id_for_turn(turn_id: &str) -> String {
    format!("trace_{}", crate::ids::short_hash(turn_id.as_bytes(), 16))
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

pub(crate) fn ref_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RefRecord> {
    Ok(RefRecord {
        name: row.get(0)?,
        change_id: ChangeId(row.get(1)?),
        root_id: ObjectId(row.get(2)?),
        operation_id: ObjectId(row.get(3)?),
        generation: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

pub(crate) fn file_history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileHistoryEntry> {
    Ok(FileHistoryEntry {
        file_id: row.get(0)?,
        change_id: ChangeId(row.get(1)?),
        path: row.get(2)?,
        old_path: row.get(3)?,
        kind: parse_file_change_kind(&row.get::<_, String>(4)?),
        before_hash: row.get(5)?,
        after_hash: row.get(6)?,
        created_at: row.get(7)?,
    })
}

pub(crate) fn line_history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LineHistoryEntry> {
    Ok(LineHistoryEntry {
        change_id: ChangeId(row.get(0)?),
        path: row.get(1)?,
        line_number: row.get::<_, Option<i64>>(2)?.map(|n| n as u64),
        kind: parse_line_change_kind(&row.get::<_, String>(3)?),
        text_hash: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub(crate) fn agent_details_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentDetails> {
    Ok(AgentDetails {
        record: AgentRecord {
            agent_id: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            provider: row.get(3)?,
            model: row.get(4)?,
            created_at: row.get(5)?,
            metadata_json: row.get(6)?,
        },
        branch: AgentBranch {
            agent_id: row.get(0)?,
            ref_name: row.get(7)?,
            base_change: ChangeId(row.get(8)?),
            head_change: ChangeId(row.get(9)?),
            base_root: ObjectId(row.get(10)?),
            head_root: ObjectId(row.get(11)?),
            session_id: row.get(12)?,
            workdir: row.get(13)?,
            status: row.get(14)?,
            created_at: row.get(15)?,
            updated_at: row.get(16)?,
        },
    })
}

pub(crate) fn merge_queue_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MergeQueueEntry> {
    Ok(MergeQueueEntry {
        queue_id: row.get(0)?,
        source_ref: row.get(1)?,
        target_ref: row.get(2)?,
        status: row.get(3)?,
        priority: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

pub(crate) fn conflict_set_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConflictSetSummary> {
    let details_json: Option<String> = row.get(5)?;
    let details = details_json
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default();
    Ok(ConflictSetSummary {
        conflict_set_id: row.get(0)?,
        merge_id: row.get(1)?,
        source_ref: row.get(2)?,
        target_ref: row.get(3)?,
        status: row.get(4)?,
        details,
        created_at: row.get(6)?,
    })
}

pub(crate) fn lease_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LeaseRecord> {
    Ok(LeaseRecord {
        lease_id: row.get(0)?,
        agent_id: row.get(1)?,
        ref_name: row.get(2)?,
        path: row.get(3)?,
        file_id: row.get(4)?,
        mode: row.get(5)?,
        expires_at: row.get(6)?,
        created_at: row.get(7)?,
    })
}

pub(crate) fn git_mapping_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GitMapping> {
    Ok(GitMapping {
        mapping_id: row.get(0)?,
        direction: row.get(1)?,
        branch: row.get(2)?,
        git_head: row.get(3)?,
        git_dirty: row.get::<_, i64>(4)? != 0,
        crab_change: ChangeId(row.get(5)?),
        crab_root: ObjectId(row.get(6)?),
        created_at: row.get(7)?,
    })
}

pub(crate) fn agent_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSession> {
    Ok(AgentSession {
        session_id: row.get(0)?,
        agent_id: row.get(1)?,
        title: row.get(2)?,
        status: row.get(3)?,
        started_at: row.get(4)?,
        ended_at: row.get(5)?,
        metadata_json: row.get(6)?,
    })
}

pub(crate) fn agent_turn_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTurn> {
    Ok(AgentTurn {
        turn_id: row.get(0)?,
        agent_id: row.get(1)?,
        session_id: row.get(2)?,
        base_change: ChangeId(row.get(3)?),
        before_change: ChangeId(row.get(4)?),
        after_change: row.get::<_, Option<String>>(5)?.map(ChangeId),
        status: row.get(6)?,
        started_at: row.get(7)?,
        ended_at: row.get(8)?,
        metadata_json: row.get(9)?,
    })
}

pub(crate) fn agent_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentEventRecord> {
    let payload_json: Option<String> = row.get(7)?;
    let payload =
        payload_json.and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
    Ok(AgentEventRecord {
        event_id: row.get(0)?,
        agent_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        event_type: row.get(4)?,
        change_id: row.get::<_, Option<String>>(5)?.map(ChangeId),
        message_id: row.get::<_, Option<String>>(6)?.map(MessageId),
        payload,
        created_at: row.get(8)?,
    })
}

pub(crate) fn agent_approval_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentApproval> {
    let payload_json: Option<String> = row.get(6)?;
    let payload =
        payload_json.and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
    Ok(AgentApproval {
        approval_id: row.get(0)?,
        agent_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        action: row.get(4)?,
        summary: row.get(5)?,
        payload,
        status: row.get(7)?,
        requested_at: row.get(8)?,
        decided_at: row.get(9)?,
        reviewer: row.get(10)?,
        note: row.get(11)?,
    })
}

pub(crate) fn agent_run_state_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRunState> {
    let state_json: String = row.get(8)?;
    let state =
        serde_json::from_str::<serde_json::Value>(&state_json).unwrap_or(serde_json::Value::Null);
    let interruption_json: Option<String> = row.get(9)?;
    let interruption =
        interruption_json.and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
    Ok(AgentRunState {
        run_id: row.get(0)?,
        agent_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        approval_id: row.get(4)?,
        status: row.get(5)?,
        reason: row.get(6)?,
        summary: row.get(7)?,
        state,
        interruption,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        resumed_at: row.get(12)?,
        reviewer: row.get(13)?,
        note: row.get(14)?,
    })
}

pub(crate) fn timeline_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TimelineEntry> {
    Ok(TimelineEntry {
        change_id: ChangeId(row.get(0)?),
        kind: parse_operation_kind(&row.get::<_, String>(1)?),
        branch: row.get(2)?,
        actor_id: row.get(3)?,
        message: row.get(4)?,
        created_at: row.get(5)?,
        path_count: row.get::<_, i64>(6)? as u64,
    })
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
        "AgentSpawn" => OperationKind::AgentSpawn,
        "AgentPatch" => OperationKind::AgentPatch,
        "AgentRecord" => OperationKind::AgentRecord,
        "AgentMerge" => OperationKind::AgentMerge,
        "GitExport" => OperationKind::GitExport,
        _ => OperationKind::Init,
    }
}

#[cfg(unix)]
pub(crate) fn executable(path: &Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
}

#[cfg(unix)]
pub(crate) fn executable_from_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
pub(crate) fn executable(_path: &Path) -> Result<bool> {
    Ok(false)
}

#[cfg(not(unix))]
pub(crate) fn executable_from_metadata(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn set_executable(path: &Path, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    let mut mode = permissions.mode();
    if executable {
        mode |= 0o755;
    } else {
        mode &= !0o111;
    }
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn set_executable(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}
