use super::*;

impl Trail {
    pub(crate) fn normalize_lane_merge_queue_target_ref(&self, target: &str) -> Result<String> {
        let target_ref_name = branch_ref(target);
        if !target_ref_name.starts_with(MAIN_REF_PREFIX) {
            return Err(Error::InvalidInput(
                "merge queue target must be a branch ref".to_string(),
            ));
        }
        self.get_ref(&target_ref_name)?;
        Ok(target_ref_name)
    }

    pub(crate) fn queued_lane_merge_entries(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<LaneMergeQueueEntry>> {
        let sql = "SELECT q.queue_id, q.lane_id, l.name, q.target_ref, q.status, q.priority, q.created_at, q.updated_at \
             FROM lane_merge_queue q JOIN lanes l ON l.lane_id = q.lane_id \
             WHERE q.status = 'queued' ORDER BY q.priority DESC, q.created_at ASC";
        match limit {
            Some(limit) => {
                let mut stmt = self.conn.prepare(&format!("{sql} LIMIT ?1"))?;
                let rows = stmt.query_map(params![limit as i64], lane_merge_queue_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            None => {
                let mut stmt = self.conn.prepare(sql)?;
                let rows = stmt.query_map([], lane_merge_queue_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        }
    }

    pub(crate) fn set_lane_merge_queue_status(&self, queue_id: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE lane_merge_queue SET status = ?1, updated_at = ?2 WHERE queue_id = ?3",
            params![status, now_ts(), queue_id],
        )?;
        Ok(())
    }

    pub(crate) fn lane_merge_queue_entry(
        &mut self,
        entry: &LaneMergeQueueEntry,
    ) -> Result<MergeReport> {
        let target = entry
            .target_ref
            .strip_prefix(MAIN_REF_PREFIX)
            .unwrap_or(&entry.target_ref);
        self.merge_lane_unlocked(&entry.lane_id, target, false, false)
    }

    pub(crate) fn lane_merge_queue_entry_dry_run(
        &mut self,
        entry: &LaneMergeQueueEntry,
    ) -> Result<MergeReport> {
        let target = entry
            .target_ref
            .strip_prefix(MAIN_REF_PREFIX)
            .unwrap_or(&entry.target_ref);
        self.merge_lane_unlocked(&entry.lane_id, target, true, false)
    }

    pub(crate) fn lane_merge_queue_entry_by_selector(
        &self,
        selector: &str,
    ) -> Result<LaneMergeQueueEntry> {
        self.conn
            .query_row(
                "SELECT q.queue_id, q.lane_id, l.name, q.target_ref, q.status, q.priority, q.created_at, q.updated_at \
                 FROM lane_merge_queue q JOIN lanes l ON l.lane_id = q.lane_id \
                 WHERE (q.queue_id = ?1 OR q.lane_id = ?1 OR l.name = ?1) \
                   AND q.status NOT IN ('merged', 'cancelled') \
                 ORDER BY q.status = 'queued' DESC, q.priority DESC, q.created_at ASC LIMIT 1",
                params![selector],
                lane_merge_queue_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("merge queue item `{selector}` not found")))
    }

    pub(crate) fn lane_merge_queue_context(
        &self,
        entry: &LaneMergeQueueEntry,
    ) -> Result<MergeContext> {
        let lane = self.lane_details(&entry.lane_id)?;
        let source_ref_name = &lane.branch.ref_name;
        let target_ref_name = &entry.target_ref;
        let source_ref = self.get_ref(source_ref_name)?;
        let target_ref = self.get_ref(target_ref_name)?;
        let base_change = lane.branch.base_change;
        let base_ref = self.ref_from_change(&base_change)?;
        Ok(MergeContext {
            base_change,
            left_change: target_ref.change_id,
            right_change: source_ref.change_id,
            base_root: base_ref.root_id,
            left_root: target_ref.root_id,
            right_root: source_ref.root_id,
        })
    }

    pub(crate) fn pending_conflict_merge(
        &self,
        conflict_set_id: &str,
    ) -> Result<PendingConflictMerge> {
        self.conn
            .query_row(
                "SELECT merge_id, lane_queue_id, source_ref, target_ref, base_change, left_change, right_change, base_root, left_root, right_root \
                 FROM merge_results WHERE conflict_set = ?1 ORDER BY created_at DESC LIMIT 1",
                params![conflict_set_id],
                |row| {
                    Ok(PendingConflictMerge {
                        merge_id: row.get(0)?,
                        lane_queue_id: row.get(1)?,
                        source_ref: row.get(2)?,
                        target_ref: row.get(3)?,
                        base_change: ChangeId(row.get(4)?),
                        left_change: ChangeId(row.get(5)?),
                        right_change: ChangeId(row.get(6)?),
                        base_root: row.get::<_, Option<String>>(7)?.map(ObjectId),
                        left_root: row.get::<_, Option<String>>(8)?.map(ObjectId),
                        right_root: row.get::<_, Option<String>>(9)?.map(ObjectId),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "conflict set `{conflict_set_id}` is not linked to a merge result"
                ))
            })
    }

    pub(crate) fn insert_merge_result(
        &self,
        entry: &LaneMergeQueueEntry,
        context: &MergeContext,
        result_change: Option<&ChangeId>,
        status: &str,
        conflict_detail: Option<&str>,
    ) -> Result<()> {
        let lane = self.lane_details(&entry.lane_id)?;
        self.insert_merge_result_for_refs(
            Some(&entry.queue_id),
            &lane.branch.ref_name,
            &entry.target_ref,
            context,
            result_change,
            status,
            conflict_detail,
        )?;
        Ok(())
    }

    pub(crate) fn insert_merge_result_for_refs(
        &self,
        lane_queue_id: Option<&str>,
        source_ref: &str,
        target_ref: &str,
        context: &MergeContext,
        result_change: Option<&ChangeId>,
        status: &str,
        conflict_detail: Option<&str>,
    ) -> Result<Option<String>> {
        let created_at = now_ts();
        let seed = format!(
            "{}:{}:{}:{}:{}",
            lane_queue_id.unwrap_or("direct"),
            source_ref,
            target_ref,
            status,
            created_at
        );
        let hash = sha256_hex(seed.as_bytes());
        let merge_id = format!("merge_{}", &hash[..16]);
        let conflict_set = conflict_detail.map(|detail| {
            let conflict_hash = sha256_hex(format!("{merge_id}:{detail}").as_bytes());
            format!("conflict_{}", &conflict_hash[..16])
        });
        let conflict_details_json = conflict_detail
            .map(|detail| {
                let details = detail
                    .split("; ")
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                serde_json::to_string(&details)
            })
            .transpose()?;
        let result_change = result_change.map(|change| change.0.clone());
        self.conn.execute(
            "INSERT INTO merge_results \
             (merge_id, lane_queue_id, source_ref, target_ref, base_change, left_change, right_change, base_root, left_root, right_root, result_change, status, conflict_set, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                merge_id,
                lane_queue_id,
                source_ref,
                target_ref,
                context.base_change.0,
                context.left_change.0,
                context.right_change.0,
                context.base_root.0,
                context.left_root.0,
                context.right_root.0,
                result_change,
                status,
                conflict_set,
                created_at
            ],
        )?;
        if let Some(conflict_set_id) = &conflict_set {
            self.conn.execute(
                "INSERT INTO conflict_sets \
                 (conflict_set_id, merge_id, source_ref, target_ref, status, details_json, created_at) \
                 VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6)",
                params![
                    conflict_set_id,
                    merge_id,
                    source_ref,
                    target_ref,
                    conflict_details_json,
                    created_at
                ],
            )?;
        }
        Ok(conflict_set)
    }

    pub(crate) fn existing_open_conflict_set(
        &self,
        source_ref: &str,
        target_ref: &str,
        context: &MergeContext,
    ) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT mr.conflict_set \
                 FROM merge_results mr \
                 JOIN conflict_sets cs ON cs.conflict_set_id = mr.conflict_set \
                 WHERE mr.source_ref = ?1 \
                   AND mr.target_ref = ?2 \
                   AND mr.base_change = ?3 \
                   AND mr.left_change = ?4 \
                   AND mr.right_change = ?5 \
                   AND mr.status = 'conflicted' \
                   AND cs.status = 'open' \
                 ORDER BY mr.created_at DESC LIMIT 1",
                params![
                    source_ref,
                    target_ref,
                    context.base_change.0,
                    context.left_change.0,
                    context.right_change.0
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }
}
