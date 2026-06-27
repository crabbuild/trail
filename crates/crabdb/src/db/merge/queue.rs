use super::*;

impl CrabDb {
    pub fn enqueue_merge(
        &mut self,
        source: &str,
        target: &str,
        priority: i64,
    ) -> Result<MergeQueueAddReport> {
        let _lock = self.acquire_write_lock()?;
        let source_ref = self.normalize_merge_queue_source_ref(source)?;
        let target_ref = self.normalize_merge_queue_target_ref(target)?;
        if let Some(entry) = self
            .conn
            .query_row(
                "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
                 FROM merge_queue \
                 WHERE source_ref = ?1 AND target_ref = ?2 AND status IN ('queued', 'running') \
                 ORDER BY created_at LIMIT 1",
                params![source_ref, target_ref],
                merge_queue_row,
            )
            .optional()?
        {
            return Ok(MergeQueueAddReport { entry });
        }

        let now = now_ts();
        let seed = format!("{source_ref}:{target_ref}:{priority}:{now}");
        let hash = sha256_hex(seed.as_bytes());
        let queue_id = format!("mq_{}", &hash[..16]);
        self.conn.execute(
            "INSERT INTO merge_queue \
             (queue_id, source_ref, target_ref, status, priority, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?5)",
            params![queue_id, source_ref, target_ref, priority, now],
        )?;

        Ok(MergeQueueAddReport {
            entry: MergeQueueEntry {
                queue_id,
                source_ref,
                target_ref,
                status: "queued".to_string(),
                priority,
                created_at: now,
                updated_at: now,
            },
        })
    }

    pub fn list_merge_queue(&self) -> Result<Vec<MergeQueueEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
             FROM merge_queue ORDER BY status = 'queued' DESC, priority DESC, created_at ASC",
        )?;
        let rows = stmt.query_map([], merge_queue_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn remove_merge_queue(&mut self, selector: &str) -> Result<MergeQueueRemoveReport> {
        let _lock = self.acquire_write_lock()?;
        let lane_candidate = lane_ref(selector);
        let branch_candidate = branch_ref(selector);
        let entry = self
            .conn
            .query_row(
                "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
                 FROM merge_queue \
                 WHERE (queue_id = ?1 OR source_ref = ?1 OR source_ref = ?2 OR source_ref = ?3) \
                   AND status NOT IN ('merged', 'cancelled') \
                 ORDER BY priority DESC, created_at ASC LIMIT 1",
                params![selector, lane_candidate, branch_candidate],
                merge_queue_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("merge queue item `{selector}` not found")))?;
        let now = now_ts();
        self.conn.execute(
            "UPDATE merge_queue SET status = 'cancelled', updated_at = ?1 WHERE queue_id = ?2",
            params![now, entry.queue_id],
        )?;
        Ok(MergeQueueRemoveReport {
            entry: MergeQueueEntry {
                status: "cancelled".to_string(),
                updated_at: now,
                ..entry
            },
        })
    }

    pub fn explain_merge_queue(&mut self, selector: &str) -> Result<MergeQueueExplainReport> {
        let _lock = self.acquire_write_lock()?;
        let entry = self.merge_queue_entry_by_selector(selector)?;
        let mut readiness = None;
        let mut blockers = Vec::new();
        let mut warnings = Vec::new();
        let mut next_steps = Vec::new();

        if let Some(lane) = entry.source_ref.strip_prefix(LANE_REF_PREFIX) {
            let report = self.lane_readiness(lane)?;
            blockers.extend(report.blockers.clone());
            warnings.extend(report.warnings.clone());
            next_steps.extend(merge_queue_readiness_next_steps(lane, &report));
            readiness = Some(report);
        }

        let (dry_run, error) = match self.merge_queue_entry_dry_run(&entry) {
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
                    next_steps.push(merge_queue_dry_run_next_step(&entry));
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
                    "Fix the preflight error, then run `crabdb merge-queue explain` again."
                        .to_string(),
                );
                (None, Some(message))
            }
        };

        if blockers.is_empty() {
            next_steps.push("Run `crabdb merge-queue run` to merge this item.".to_string());
        }

        Ok(MergeQueueExplainReport {
            entry,
            readiness,
            dry_run,
            blockers,
            warnings,
            error,
            next_steps,
        })
    }

    pub fn run_merge_queue(&mut self, limit: Option<usize>) -> Result<MergeQueueRunReport> {
        let _lock = self.acquire_write_lock()?;
        let entries = self.queued_merge_entries(limit)?;
        let mut processed = Vec::new();
        let mut stopped_on_conflict = false;
        let mut stopped_on_failure = false;

        for entry in entries {
            self.set_merge_queue_status(&entry.queue_id, "running")?;
            let context = match self.merge_queue_context(&entry.source_ref, &entry.target_ref) {
                Ok(context) => context,
                Err(err) => {
                    self.set_merge_queue_status(&entry.queue_id, "failed")?;
                    processed.push(MergeQueueRunItem {
                        queue_id: entry.queue_id,
                        source_ref: entry.source_ref,
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

            match self.merge_queue_entry(&entry) {
                Ok(report) => {
                    self.set_merge_queue_status(&entry.queue_id, "merged")?;
                    self.insert_merge_result(
                        &entry,
                        &context,
                        Some(&report.operation),
                        "merged",
                        None,
                    )?;
                    processed.push(MergeQueueRunItem {
                        queue_id: entry.queue_id,
                        source_ref: report.source_ref,
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
                    self.set_merge_queue_status(&entry.queue_id, status)?;
                    self.insert_merge_result(
                        &entry,
                        &context,
                        None,
                        status,
                        is_conflict.then_some(message.as_str()),
                    )?;
                    processed.push(MergeQueueRunItem {
                        queue_id: entry.queue_id,
                        source_ref: entry.source_ref,
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

        Ok(MergeQueueRunReport {
            processed,
            stopped_on_conflict,
            stopped_on_failure,
        })
    }
}

fn merge_queue_dry_run_next_step(entry: &MergeQueueEntry) -> String {
    let target = entry
        .target_ref
        .strip_prefix(MAIN_REF_PREFIX)
        .unwrap_or(&entry.target_ref);
    if let Some(lane) = entry.source_ref.strip_prefix(LANE_REF_PREFIX) {
        return format!(
            "Inspect conflicts with `crabdb merge-lane {lane} --into {target} --dry-run` or run the queue to record a conflict set."
        );
    }
    let source = entry
        .source_ref
        .strip_prefix(MAIN_REF_PREFIX)
        .unwrap_or(&entry.source_ref);
    format!(
        "Inspect conflicts with `crabdb merge {source} --into {target} --dry-run` or run the queue to record a conflict set."
    )
}

fn merge_queue_readiness_next_steps(lane: &str, readiness: &LaneReadinessReport) -> Vec<String> {
    let mut steps = Vec::new();
    for issue in &readiness.blockers {
        match issue.code.as_str() {
            "dirty_workdir" => {
                steps.push(format!(
                    "Record or discard dirty workdir changes with `crabdb lane record {lane}`."
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
                        "Review pending approvals with `crabdb approvals list --lane {lane}`; unblock this queue item with `crabdb approvals decide {first_id} --decision approved` after review."
                    ));
                } else {
                    steps.push(format!(
                        "Review pending approvals with `crabdb approvals list --lane {lane}`."
                    ));
                }
            }
            "open_conflicts" => {
                steps.push(
                    "Inspect open conflicts with `crabdb conflicts list` and resolve them before retrying."
                        .to_string(),
                );
            }
            "missing_latest_test" | "latest_test_failed" => {
                steps.push(format!(
                    "Run or fix the required test gate with `crabdb lane test {lane} -- <command>`."
                ));
            }
            "missing_latest_eval" | "latest_eval_failed" => {
                steps.push(format!(
                    "Run or fix the required eval gate with `crabdb lane eval {lane} -- <command>`."
                ));
            }
            _ => steps.push(format!("Resolve readiness blocker `{}`.", issue.code)),
        }
    }
    steps
}
