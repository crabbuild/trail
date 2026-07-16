use super::*;

impl Trail {
    pub fn enqueue_lane_merge(
        &mut self,
        lane: &str,
        target: &str,
        priority: i64,
    ) -> Result<LaneMergeQueueAddReport> {
        let _lock = self.acquire_write_lock()?;
        let lane = self.lane_details(lane).map_err(|err| match err {
            Error::RefNotFound(_) => Error::InvalidInput(format!("lane `{lane}` does not exist")),
            other => other,
        })?;
        let target_ref = self.normalize_lane_merge_queue_target_ref(target)?;
        if let Some(entry) = self
            .conn
            .query_row(
                "SELECT q.queue_id, q.lane_id, l.name, q.target_ref, q.status, q.priority, q.created_at, q.updated_at \
                 FROM lane_merge_queue q JOIN lanes l ON l.lane_id = q.lane_id \
                 WHERE q.lane_id = ?1 AND q.target_ref = ?2 AND q.status IN ('queued', 'running') \
                 ORDER BY q.created_at LIMIT 1",
                params![lane.record.lane_id, target_ref],
                lane_merge_queue_row,
            )
            .optional()?
        {
            return Ok(LaneMergeQueueAddReport { entry });
        }

        let now = now_ts();
        let seed = format!("{}:{target_ref}:{priority}:{now}", lane.record.lane_id);
        let hash = sha256_hex(seed.as_bytes());
        let queue_id = format!("lmq_{}", &hash[..16]);
        self.conn.execute(
            "INSERT INTO lane_merge_queue \
             (queue_id, lane_id, target_ref, status, priority, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?5)",
            params![queue_id, lane.record.lane_id, target_ref, priority, now],
        )?;

        Ok(LaneMergeQueueAddReport {
            entry: LaneMergeQueueEntry {
                queue_id,
                lane_id: lane.record.lane_id,
                lane: lane.record.name,
                target_ref,
                status: "queued".to_string(),
                priority,
                created_at: now,
                updated_at: now,
            },
        })
    }

    pub fn list_lane_merge_queue(&self) -> Result<Vec<LaneMergeQueueEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT q.queue_id, q.lane_id, l.name, q.target_ref, q.status, q.priority, q.created_at, q.updated_at \
             FROM lane_merge_queue q JOIN lanes l ON l.lane_id = q.lane_id \
             ORDER BY q.status = 'queued' DESC, q.priority DESC, q.created_at ASC",
        )?;
        let rows = stmt.query_map([], lane_merge_queue_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn remove_lane_merge_queue(
        &mut self,
        selector: &str,
    ) -> Result<LaneMergeQueueRemoveReport> {
        let _lock = self.acquire_write_lock()?;
        let entry = self
            .conn
            .query_row(
                "SELECT q.queue_id, q.lane_id, l.name, q.target_ref, q.status, q.priority, q.created_at, q.updated_at \
                 FROM lane_merge_queue q JOIN lanes l ON l.lane_id = q.lane_id \
                 WHERE (q.queue_id = ?1 OR q.lane_id = ?1 OR l.name = ?1) \
                   AND q.status NOT IN ('merged', 'cancelled') \
                 ORDER BY q.priority DESC, q.created_at ASC LIMIT 1",
                params![selector],
                lane_merge_queue_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("merge queue item `{selector}` not found")))?;
        let now = now_ts();
        self.conn.execute(
            "UPDATE lane_merge_queue SET status = 'cancelled', updated_at = ?1 WHERE queue_id = ?2",
            params![now, entry.queue_id],
        )?;
        Ok(LaneMergeQueueRemoveReport {
            entry: LaneMergeQueueEntry {
                status: "cancelled".to_string(),
                updated_at: now,
                ..entry
            },
        })
    }

    pub fn explain_lane_merge_queue(
        &mut self,
        selector: &str,
    ) -> Result<LaneMergeQueueExplainReport> {
        let _lock = self.acquire_write_lock()?;
        let entry = self.lane_merge_queue_entry_by_selector(selector)?;
        let mut blockers = Vec::new();
        let mut warnings = Vec::new();
        let mut next_steps = Vec::new();

        let report = self.lane_readiness(&entry.lane_id)?;
        blockers.extend(report.blockers.clone());
        warnings.extend(report.warnings.clone());
        next_steps.extend(lane_merge_queue_readiness_next_steps(&entry.lane, &report));
        let readiness = Some(report);

        let (dry_run, error) = match self.lane_merge_queue_entry_dry_run(&entry) {
            Ok(report) => {
                if !report.conflicts.is_empty() {
                    blockers.push(readiness_issue(
                        "merge_conflicts",
                        format!(
                            "dry-run merge reports {} conflict(s)",
                            report.conflicts.len()
                        ),
                        Some(serde_json::json!({
                            "conflicts": report.conflicts.clone()
                        })),
                    ));
                    next_steps.push(lane_merge_queue_dry_run_next_step(&entry));
                }
                (Some(report), None)
            }
            Err(err) => {
                let message = err.to_string();
                blockers.push(readiness_issue(
                    "merge_preflight_failed",
                    "dry-run merge preflight failed",
                    Some(serde_json::json!({ "error": message })),
                ));
                next_steps.push(
                    "Fix the preflight error, then run `trail lane merge-queue explain` again."
                        .to_string(),
                );
                (None, Some(message))
            }
        };

        if blockers.is_empty() {
            next_steps.push("Run `trail lane merge-queue run` to merge this item.".to_string());
        }

        Ok(LaneMergeQueueExplainReport {
            entry,
            readiness,
            dry_run,
            blockers,
            warnings,
            error,
            next_steps,
        })
    }

    pub fn run_lane_merge_queue(
        &mut self,
        limit: Option<usize>,
    ) -> Result<LaneMergeQueueRunReport> {
        let _lock = self.acquire_write_lock()?;
        let entries = self.queued_lane_merge_entries(limit)?;
        let mut processed = Vec::new();
        let mut stopped_on_conflict = false;
        let mut stopped_on_failure = false;

        for entry in entries {
            self.set_lane_merge_queue_status(&entry.queue_id, "running")?;
            let context = match self.lane_merge_queue_context(&entry) {
                Ok(context) => context,
                Err(err) => {
                    self.set_lane_merge_queue_status(&entry.queue_id, "failed")?;
                    processed.push(LaneMergeQueueRunItem {
                        queue_id: entry.queue_id,
                        lane_id: entry.lane_id,
                        lane: entry.lane,
                        target_ref: entry.target_ref,
                        status: "failed".to_string(),
                        operation: None,
                        changed_paths: Vec::new(),
                        error: Some(err.to_string()),
                    });
                    stopped_on_failure = true;
                    break;
                }
            };

            match self.lane_merge_queue_entry(&entry) {
                Ok(report) => {
                    self.set_lane_merge_queue_status(&entry.queue_id, "merged")?;
                    self.insert_merge_result(
                        &entry,
                        &context,
                        Some(&report.operation),
                        "merged",
                        None,
                    )?;
                    processed.push(LaneMergeQueueRunItem {
                        queue_id: entry.queue_id,
                        lane_id: entry.lane_id,
                        lane: entry.lane,
                        target_ref: report.target_ref,
                        status: "merged".to_string(),
                        operation: Some(report.operation),
                        changed_paths: report.changed_paths,
                        error: None,
                    });
                }
                Err(err) => {
                    let is_conflict = matches!(err, Error::Conflict(_));
                    let status = if is_conflict { "conflicted" } else { "failed" };
                    let message = err.to_string();
                    self.set_lane_merge_queue_status(&entry.queue_id, status)?;
                    self.insert_merge_result(
                        &entry,
                        &context,
                        None,
                        status,
                        is_conflict.then_some(message.as_str()),
                    )?;
                    processed.push(LaneMergeQueueRunItem {
                        queue_id: entry.queue_id,
                        lane_id: entry.lane_id,
                        lane: entry.lane,
                        target_ref: entry.target_ref,
                        status: status.to_string(),
                        operation: None,
                        changed_paths: Vec::new(),
                        error: Some(message),
                    });
                    if is_conflict {
                        stopped_on_conflict = true;
                    } else {
                        stopped_on_failure = true;
                    }
                    break;
                }
            }
        }

        Ok(LaneMergeQueueRunReport {
            processed,
            stopped_on_conflict,
            stopped_on_failure,
        })
    }
}

fn lane_merge_queue_dry_run_next_step(entry: &LaneMergeQueueEntry) -> String {
    let target = entry
        .target_ref
        .strip_prefix(MAIN_REF_PREFIX)
        .unwrap_or(&entry.target_ref);
    format!(
        "Inspect conflicts with `trail lane merge {} --into {target} --dry-run` or run the queue to record a conflict set.",
        entry.lane
    )
}

fn lane_merge_queue_readiness_next_steps(
    lane: &str,
    readiness: &LaneReadinessReport,
) -> Vec<String> {
    let mut steps = Vec::new();
    for issue in &readiness.blockers {
        match issue.code.as_str() {
            "dirty_workdir" => {
                steps.push(format!(
                    "Record or discard dirty workdir changes with `trail lane record {lane}`."
                ));
            }
            "pending_approvals" => {
                let approval_ids = readiness
                    .pending_approvals
                    .iter()
                    .map(|approval| approval.approval_id.as_str())
                    .collect::<Vec<_>>();
                if let Some(first_id) = approval_ids.first() {
                    steps.push(format!(
                        "Review pending approvals with `trail approvals list --lane {lane}`; unblock this queue item with `trail approvals decide {first_id} --decision approved` after review."
                    ));
                } else {
                    steps.push(format!(
                        "Review pending approvals with `trail approvals list --lane {lane}`."
                    ));
                }
            }
            "open_conflicts" => {
                steps.push(
                    "Inspect open conflicts with `trail conflicts list` and resolve them before retrying."
                        .to_string(),
                );
            }
            "missing_latest_test" | "latest_test_failed" => {
                steps.push(format!(
                    "Run or fix the required test gate with `trail lane test {lane} -- <command>`."
                ));
            }
            "missing_latest_eval" | "latest_eval_failed" => {
                steps.push(format!(
                    "Run or fix the required eval gate with `trail lane eval {lane} -- <command>`."
                ));
            }
            _ => steps.push(format!("Resolve readiness blocker `{}`.", issue.code)),
        }
    }
    steps
}
