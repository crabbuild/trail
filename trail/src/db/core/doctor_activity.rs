use super::*;

pub(super) fn push_pending_approvals_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    match db.list_lane_approvals(None, Some("pending")) {
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
}

pub(super) fn push_active_leases_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    match db.list_leases(false) {
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
}

pub(super) fn push_merge_queue_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    match db.list_lane_merge_queue() {
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
}

pub(super) fn push_conflicts_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    match db.list_conflicts() {
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
}

pub(super) fn push_lanes_check(db: &Trail, checks: &mut Vec<DoctorCheck>) {
    match db.list_lanes() {
        Ok(lanes) => {
            let mut dirty_lanes = Vec::new();
            let mut missing_workdirs = Vec::new();
            let mut inspect_errors = Vec::new();
            for lane in &lanes {
                if lane.branch.workdir.is_none() {
                    continue;
                }
                match db.lane_status(&lane.branch.lane_id) {
                    Ok(status) if !status.workdir_changed_paths.is_empty() => {
                        dirty_lanes.push(lane.record.name.clone());
                    }
                    Ok(_) => {}
                    Err(Error::WorkspaceNotFound(path)) => {
                        missing_workdirs.push(path.to_string_lossy().to_string());
                    }
                    Err(err) => inspect_errors.push(format!("{}: {err}", lane.record.name)),
                }
            }
            let check_status = if !inspect_errors.is_empty() {
                "error"
            } else if !dirty_lanes.is_empty() || !missing_workdirs.is_empty() {
                "warning"
            } else {
                "ok"
            };
            let message = match check_status {
                "ok" => format!("{} lane branch(es) inspected", lanes.len()),
                "warning" => format!(
                    "{} dirty lane workdir(s), {} missing lane workdir(s)",
                    dirty_lanes.len(),
                    missing_workdirs.len()
                ),
                _ => format!(
                    "{} lane branch(es) could not be inspected",
                    inspect_errors.len()
                ),
            };
            checks.push(doctor_check(
                "lanes",
                check_status,
                message,
                Some(serde_json::json!({
                    "count": lanes.len(),
                    "dirty_lanes": dirty_lanes,
                    "missing_workdirs": missing_workdirs,
                    "errors": inspect_errors
                })),
            ));
        }
        Err(err) => checks.push(doctor_check(
            "lanes",
            "error",
            format!("could not list lanes: {err}"),
            None,
        )),
    }
}
