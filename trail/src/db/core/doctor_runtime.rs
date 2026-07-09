use super::*;

pub(super) fn push_write_lock_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    let lock_path = db.db_dir.join("lock");
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
}

pub(super) fn push_daemon_token_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    let token_path = db.db_dir.join("daemon.token");
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
}

pub(super) fn push_fsck_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    match db.fsck() {
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
}
