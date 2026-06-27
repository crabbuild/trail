use super::*;

pub(crate) fn readiness_issue(
    code: impl Into<String>,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> LaneReadinessIssue {
    LaneReadinessIssue {
        code: code.into(),
        message: message.into(),
        details,
    }
}
