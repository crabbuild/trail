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
        let agent_candidate = agent_ref(selector);
        let branch_candidate = branch_ref(selector);
        let entry = self
            .conn
            .query_row(
                "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
                 FROM merge_queue \
                 WHERE (queue_id = ?1 OR source_ref = ?1 OR source_ref = ?2 OR source_ref = ?3) \
                   AND status NOT IN ('merged', 'cancelled') \
                 ORDER BY priority DESC, created_at ASC LIMIT 1",
                params![selector, agent_candidate, branch_candidate],
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
