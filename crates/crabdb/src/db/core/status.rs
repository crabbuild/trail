use super::*;

impl CrabDb {
    pub fn status(&self, branch: Option<&str>) -> Result<StatusReport> {
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let head = self.resolve_branch_ref(&branch)?;
        let head_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_worktree_files()?;
        let disk_manifest = self.disk_manifest(&disk_files);
        let changed_paths = self.diff_file_maps_to_manifest(&head_files, &disk_manifest);
        let worktree_state = worktree_state_from_changes(&changed_paths);
        Ok(StatusReport {
            branch,
            head,
            worktree_state,
            changed_paths,
        })
    }

    pub fn doctor(&self) -> Result<DoctorReport> {
        let mut checks = Vec::new();

        let workspace_path = self.workspace_root.to_string_lossy().to_string();
        if self.workspace_root.is_dir() {
            checks.push(doctor_check(
                "workspace",
                "ok",
                format!("workspace root is available at {workspace_path}"),
                Some(serde_json::json!({ "path": workspace_path })),
            ));
        } else {
            checks.push(doctor_check(
                "workspace",
                "error",
                format!("workspace root is missing at {workspace_path}"),
                Some(serde_json::json!({ "path": workspace_path })),
            ));
        }

        let sqlite_path = self.db_dir.join(DB_RELATIVE_PATH);
        let db_path = self.db_dir.to_string_lossy().to_string();
        let sqlite_path_text = sqlite_path.to_string_lossy().to_string();
        if self.db_dir.is_dir() && sqlite_path.is_file() {
            checks.push(doctor_check(
                "database",
                "ok",
                "database directory and SQLite store are present",
                Some(serde_json::json!({
                    "db_dir": db_path,
                    "sqlite": sqlite_path_text
                })),
            ));
        } else {
            checks.push(doctor_check(
                "database",
                "error",
                "database directory or SQLite store is missing",
                Some(serde_json::json!({
                    "db_dir": db_path,
                    "db_dir_exists": self.db_dir.is_dir(),
                    "sqlite": sqlite_path_text,
                    "sqlite_exists": sqlite_path.is_file()
                })),
            ));
        }

        match (
            self.schema_user_version(),
            self.schema_meta_value(SCHEMA_META_VERSION_KEY),
        ) {
            (Ok(user_version), Ok(meta_version)) => {
                let meta_version_int = meta_version
                    .as_deref()
                    .and_then(|value| value.parse::<i64>().ok());
                let details = Some(serde_json::json!({
                    "supported_version": CRABDB_SCHEMA_VERSION,
                    "sqlite_user_version": user_version,
                    "metadata_version": meta_version,
                    "app_version": self.schema_meta_value(SCHEMA_META_APP_VERSION_KEY).ok().flatten()
                }));
                if user_version == CRABDB_SCHEMA_VERSION
                    && meta_version_int == Some(CRABDB_SCHEMA_VERSION)
                {
                    checks.push(doctor_check(
                        "schema_version",
                        "ok",
                        format!("schema version {CRABDB_SCHEMA_VERSION} is current"),
                        details,
                    ));
                } else if user_version > CRABDB_SCHEMA_VERSION
                    || meta_version_int.is_some_and(|version| version > CRABDB_SCHEMA_VERSION)
                {
                    checks.push(doctor_check(
                        "schema_version",
                        "error",
                        "workspace schema is newer than this CrabDB binary",
                        details,
                    ));
                } else {
                    checks.push(doctor_check(
                        "schema_version",
                        "warning",
                        "schema metadata is missing or older than the current version",
                        details,
                    ));
                }
            }
            (Err(err), _) | (_, Err(err)) => checks.push(doctor_check(
                "schema_version",
                "error",
                format!("failed to inspect schema version: {err}"),
                None,
            )),
        }

        match self.current_branch() {
            Ok(branch) => match self.resolve_branch_ref(&branch) {
                Ok(head) => checks.push(doctor_check(
                    "current_branch",
                    "ok",
                    format!("current branch `{branch}` resolves to {}", head.change_id.0),
                    Some(serde_json::json!({
                        "branch": branch,
                        "change_id": head.change_id.0,
                        "root_id": head.root_id.0
                    })),
                )),
                Err(err) => checks.push(doctor_check(
                    "current_branch",
                    "error",
                    format!("current branch `{branch}` does not resolve: {err}"),
                    Some(serde_json::json!({ "branch": branch })),
                )),
            },
            Err(err) => checks.push(doctor_check(
                "current_branch",
                "error",
                format!("could not read current branch: {err}"),
                None,
            )),
        }

        let crabignore_path = self.workspace_root.join(".crabignore");
        match read_ignore_patterns(&crabignore_path) {
            Ok(patterns) if crabignore_path.exists() => {
                let active: BTreeSet<&str> = patterns
                    .iter()
                    .map(|pattern| pattern.pattern.as_str())
                    .collect();
                let missing: Vec<&str> = DEFAULT_CRABIGNORE_PATTERNS
                    .iter()
                    .copied()
                    .filter(|pattern| !active.contains(pattern))
                    .collect();
                if missing.is_empty() {
                    checks.push(doctor_check(
                        "ignore_policy",
                        "ok",
                        ".crabignore includes CrabDB's default private and generated paths",
                        Some(serde_json::json!({
                            "path": crabignore_path.to_string_lossy(),
                            "patterns": patterns.len()
                        })),
                    ));
                } else {
                    checks.push(doctor_check(
                        "ignore_policy",
                        "warning",
                        ".crabignore is missing some default private or generated path rules",
                        Some(serde_json::json!({
                            "path": crabignore_path.to_string_lossy(),
                            "missing": missing
                        })),
                    ));
                }
            }
            Ok(_) => checks.push(doctor_check(
                "ignore_policy",
                "warning",
                ".crabignore is missing; agent patches still block CrabDB's hardcoded denylist",
                Some(serde_json::json!({ "path": crabignore_path.to_string_lossy() })),
            )),
            Err(err) => checks.push(doctor_check(
                "ignore_policy",
                "error",
                format!("could not read .crabignore: {err}"),
                Some(serde_json::json!({ "path": crabignore_path.to_string_lossy() })),
            )),
        }

        let lock_path = self.db_dir.join("lock");
        if lock_path.exists() {
            let holder = fs::read_to_string(&lock_path)
                .unwrap_or_else(|_| "unknown writer".to_string())
                .trim()
                .to_string();
            checks.push(doctor_check(
                "write_lock",
                "warning",
                "workspace write lock file is present",
                Some(serde_json::json!({
                    "path": lock_path.to_string_lossy(),
                    "holder": holder
                })),
            ));
        } else {
            checks.push(doctor_check(
                "write_lock",
                "ok",
                "no workspace write lock file is present",
                Some(serde_json::json!({ "path": lock_path.to_string_lossy() })),
            ));
        }

        let token_path = self.db_dir.join("daemon.token");
        if token_path.exists() {
            match fs::metadata(&token_path) {
                Ok(metadata) if metadata.len() == 0 => checks.push(doctor_check(
                    "daemon_token",
                    "error",
                    "daemon token file exists but is empty",
                    Some(serde_json::json!({ "path": token_path.to_string_lossy() })),
                )),
                Ok(metadata) => {
                    #[cfg(unix)]
                    {
                        let mode = metadata.permissions().mode() & 0o777;
                        if mode & 0o077 != 0 {
                            checks.push(doctor_check(
                                "daemon_token",
                                "warning",
                                format!("daemon token file permissions are {mode:o}; expected no group/other access"),
                                Some(serde_json::json!({
                                    "path": token_path.to_string_lossy(),
                                    "mode": format!("{mode:o}")
                                })),
                            ));
                        } else {
                            checks.push(doctor_check(
                                "daemon_token",
                                "ok",
                                "daemon token file exists with private permissions",
                                Some(serde_json::json!({
                                    "path": token_path.to_string_lossy(),
                                    "mode": format!("{mode:o}")
                                })),
                            ));
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        checks.push(doctor_check(
                            "daemon_token",
                            "ok",
                            "daemon token file exists",
                            Some(serde_json::json!({ "path": token_path.to_string_lossy() })),
                        ));
                    }
                }
                Err(err) => checks.push(doctor_check(
                    "daemon_token",
                    "error",
                    format!("could not inspect daemon token file: {err}"),
                    Some(serde_json::json!({ "path": token_path.to_string_lossy() })),
                )),
            }
        } else {
            checks.push(doctor_check(
                "daemon_token",
                "ok",
                "daemon token has not been created yet; the daemon will create one when auth is enabled",
                Some(serde_json::json!({ "path": token_path.to_string_lossy() })),
            ));
        }

        match self.fsck() {
            Ok(report) if report.errors.is_empty() => checks.push(doctor_check(
                "fsck",
                "ok",
                "refs, roots, text objects, and indexes are internally consistent",
                Some(serde_json::json!({
                    "checked_refs": report.checked_refs,
                    "checked_roots": report.checked_roots,
                    "checked_texts": report.checked_texts
                })),
            )),
            Ok(report) => checks.push(doctor_check(
                "fsck",
                "error",
                format!("fsck found {} error(s)", report.errors.len()),
                Some(serde_json::json!({
                    "checked_refs": report.checked_refs,
                    "checked_roots": report.checked_roots,
                    "checked_texts": report.checked_texts,
                    "errors": report.errors
                })),
            )),
            Err(err) => checks.push(doctor_check(
                "fsck",
                "error",
                format!("fsck failed: {err}"),
                None,
            )),
        }

        match self.list_agent_approvals(None, Some("pending")) {
            Ok(approvals) if approvals.is_empty() => checks.push(doctor_check(
                "pending_approvals",
                "ok",
                "no pending human approval gates",
                Some(serde_json::json!({ "count": 0 })),
            )),
            Ok(approvals) => checks.push(doctor_check(
                "pending_approvals",
                "warning",
                format!("{} human approval gate(s) are pending", approvals.len()),
                Some(serde_json::json!({
                    "count": approvals.len(),
                    "approval_ids": approvals.iter().map(|approval| approval.approval_id.clone()).collect::<Vec<_>>()
                })),
            )),
            Err(err) => checks.push(doctor_check(
                "pending_approvals",
                "error",
                format!("could not list pending approvals: {err}"),
                None,
            )),
        }

        match self.list_leases(false) {
            Ok(leases) => checks.push(doctor_check(
                "active_leases",
                "ok",
                format!("{} active advisory lease(s)", leases.len()),
                Some(serde_json::json!({
                    "count": leases.len(),
                    "lease_ids": leases.iter().map(|lease| lease.lease_id.clone()).collect::<Vec<_>>()
                })),
            )),
            Err(err) => checks.push(doctor_check(
                "active_leases",
                "error",
                format!("could not list active leases: {err}"),
                None,
            )),
        }

        match self.list_merge_queue() {
            Ok(entries) => {
                let queued = entries
                    .iter()
                    .filter(|entry| entry.status == "queued")
                    .count();
                let running = entries
                    .iter()
                    .filter(|entry| entry.status == "running")
                    .count();
                let conflicted = entries
                    .iter()
                    .filter(|entry| entry.status == "conflicted")
                    .count();
                let failed = entries
                    .iter()
                    .filter(|entry| entry.status == "failed")
                    .count();
                let status = if conflicted > 0 || failed > 0 || queued > 0 || running > 0 {
                    "warning"
                } else {
                    "ok"
                };
                let message = if status == "ok" {
                    "merge queue has no pending attention".to_string()
                } else {
                    format!(
                        "merge queue has {queued} queued, {running} running, {conflicted} conflicted, and {failed} failed item(s)"
                    )
                };
                checks.push(doctor_check(
                    "merge_queue",
                    status,
                    message,
                    Some(serde_json::json!({
                        "total": entries.len(),
                        "queued": queued,
                        "running": running,
                        "conflicted": conflicted,
                        "failed": failed
                    })),
                ));
            }
            Err(err) => checks.push(doctor_check(
                "merge_queue",
                "error",
                format!("could not list merge queue: {err}"),
                None,
            )),
        }

        match self.list_conflicts() {
            Ok(conflicts) => {
                let open: Vec<String> = conflicts
                    .iter()
                    .filter(|conflict| conflict.status != "resolved")
                    .map(|conflict| conflict.conflict_set_id.clone())
                    .collect();
                if open.is_empty() {
                    checks.push(doctor_check(
                        "conflicts",
                        "ok",
                        "no open conflict sets",
                        Some(serde_json::json!({ "open": 0 })),
                    ));
                } else {
                    checks.push(doctor_check(
                        "conflicts",
                        "warning",
                        format!("{} conflict set(s) are still open", open.len()),
                        Some(serde_json::json!({
                            "open": open.len(),
                            "conflict_set_ids": open
                        })),
                    ));
                }
            }
            Err(err) => checks.push(doctor_check(
                "conflicts",
                "error",
                format!("could not list conflict sets: {err}"),
                None,
            )),
        }

        match self.list_agents() {
            Ok(agents) => {
                let mut dirty_agents = Vec::new();
                let mut missing_workdirs = Vec::new();
                let mut inspect_errors = Vec::new();
                for agent in &agents {
                    if agent.branch.workdir.is_none() {
                        continue;
                    }
                    match self.agent_status(&agent.branch.agent_id) {
                        Ok(status) if !status.workdir_changed_paths.is_empty() => {
                            dirty_agents.push(agent.record.name.clone());
                        }
                        Ok(_) => {}
                        Err(Error::WorkspaceNotFound(path)) => {
                            missing_workdirs.push(path.to_string_lossy().to_string());
                        }
                        Err(err) => inspect_errors.push(format!("{}: {err}", agent.record.name)),
                    }
                }
                let check_status = if !inspect_errors.is_empty() {
                    "error"
                } else if !dirty_agents.is_empty() || !missing_workdirs.is_empty() {
                    "warning"
                } else {
                    "ok"
                };
                let message = match check_status {
                    "ok" => format!("{} agent branch(es) inspected", agents.len()),
                    "warning" => format!(
                        "{} dirty agent workdir(s), {} missing agent workdir(s)",
                        dirty_agents.len(),
                        missing_workdirs.len()
                    ),
                    _ => format!(
                        "{} agent branch(es) could not be inspected",
                        inspect_errors.len()
                    ),
                };
                checks.push(doctor_check(
                    "agents",
                    check_status,
                    message,
                    Some(serde_json::json!({
                        "count": agents.len(),
                        "dirty_agents": dirty_agents,
                        "missing_workdirs": missing_workdirs,
                        "errors": inspect_errors
                    })),
                ));
            }
            Err(err) => checks.push(doctor_check(
                "agents",
                "error",
                format!("could not list agents: {err}"),
                None,
            )),
        }

        Ok(doctor_report(checks))
    }
}
