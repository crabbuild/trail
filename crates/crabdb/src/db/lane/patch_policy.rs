use super::*;

impl CrabDb {
    pub(crate) fn ensure_lane_patch_policy(
        &self,
        branch: &LaneBranch,
        patch: &PatchDocument,
        touched_paths: &[String],
    ) -> Result<()> {
        self.ensure_patch_size_limits(patch)?;
        self.ensure_patch_secret_scan(patch)?;
        self.ensure_lane_mutation_paths_allowed(branch, touched_paths)
    }

    pub(crate) fn ensure_patch_final_root_paths_safe(
        &self,
        head_root_id: &ObjectId,
        previous_touched: &BTreeMap<String, FileEntry>,
        target_touched: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        let mut paths = self
            .load_root_paths(head_root_id)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        for path in previous_touched.keys() {
            paths.remove(path);
        }
        paths.extend(target_touched.keys().cloned());
        validate_no_case_fold_collisions(paths.iter())
    }

    pub(crate) fn ensure_lane_record_policy(
        &self,
        branch: &LaneBranch,
        summaries: &[FileDiffSummary],
    ) -> Result<()> {
        let paths = diff_summary_policy_paths(summaries)?;
        self.ensure_lane_mutation_paths_allowed(branch, &paths)
    }

    pub(crate) fn ensure_record_final_root_paths_safe_from_summaries(
        &self,
        head_root_id: &ObjectId,
        summaries: &[FileDiffSummary],
    ) -> Result<()> {
        let mut paths = self
            .load_root_paths(head_root_id)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        for summary in summaries {
            let path = normalize_relative_path(&summary.path)?;
            if let Some(old_path) = &summary.old_path {
                paths.remove(&normalize_relative_path(old_path)?);
            }
            paths.remove(&path);
            if summary.kind != FileChangeKind::Deleted {
                paths.insert(path);
            }
        }
        validate_no_case_fold_collisions(paths.iter())
    }

    pub(crate) fn preview_lane_record_policy(
        &self,
        branch: &LaneBranch,
        summaries: &[FileDiffSummary],
    ) -> Result<LaneRecordPolicyPreview> {
        let paths = diff_summary_policy_paths(summaries)?;
        self.preview_lane_mutation_paths_policy(branch, &paths)
    }

    pub(crate) fn ensure_patch_edit_allowed(
        &self,
        edit: &PatchEdit,
        allow_ignored: bool,
    ) -> Result<()> {
        match edit {
            PatchEdit::Write { path, .. }
            | PatchEdit::WriteBytes { path, .. }
            | PatchEdit::Delete { path } => {
                let path = normalize_relative_path(path)?;
                self.ensure_patch_path_allowed(&path, allow_ignored)
            }
            PatchEdit::ReplaceLine {
                path,
                expected_text,
                ..
            } => {
                let path = normalize_relative_path(path)?;
                self.ensure_patch_path_allowed(&path, allow_ignored)?;
                if expected_text.is_none() {
                    return Err(Error::PatchRejected(format!(
                        "replace_line for `{path}` requires expected_text; include the current line text so stale edits are rejected"
                    )));
                }
                Ok(())
            }
            PatchEdit::Rename { from, to } => {
                let from = normalize_relative_path(from)?;
                let to = normalize_relative_path(to)?;
                self.ensure_patch_path_allowed(&from, allow_ignored)?;
                self.ensure_patch_path_allowed(&to, allow_ignored)
            }
        }
    }

    pub(crate) fn ensure_patch_path_allowed(&self, path: &str, allow_ignored: bool) -> Result<()> {
        if is_internal_path(path) {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        if allow_ignored {
            return Ok(());
        }
        let report = self.ignore_check(path)?;
        if report.ignored {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        Ok(())
    }

    fn ensure_patch_size_limits(&self, patch: &PatchDocument) -> Result<()> {
        let max_patch_bytes = self.config.lane.max_patch_bytes;
        if max_patch_bytes > 0 {
            let patch_bytes = serde_json::to_vec(patch)?.len() as u64;
            if patch_bytes > max_patch_bytes {
                return Err(Error::PatchRejected(format!(
                    "patch payload is {patch_bytes} bytes, exceeding lane.max_patch_bytes {max_patch_bytes}"
                )));
            }
        }

        let max_file_bytes = self.config.lane.max_patch_file_bytes;
        if max_file_bytes == 0 {
            return Ok(());
        }
        for edit in &patch.edits {
            let (path, size_bytes) = match edit {
                PatchEdit::Write { path, content, .. } => {
                    (path.as_str(), content.as_bytes().len() as u64)
                }
                PatchEdit::WriteBytes {
                    path, bytes_hex, ..
                } => (path.as_str(), bytes_hex.len().div_ceil(2) as u64),
                PatchEdit::ReplaceLine { path, new_text, .. } => {
                    (path.as_str(), new_text.as_bytes().len() as u64)
                }
                PatchEdit::Delete { .. } | PatchEdit::Rename { .. } => continue,
            };
            if size_bytes > max_file_bytes {
                let path = normalize_relative_path(path)?;
                return Err(Error::PatchRejected(format!(
                    "patch edit for `{path}` is {size_bytes} bytes, exceeding lane.max_patch_file_bytes {max_file_bytes}"
                )));
            }
        }
        Ok(())
    }

    fn ensure_patch_secret_scan(&self, patch: &PatchDocument) -> Result<()> {
        if let Some(message) = &patch.message {
            if contains_sensitive_text(message) {
                return Err(Error::PatchRejected(
                    "secret scan rejected patch message; remove credentials from the patch payload"
                        .to_string(),
                ));
            }
        }
        for edit in &patch.edits {
            match edit {
                PatchEdit::Write { path, content, .. } => {
                    self.ensure_patch_text_has_no_secrets(path, "content", content)?;
                }
                PatchEdit::WriteBytes {
                    path, bytes_hex, ..
                } => {
                    let bytes = hex::decode(bytes_hex).map_err(|err| {
                        let path = normalize_relative_path(path).unwrap_or_else(|_| path.clone());
                        Error::PatchRejected(format!("invalid bytes_hex for `{path}`: {err}"))
                    })?;
                    let content = String::from_utf8_lossy(&bytes);
                    self.ensure_patch_text_has_no_secrets(path, "bytes", &content)?;
                }
                PatchEdit::ReplaceLine {
                    path,
                    expected_text,
                    new_text,
                    ..
                } => {
                    if let Some(expected_text) = expected_text {
                        self.ensure_patch_text_has_no_secrets(
                            path,
                            "expected_text",
                            expected_text,
                        )?;
                    }
                    self.ensure_patch_text_has_no_secrets(path, "new_text", new_text)?;
                }
                PatchEdit::Delete { .. } | PatchEdit::Rename { .. } => {}
            }
        }
        Ok(())
    }

    fn ensure_patch_text_has_no_secrets(&self, path: &str, field: &str, value: &str) -> Result<()> {
        if !contains_sensitive_text(value) {
            return Ok(());
        }
        let path = normalize_relative_path(path)?;
        Err(Error::PatchRejected(format!(
            "secret scan rejected patch {field} for `{path}`; remove credentials from the patch payload"
        )))
    }

    fn ensure_lane_mutation_paths_allowed(
        &self,
        branch: &LaneBranch,
        paths: &[String],
    ) -> Result<()> {
        let paths = normalize_policy_paths(paths)?;
        if paths.is_empty() {
            return Ok(());
        }
        validate_no_case_fold_collisions(paths.iter())?;
        self.ensure_changed_path_limit(paths.len())?;
        self.ensure_sparse_policy_paths_allowed(branch, &paths)?;
        self.ensure_claim_policy_paths_allowed(branch, &paths)
    }

    fn preview_lane_mutation_paths_policy(
        &self,
        branch: &LaneBranch,
        paths: &[String],
    ) -> Result<LaneRecordPolicyPreview> {
        let paths = normalize_policy_paths(paths)?;
        let mut preview = LaneRecordPolicyPreview {
            allowed: true,
            warnings: Vec::new(),
            error: None,
        };
        if paths.is_empty() {
            return Ok(preview);
        }
        validate_no_case_fold_collisions(paths.iter())?;

        let max_changed_paths = self.config.lane.max_changed_paths;
        if max_changed_paths > 0 && paths.len() as u64 > max_changed_paths {
            preview.allowed = false;
            preview.error = Some(format!(
                "lane mutation touches {} path(s), exceeding lane.max_changed_paths {max_changed_paths}",
                paths.len()
            ));
            return Ok(preview);
        }

        if self.config.lane.enforce_sparse_paths {
            if let Some(workdir) = &branch.workdir {
                if let Some(allowlist) =
                    self.lane_sparse_workdir_paths(branch, Path::new(workdir))?
                {
                    let blocked = paths
                        .iter()
                        .filter(|path| {
                            !allowlist
                                .iter()
                                .any(|selected| path_matches_selection(path, selected))
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if !blocked.is_empty() {
                        preview.allowed = false;
                        preview.error = Some(format!(
                            "lane sparse path boundary blocks path(s): {}; allowed path(s): {}",
                            blocked.join(", "),
                            allowlist.join(", ")
                        ));
                        return Ok(preview);
                    }
                }
            }
        }

        let enforcement = self.config.lane.claim_enforcement.as_str();
        if enforcement == "off" {
            return Ok(preview);
        }
        let claims = self.active_write_claim_paths(&branch.lane_id)?;
        let unclaimed = paths
            .iter()
            .filter(|path| !path_is_covered_by_claims(path, &claims))
            .cloned()
            .collect::<Vec<_>>();
        if unclaimed.is_empty() {
            return Ok(preview);
        }
        let lane = branch
            .ref_name
            .strip_prefix(LANE_REF_PREFIX)
            .unwrap_or(&branch.lane_id);
        let message = format!(
            "lane `{lane}` mutation touches path(s) outside active write claims/leases: {}",
            unclaimed.join(", ")
        );
        match enforcement {
            "warn" => preview.warnings.push(message),
            "reject" => {
                preview.allowed = false;
                preview.error = Some(message);
            }
            other => {
                preview.allowed = false;
                preview.error = Some(format!(
                    "lane.claim_enforcement must be off, warn, or reject, got `{other}`"
                ));
            }
        }
        Ok(preview)
    }

    fn ensure_changed_path_limit(&self, path_count: usize) -> Result<()> {
        let max_changed_paths = self.config.lane.max_changed_paths;
        if max_changed_paths > 0 && path_count as u64 > max_changed_paths {
            return Err(Error::PatchRejected(format!(
                "lane mutation touches {path_count} path(s), exceeding lane.max_changed_paths {max_changed_paths}"
            )));
        }
        Ok(())
    }

    fn ensure_sparse_policy_paths_allowed(
        &self,
        branch: &LaneBranch,
        paths: &[String],
    ) -> Result<()> {
        if !self.config.lane.enforce_sparse_paths {
            return Ok(());
        }
        let Some(workdir) = &branch.workdir else {
            return Ok(());
        };
        let Some(allowlist) = self.lane_sparse_workdir_paths(branch, Path::new(workdir))? else {
            return Ok(());
        };
        let blocked = paths
            .iter()
            .filter(|path| {
                !allowlist
                    .iter()
                    .any(|selected| path_matches_selection(path, selected))
            })
            .cloned()
            .collect::<Vec<_>>();
        if blocked.is_empty() {
            return Ok(());
        }
        Err(Error::PatchRejected(format!(
            "lane sparse path boundary blocks path(s): {}; allowed path(s): {}",
            blocked.join(", "),
            allowlist.join(", ")
        )))
    }

    fn ensure_claim_policy_paths_allowed(
        &self,
        branch: &LaneBranch,
        paths: &[String],
    ) -> Result<()> {
        let enforcement = self.config.lane.claim_enforcement.as_str();
        if enforcement == "off" {
            return Ok(());
        }
        let claims = self.active_write_claim_paths(&branch.lane_id)?;
        let unclaimed = paths
            .iter()
            .filter(|path| !path_is_covered_by_claims(path, &claims))
            .cloned()
            .collect::<Vec<_>>();
        if unclaimed.is_empty() {
            return Ok(());
        }
        let lane = branch
            .ref_name
            .strip_prefix(LANE_REF_PREFIX)
            .unwrap_or(&branch.lane_id);
        let message = format!(
            "lane `{lane}` mutation touches path(s) outside active write claims/leases: {}",
            unclaimed.join(", ")
        );
        match enforcement {
            "warn" => {
                self.insert_lane_event(
                    &branch.lane_id,
                    "lane_policy_warning",
                    Some(&branch.head_change),
                    None,
                    &serde_json::json!({
                        "code": "unclaimed_paths",
                        "message": message,
                        "paths": unclaimed
                    }),
                )?;
                Ok(())
            }
            "reject" => Err(Error::PatchRejected(message)),
            other => Err(Error::InvalidInput(format!(
                "lane.claim_enforcement must be off, warn, or reject, got `{other}`"
            ))),
        }
    }

    fn active_write_claim_paths(&self, lane_id: &str) -> Result<Vec<Option<String>>> {
        let mut stmt = self.conn.prepare(
            "SELECT path FROM leases \
             WHERE lane_id = ?1 AND mode = 'write' AND expires_at > ?2 \
             ORDER BY path ASC",
        )?;
        let rows = stmt.query_map(params![lane_id, now_ts()], |row| {
            row.get::<_, Option<String>>(0)
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
}

fn normalize_policy_paths(paths: &[String]) -> Result<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for path in paths {
        normalized.insert(normalize_relative_path(path)?);
    }
    Ok(normalized.into_iter().collect())
}

fn diff_summary_policy_paths(summaries: &[FileDiffSummary]) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for summary in summaries {
        paths.push(normalize_relative_path(&summary.path)?);
        if let Some(old_path) = &summary.old_path {
            paths.push(normalize_relative_path(old_path)?);
        }
    }
    normalize_policy_paths(&paths)
}

fn path_is_covered_by_claims(path: &str, claims: &[Option<String>]) -> bool {
    claims.iter().any(|claim| match claim {
        Some(claim) => path_matches_selection(path, claim),
        None => true,
    })
}
