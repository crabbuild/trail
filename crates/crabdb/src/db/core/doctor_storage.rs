use super::*;

pub(super) fn push_workspace_check(db: &CrabDb, checks: &mut Vec<DoctorCheck>) {
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

pub(super) fn push_database_check(db: &CrabDb, checks: &mut Vec<DoctorCheck>) {
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

pub(super) fn push_schema_check(db: &CrabDb, checks: &mut Vec<DoctorCheck>) {
    match (
        db.schema_user_version(),
        db.schema_meta_value(SCHEMA_META_VERSION_KEY),
    ) {
        (Ok(user_version), Ok(meta_version)) => {
            let meta_version_int = meta_version
                .as_deref()
                .and_then(|value| value.parse::<i64>().ok());
            let details = Some(serde_json::json!({
                "supported_version": CRABDB_SCHEMA_VERSION,
                "sqlite_user_version": user_version,
                "metadata_version": meta_version,
                "app_version": db.schema_meta_value(SCHEMA_META_APP_VERSION_KEY).ok().flatten()
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
}

pub(super) fn push_current_branch_check(db: &CrabDb, checks: &mut Vec<DoctorCheck>) {
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

pub(super) fn push_ignore_policy_check(db: &CrabDb, checks: &mut Vec<DoctorCheck>) {
    let crabignore_path = db.workspace_root.join(".crabignore");
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
            ".crabignore is missing; lane patches still block CrabDB's hardcoded denylist",
            Some(serde_json::json!({ "path": crabignore_path.to_string_lossy() })),
        )),
        Err(err) => checks.push(doctor_check(
            "ignore_policy",
            "error",
            format!("could not read .crabignore: {err}"),
            Some(serde_json::json!({ "path": crabignore_path.to_string_lossy() })),
        )),
    }
}
