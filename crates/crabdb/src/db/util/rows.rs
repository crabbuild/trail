use super::*;

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
        explanation: None,
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
