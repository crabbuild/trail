use super::*;

impl Trail {
    pub fn rewind_lane(
        &mut self,
        lane: &str,
        target: &str,
        record_current: bool,
        sync_workdir: bool,
    ) -> Result<LaneRewindReport> {
        validate_ref_segment(lane)?;
        if target.trim().is_empty() {
            return Err(Error::InvalidInput(
                "rewind target cannot be empty".to_string(),
            ));
        }

        let mut recorded_current = None;
        if record_current {
            let branch = self.lane_branch(lane)?;
            if branch
                .workdir
                .as_deref()
                .is_some_and(|workdir| Path::new(workdir).is_dir())
            {
                let report = self.record_lane_workdir(
                    lane,
                    Some(format!("Record current lane `{lane}` before rewind")),
                )?;
                recorded_current = report.operation;
            }
        }

        let mut report = {
            let _lock = self.acquire_write_lock()?;
            let branch = self.lane_branch(lane)?;
            let head = self.get_ref(&branch.ref_name)?;
            if sync_workdir {
                self.ensure_lane_workdir_clean(&branch, &head)?;
            }

            let target_ref = self.resolve_refish(target)?;
            if head.change_id == target_ref.change_id && head.root_id == target_ref.root_id {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` is already at `{target}`"
                )));
            }

            let (preserved_branch, preserved_ref) = if record_current {
                let preserved_branch = self.preserve_rewind_head_branch(lane, &head)?;
                let preserved_ref = branch_ref(&preserved_branch);
                (Some(preserved_branch), Some(preserved_ref))
            } else {
                (None, None)
            };

            let current_files = self.load_root_files(&head.root_id)?;
            let target_files = self.load_root_files(&target_ref.root_id)?;
            let diff = self.diff_file_maps(&current_files, &target_files)?;
            let actor = Actor::system();
            let change_id = self.allocate_change_id(&actor.id, "lane_rewind")?;
            let message = match &preserved_branch {
                Some(branch) => format!(
                    "Rewind lane `{lane}` to `{target}`; preserved previous head as `{branch}`"
                ),
                None => format!("Rewind lane `{lane}` to `{target}`"),
            };
            let operation = Operation {
                version: OP_OBJECT_VERSION,
                change_id: change_id.clone(),
                kind: OperationKind::LaneRewind,
                parents: vec![head.change_id.clone()],
                before_root: Some(head.root_id.clone()),
                after_root: target_ref.root_id.clone(),
                branch: branch.ref_name.clone(),
                actor,
                session_id: branch.session_id.clone(),
                message: Some(message),
                changes: diff.changes,
                created_at: now_ts(),
            };
            let operation_id = self.store_operation(&operation)?;
            self.advance_ref_cas(&head, &change_id, &target_ref.root_id, &operation_id)?;
            self.conn.execute(
                "UPDATE lane_branches SET head_change = ?1, head_root = ?2, status = 'active', updated_at = ?3 WHERE lane_id = ?4",
                params![change_id.0, target_ref.root_id.0, now_ts(), branch.lane_id],
            )?;
            self.insert_lane_event_with_context(
                &branch.lane_id,
                branch.session_id.as_deref(),
                None,
                "lane_rewound",
                Some(&change_id),
                None,
                &serde_json::json!({
                    "target": target,
                    "target_change": target_ref.change_id.0,
                    "target_root": target_ref.root_id.0,
                    "previous_change": head.change_id.0,
                    "previous_root": head.root_id.0,
                    "record_current": record_current,
                    "recorded_current": recorded_current.as_ref().map(|id| id.0.clone()),
                    "preserved_branch": preserved_branch.clone(),
                    "preserved_ref": preserved_ref.clone(),
                    "sync_workdir": sync_workdir,
                    "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
                }),
            )?;

            LaneRewindReport {
                lane_id: branch.lane_id,
                ref_name: branch.ref_name,
                target: target.to_string(),
                previous_change: head.change_id,
                previous_root: head.root_id,
                target_change: target_ref.change_id,
                target_root: target_ref.root_id.clone(),
                operation: change_id,
                root_id: target_ref.root_id,
                changed_paths: diff.summaries,
                recorded_current,
                preserved_branch,
                preserved_ref,
                workdir: branch.workdir,
                workdir_synced: false,
            }
        };

        if sync_workdir && report.workdir.is_some() {
            let sync = self.sync_lane_workdir(lane, true)?;
            report.workdir = Some(sync.workdir);
            report.workdir_synced = true;
        }

        Ok(report)
    }

    fn preserve_rewind_head_branch(&self, lane: &str, head: &RefRecord) -> Result<String> {
        let short_change = short_rewind_change_id(&head.change_id);
        for suffix in 0..1000u16 {
            let branch = if suffix == 0 {
                format!("rewind/{lane}/{short_change}")
            } else {
                format!("rewind/{lane}/{short_change}-{suffix}")
            };
            validate_ref_segment(&branch)?;
            let ref_name = branch_ref(&branch);
            if self.try_get_ref(&ref_name)?.is_none() {
                self.set_ref(
                    &ref_name,
                    &head.change_id,
                    &head.root_id,
                    &head.operation_id,
                )?;
                return Ok(branch);
            }
        }
        Err(Error::InvalidInput(format!(
            "could not allocate rewind preservation branch for `{lane}`"
        )))
    }
}

fn short_rewind_change_id(change_id: &ChangeId) -> String {
    crate::ids::change_id_hash(&change_id.0)
        .unwrap_or(&change_id.0)
        .chars()
        .take(12)
        .collect()
}
