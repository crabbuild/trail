use super::acp_sessions::lane_acp_session_row;
use super::*;

impl Trail {
    pub fn start_lane_session(
        &mut self,
        lane: &str,
        title: Option<String>,
        requested_session_id: Option<String>,
    ) -> Result<LaneSessionStartReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let session_id = match requested_session_id {
            Some(session_id) => {
                validate_session_id(&session_id)?;
                session_id
            }
            None => self.allocate_session_id(&branch.lane_id, title.as_deref()),
        };
        if self.try_lane_session(&session_id)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "session `{session_id}` already exists"
            )));
        }
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO lane_sessions \
             (session_id, lane_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, branch.lane_id, title, now],
        )?;
        self.conn.execute(
            "UPDATE lane_branches SET session_id = ?1, updated_at = ?2 WHERE lane_id = ?3",
            params![session_id, now, branch.lane_id],
        )?;
        self.insert_lane_event_with_context(
            &branch.lane_id,
            Some(&session_id),
            None,
            "session_started",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "session_id": session_id.clone(),
                "title": title.clone()
            }),
        )?;
        Ok(LaneSessionStartReport {
            session: self.lane_session(&session_id)?,
        })
    }

    pub fn list_lane_sessions(&self, lane: Option<&str>) -> Result<Vec<LaneSession>> {
        if let Some(lane) = lane {
            let branch = self.lane_branch(lane)?;
            let mut stmt = self.conn.prepare(
                "SELECT session_id, lane_id, title, status, started_at, ended_at, metadata_json \
                 FROM lane_sessions WHERE lane_id = ?1 ORDER BY started_at DESC, session_id DESC",
            )?;
            let rows = stmt.query_map(params![branch.lane_id], lane_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT session_id, lane_id, title, status, started_at, ended_at, metadata_json \
                 FROM lane_sessions ORDER BY started_at DESC, session_id DESC",
            )?;
            let rows = stmt.query_map([], lane_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        }
    }

    pub fn current_lane_sessions(
        &self,
        lane: Option<&str>,
    ) -> Result<Vec<LaneSessionCurrentReport>> {
        if let Some(lane) = lane {
            let details = self.lane_details(lane)?;
            let session = details
                .branch
                .session_id
                .as_deref()
                .map(|session_id| self.lane_session(session_id))
                .transpose()?;
            return Ok(vec![LaneSessionCurrentReport {
                lane_id: details.record.lane_id,
                lane_name: details.record.name,
                ref_name: details.branch.ref_name,
                session,
            }]);
        }

        let mut reports = Vec::new();
        for details in self.list_lanes()? {
            let Some(session_id) = details.branch.session_id.as_deref() else {
                continue;
            };
            reports.push(LaneSessionCurrentReport {
                lane_id: details.record.lane_id,
                lane_name: details.record.name,
                ref_name: details.branch.ref_name,
                session: Some(self.lane_session(session_id)?),
            });
        }
        Ok(reports)
    }

    pub fn show_lane_session(&self, session_id: &str) -> Result<LaneSessionDetails> {
        let session = self.lane_session(session_id)?;
        let turns = self.lane_session_turns(session_id)?;
        let messages = self.lane_session_messages(session_id)?;
        let events = self.lane_session_events(session_id)?;
        let operations = self.lane_session_operations(session_id)?;
        Ok(LaneSessionDetails {
            session,
            turns,
            messages,
            events,
            operations,
        })
    }

    pub fn transcript(&self, selector: &str) -> Result<TranscriptReport> {
        let selector = selector.trim();
        if selector.is_empty() {
            return Err(Error::InvalidInput(
                "transcript selector cannot be empty".to_string(),
            ));
        }

        let (resolved_kind, session, acp_session) =
            if let Some(acp) = self.try_lane_acp_session(selector)? {
                (
                    "acp_session".to_string(),
                    self.lane_session(&acp.trail_session_id)?,
                    Some(acp),
                )
            } else if let Some(session) = self.try_lane_session(selector)? {
                (
                    "session".to_string(),
                    session,
                    self.acp_session_for_session(selector)?,
                )
            } else {
                let lane_name = self.resolve_lane_handle(selector)?;
                let lane = self.lane_details(&lane_name)?;
                let session = if let Some(session_id) = lane.branch.session_id.as_deref() {
                    self.lane_session(session_id)?
                } else {
                    self.list_lane_sessions(Some(&lane_name))?
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            Error::InvalidInput(format!(
                                "lane `{selector}` has no sessions to transcript"
                            ))
                        })?
                };
                let acp = self.acp_session_for_session(&session.session_id)?;
                ("lane".to_string(), session, acp)
            };

        let lane_name = self.resolve_lane_handle(&session.lane_id)?;
        let details = self.show_lane_session(&session.session_id)?;
        let mut turns = Vec::new();
        for turn in details.turns {
            let turn_details = self.show_lane_turn(&turn.turn_id)?;
            let turn_envelope = turn_details.turn_envelope;
            let checkpoint = if turn_envelope
                .as_ref()
                .is_some_and(|envelope| envelope.outcome.no_changes)
            {
                None
            } else {
                turn_details.turn.after_change.clone().or_else(|| {
                    turn_details
                        .operations
                        .last()
                        .map(|operation| operation.change_id.clone())
                })
            };
            let tool_summaries = turn_details
                .events
                .iter()
                .filter(|event| {
                    matches!(
                        event.event_type.as_str(),
                        "tool_call" | "tool_call_update" | "span_started" | "span_ended"
                    )
                })
                .filter_map(tool_summary_for_event)
                .collect();
            turns.push(TranscriptTurn {
                turn_envelope,
                turn: turn_details.turn,
                messages: turn_details
                    .messages
                    .into_iter()
                    .map(|message| TranscriptMessage {
                        message_id: message.id,
                        role: message.role,
                        body: message.body,
                        created_at: message.created_at,
                    })
                    .collect(),
                events: turn_details.events,
                checkpoint,
                tool_summaries,
            });
        }

        Ok(TranscriptReport {
            selector: selector.to_string(),
            resolved_kind,
            lane_id: session.lane_id.clone(),
            lane_name,
            session,
            acp_session,
            turns,
            operations: details.operations,
        })
    }

    pub fn lane_session_context(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<LaneSessionContextReport> {
        let limit = normalize_query_limit(limit, 200)?;
        let session = self.lane_session(session_id)?;
        let turns = self.lane_session_turns(session_id)?;
        let messages = self.lane_session_messages(session_id)?;
        let events = self.lane_session_events(session_id)?;
        let operations = self.lane_session_operations(session_id)?;
        Ok(LaneSessionContextReport {
            session,
            message_count: messages.len() as u64,
            event_count: events.len() as u64,
            turn_count: turns.len() as u64,
            operation_count: operations.len() as u64,
            recent_messages: tail_limited(&messages, limit),
            recent_events: tail_limited(&events, limit),
            recent_turns: tail_limited(&turns, limit),
            recent_operations: tail_limited(&operations, limit),
        })
    }

    pub fn end_lane_session(
        &mut self,
        session_id: &str,
        status: &str,
    ) -> Result<LaneSessionEndReport> {
        let _lock = self.acquire_write_lock()?;
        let status = parse_session_end_status(status)?;
        let session = self.lane_session(session_id)?;
        let now = now_ts();
        self.conn.execute(
            "UPDATE lane_sessions SET status = ?1, ended_at = ?2 WHERE session_id = ?3",
            params![status, now, session_id],
        )?;
        self.conn.execute(
            "UPDATE lane_branches SET session_id = NULL, updated_at = ?1 \
             WHERE lane_id = ?2 AND session_id = ?3",
            params![now, session.lane_id, session_id],
        )?;
        self.insert_lane_event_with_context(
            &session.lane_id,
            Some(session_id),
            None,
            "session_ended",
            None,
            None,
            &serde_json::json!({
                "session_id": session_id,
                "status": status
            }),
        )?;
        Ok(LaneSessionEndReport {
            session: self.lane_session(session_id)?,
        })
    }

    fn acp_session_for_session(&self, session_id: &str) -> Result<Option<LaneAcpSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT acp_session_id, upstream_session_id, lane_id, trail_session_id, cwd, path_mappings_json, provider, model, upstream_command_json, status, created_at, updated_at, current_mode_id, config_options_json \
             FROM lane_acp_sessions WHERE trail_session_id = ?1 ORDER BY updated_at DESC, acp_session_id DESC LIMIT 1",
        )?;
        stmt.query_row(params![session_id], lane_acp_session_row)
            .optional()
            .map_err(Error::from)
    }
}

fn tool_summary_for_event(event: &LaneEventRecord) -> Option<String> {
    let payload = event.payload.as_ref()?;
    let title = payload
        .get("title")
        .or_else(|| payload.get("name"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            payload
                .get("attributes")
                .and_then(|attributes| attributes.get("title").or_else(|| attributes.get("name")))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or(event.event_type.as_str());
    let status = payload
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if status.is_empty() {
        Some(title.to_string())
    } else {
        Some(format!("{title} ({status})"))
    }
}
