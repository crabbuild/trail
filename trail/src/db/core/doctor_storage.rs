use super::*;

pub(super) fn push_workspace_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    let workspace_path = db.workspace_root.to_string_lossy().to_string();
    if db.workspace_root.is_dir() {
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
}

pub(super) fn push_database_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    let sqlite_path = db.db_dir.join(DB_RELATIVE_PATH);
    let db_path = db.db_dir.to_string_lossy().to_string();
    let sqlite_path_text = sqlite_path.to_string_lossy().to_string();
    if db.db_dir.is_dir() && sqlite_path.is_file() {
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
                "db_dir_exists": db.db_dir.is_dir(),
                "sqlite": sqlite_path_text,
                "sqlite_exists": sqlite_path.is_file()
            })),
        ));
    }
}

pub(super) fn push_schema_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    match (
        db.schema_user_version(),
        db.schema_meta_value(SCHEMA_META_VERSION_KEY),
    ) {
        (Ok(user_version), Ok(meta_version)) => {
            let meta_version_int = meta_version
                .as_deref()
                .and_then(|value| value.parse::<i64>().ok());
            let details = Some(serde_json::json!({
                "supported_version": TRAIL_SCHEMA_VERSION,
                "sqlite_user_version": user_version,
                "metadata_version": meta_version,
                "app_version": db.schema_meta_value(SCHEMA_META_APP_VERSION_KEY).ok().flatten()
            }));
            if user_version == TRAIL_SCHEMA_VERSION
                && meta_version_int == Some(TRAIL_SCHEMA_VERSION)
            {
                checks.push(doctor_check(
                    "schema_version",
                    "ok",
                    format!("schema version {TRAIL_SCHEMA_VERSION} is current"),
                    details,
                ));
            } else if user_version > TRAIL_SCHEMA_VERSION
                || meta_version_int.is_some_and(|version| version > TRAIL_SCHEMA_VERSION)
            {
                checks.push(doctor_check(
                    "schema_version",
                    "error",
                    "workspace schema is newer than this Trail binary",
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
}

pub(super) fn push_current_branch_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    match db.current_branch() {
        Ok(branch) => match db.resolve_branch_ref(&branch) {
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
}

pub(super) fn push_ignore_policy_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    let trailignore_path = db.workspace_root.join(".trailignore");
    match read_ignore_patterns(&trailignore_path) {
        Ok(patterns) if trailignore_path.exists() => {
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
                    ".trailignore includes Trail's default private and generated paths",
                    Some(serde_json::json!({
                        "path": trailignore_path.to_string_lossy(),
                        "patterns": patterns.len()
                    })),
                ));
            } else {
                checks.push(doctor_check(
                    "ignore_policy",
                    "warning",
                    ".trailignore is missing some default private or generated path rules",
                    Some(serde_json::json!({
                        "path": trailignore_path.to_string_lossy(),
                        "missing": missing
                    })),
                ));
            }
        }
        Ok(_) => checks.push(doctor_check(
            "ignore_policy",
            "warning",
            ".trailignore is missing; lane patches still block Trail's hardcoded denylist",
            Some(serde_json::json!({ "path": trailignore_path.to_string_lossy() })),
        )),
        Err(err) => checks.push(doctor_check(
            "ignore_policy",
            "error",
            format!("could not read .trailignore: {err}"),
            Some(serde_json::json!({ "path": trailignore_path.to_string_lossy() })),
        )),
    }
}

pub(super) fn push_workspace_views_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    let result = (|| -> Result<serde_json::Value> {
        let mut stmt = db.conn.prepare(
            "SELECT view_id, backend, mountpoint, source_upper, generated_upper, scratch_upper, meta_dir, journal_path, status, owner_pid, owner_start_token FROM workspace_views ORDER BY view_id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<u32>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut errors = Vec::new();
        let mut stale_leases = Vec::new();
        for (
            view_id,
            backend,
            mountpoint,
            source,
            generated,
            scratch,
            meta,
            journal,
            status,
            owner_pid,
            owner_token,
        ) in &rows
        {
            for path in [source, generated, scratch, meta] {
                if !Path::new(path).is_dir() {
                    errors.push(format!("{view_id}: missing directory {path}"));
                }
            }
            if let Err(err) = validate_workspace_journal(Path::new(journal)) {
                errors.push(format!("{view_id}: {err}"));
            }
            if matches!(status.as_str(), "failed" | "unhealthy" | "corrupt") {
                errors.push(format!("{view_id}: status is {status}"));
            }
            if let (Some(pid), Some(token)) = (*owner_pid, owner_token.as_deref())
                && !process_matches_start_token(pid, token)
            {
                stale_leases.push(format!("{view_id}:{pid}"));
            }
            let backend_available = match backend.as_str() {
                "fuse" if cfg!(target_os = "linux") => Path::new("/dev/fuse").exists(),
                "fuse" if cfg!(target_os = "macos") => cfg!(feature = "macfuse"),
                "nfs" if cfg!(target_os = "macos") => Path::new("/sbin/mount_nfs").is_file(),
                "dokan" if cfg!(target_os = "windows") => true,
                "clone" | "virtual" => true,
                "fuse" | "nfs" | "dokan" => false,
                _ => false,
            };
            if !backend_available {
                errors.push(format!(
                    "{view_id}: backend {backend} is unavailable for mountpoint {mountpoint}"
                ));
            }
        }
        for layer in db.list_workspace_layers()? {
            if layer.state == "ready" && db.verify_workspace_layer(&layer.layer_id).is_err() {
                errors.push(format!(
                    "{}: immutable layer failed verification",
                    layer.layer_id
                ));
            }
        }
        Ok(serde_json::json!({
            "views": rows.len(),
            "errors": errors,
            "stale_leases": stale_leases,
            "cache_reclaimable_bytes": db.workspace_reclaimable_cache_bytes()?,
        }))
    })();
    match result {
        Ok(details) => {
            let error_count = details["errors"].as_array().map_or(0, Vec::len);
            let stale_count = details["stale_leases"].as_array().map_or(0, Vec::len);
            let (status, message) = if error_count > 0 {
                (
                    "error",
                    format!("{error_count} workspace view integrity problem(s) found"),
                )
            } else if stale_count > 0 {
                (
                    "warning",
                    format!("{stale_count} stale workspace mount lease(s) can be recovered"),
                )
            } else {
                (
                    "ok",
                    "workspace views, layers, journals, and mount backends are healthy".to_string(),
                )
            };
            checks.push(doctor_check(
                "workspace_views",
                status,
                message,
                Some(details),
            ));
        }
        Err(err) => checks.push(doctor_check(
            "workspace_views",
            "error",
            format!("workspace view diagnostics failed: {err}"),
            None,
        )),
    }
}

fn validate_workspace_journal(path: &Path) -> Result<()> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Io(err)),
    };
    let mut previous = 0_u64;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if line.last() != Some(&b'\n') {
            break;
        }
        let value: serde_json::Value = serde_json::from_slice(&line[..line.len() - 1])?;
        let sequence = value["sequence"].as_u64().ok_or_else(|| {
            Error::Corrupt(format!(
                "workspace journal `{}` has a record without a sequence",
                path.display()
            ))
        })?;
        if sequence <= previous {
            return Err(Error::Corrupt(format!(
                "workspace journal `{}` is not strictly ordered",
                path.display()
            )));
        }
        previous = sequence;
    }
    Ok(())
}
