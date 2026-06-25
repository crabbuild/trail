use super::*;

pub(crate) fn doctor_check(
    name: impl Into<String>,
    status: impl Into<String>,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> DoctorCheck {
    DoctorCheck {
        name: name.into(),
        status: status.into(),
        message: message.into(),
        details,
    }
}

pub(crate) fn doctor_report(checks: Vec<DoctorCheck>) -> DoctorReport {
    let status = if checks.iter().any(|check| check.status == "error") {
        "error"
    } else if checks.iter().any(|check| check.status == "warning") {
        "warning"
    } else {
        "ok"
    };
    DoctorReport {
        status: status.to_string(),
        checks,
    }
}

pub(crate) fn normalize_query_limit(limit: usize, max: usize) -> Result<usize> {
    if limit == 0 {
        return Err(Error::InvalidInput(
            "limit must be greater than 0".to_string(),
        ));
    }
    Ok(limit.min(max))
}
