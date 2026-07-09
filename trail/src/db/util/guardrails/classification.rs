use super::*;

pub(crate) fn guardrail_reason(
    code: impl Into<String>,
    severity: impl Into<String>,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> GuardrailReason {
    GuardrailReason {
        code: code.into(),
        severity: severity.into(),
        message: message.into(),
        details,
    }
}

pub(crate) fn guardrail_risk_text(
    action: &str,
    summary: Option<&str>,
    payload: Option<&serde_json::Value>,
) -> String {
    let mut text = action.to_ascii_lowercase();
    if let Some(summary) = summary {
        text.push(' ');
        text.push_str(&summary.to_ascii_lowercase());
    }
    if let Some(payload) = payload {
        text.push(' ');
        text.push_str(&payload.to_string().to_ascii_lowercase());
    }
    text
}

pub(crate) fn classify_guardrail_action(text: &str) -> Vec<GuardrailReason> {
    let mut reasons = Vec::new();
    if contains_any(
        text,
        &[
            "rm -rf /", "rm -rf ~", "mkfs", "dd if=", "shutdown", "reboot", ":(){",
        ],
    ) {
        reasons.push(guardrail_reason(
            "dangerous_command",
            "blocked",
            "action resembles a destructive host-level command",
            None,
        ));
    }
    if contains_any(
        text,
        &[
            "shell",
            "exec",
            "terminal",
            "command",
            "process",
            "subprocess",
        ],
    ) {
        reasons.push(guardrail_reason(
            "shell_action",
            "approval_required",
            "shell or process execution requires human approval",
            None,
        ));
    }
    if contains_any(
        text,
        &[
            "curl", "wget", "http://", "https://", "ssh", "scp", "rsync", "network", "external",
        ],
    ) {
        reasons.push(guardrail_reason(
            "network_action",
            "approval_required",
            "network or external-system access requires human approval",
            None,
        ));
    }
    if contains_any(
        text,
        &["deploy", "release", "publish", "production", "preview"],
    ) {
        reasons.push(guardrail_reason(
            "release_action",
            "approval_required",
            "deployment, release, or publishing actions require human approval",
            None,
        ));
    }
    if contains_any(
        text,
        &[
            "delete",
            "remove",
            "overwrite",
            "force",
            "reset",
            "clean",
            "truncate",
            "chmod",
            "chown",
        ],
    ) {
        reasons.push(guardrail_reason(
            "destructive_action",
            "approval_required",
            "destructive or forceful workspace changes require human approval",
            None,
        ));
    }
    if contains_any(text, &["ignore_add", "ignore_remove", ".trailignore"]) {
        reasons.push(guardrail_reason(
            "policy_change",
            "approval_required",
            "ignore or guardrail policy changes require human approval",
            None,
        ));
    }
    reasons
}

pub(crate) fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}
