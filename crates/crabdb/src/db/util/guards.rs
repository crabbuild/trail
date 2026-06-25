use super::*;

pub(crate) fn readiness_issue(
    code: impl Into<String>,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> AgentReadinessIssue {
    AgentReadinessIssue {
        code: code.into(),
        message: message.into(),
        details,
    }
}

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
    if contains_any(text, &["ignore_add", "ignore_remove", ".crabignore"]) {
        reasons.push(guardrail_reason(
            "policy_change",
            "approval_required",
            "ignore or guardrail policy changes require human approval",
            None,
        ));
    }
    reasons
}

pub(crate) fn apply_configured_guardrail_policy(
    reasons: &mut Vec<GuardrailReason>,
    policy: &str,
    action: &str,
    risk_text: &str,
    path_checks: &[IgnoreCheckReport],
) -> Result<()> {
    let rules = parse_guardrail_policy(policy)?;
    let mut allow_matches = Vec::new();
    let mut approval_matches = Vec::new();
    let mut block_matches = Vec::new();
    for rule in rules {
        if !guardrail_policy_rule_matches(&rule, action, risk_text, path_checks) {
            continue;
        }
        match rule.decision.as_str() {
            "allow" => allow_matches.push(rule),
            "approval" => approval_matches.push(rule),
            "block" => block_matches.push(rule),
            _ => {}
        }
    }

    if !block_matches.is_empty() {
        reasons.push(guardrail_reason(
            "policy_block",
            "blocked",
            "workspace guardrail policy blocks this action",
            Some(serde_json::json!({ "rules": guardrail_policy_rule_details(&block_matches) })),
        ));
    }
    if !approval_matches.is_empty() {
        reasons.push(guardrail_reason(
            "policy_approval",
            "approval_required",
            "workspace guardrail policy requires approval for this action",
            Some(serde_json::json!({ "rules": guardrail_policy_rule_details(&approval_matches) })),
        ));
    }
    if !allow_matches.is_empty() && block_matches.is_empty() && approval_matches.is_empty() {
        reasons.retain(|reason| reason.severity != "approval_required");
        reasons.push(guardrail_reason(
            "policy_allow",
            "info",
            "workspace guardrail policy allows this action",
            Some(serde_json::json!({ "rules": guardrail_policy_rule_details(&allow_matches) })),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct GuardrailPolicyRule {
    decision: String,
    scope: String,
    pattern: String,
}

pub(crate) fn parse_guardrail_policy(policy: &str) -> Result<Vec<GuardrailPolicyRule>> {
    let mut rules = Vec::new();
    for raw_rule in policy.split([';', '\n']) {
        let raw_rule = raw_rule.trim();
        if raw_rule.is_empty() {
            continue;
        }
        let parts = raw_rule.splitn(3, ':').collect::<Vec<_>>();
        if parts.len() != 3 {
            return Err(Error::InvalidInput(format!(
                "guardrails.policy rule `{raw_rule}` must be decision:scope:pattern"
            )));
        }
        let decision = parts[0].trim().to_ascii_lowercase();
        let scope = parts[1].trim().to_ascii_lowercase();
        let pattern = parts[2].trim().to_ascii_lowercase();
        if !matches!(decision.as_str(), "allow" | "approval" | "block") {
            return Err(Error::InvalidInput(format!(
                "guardrails.policy decision must be allow, approval, or block, got `{}`",
                parts[0].trim()
            )));
        }
        if !matches!(scope.as_str(), "action" | "keyword" | "path") {
            return Err(Error::InvalidInput(format!(
                "guardrails.policy scope must be action, keyword, or path, got `{}`",
                parts[1].trim()
            )));
        }
        if pattern.is_empty() {
            return Err(Error::InvalidInput(
                "guardrails.policy pattern cannot be empty".to_string(),
            ));
        }
        rules.push(GuardrailPolicyRule {
            decision,
            scope,
            pattern,
        });
    }
    Ok(rules)
}

pub(crate) fn guardrail_policy_rule_matches(
    rule: &GuardrailPolicyRule,
    action: &str,
    risk_text: &str,
    path_checks: &[IgnoreCheckReport],
) -> bool {
    match rule.scope.as_str() {
        "action" => action.to_ascii_lowercase().contains(&rule.pattern),
        "keyword" => risk_text.contains(&rule.pattern),
        "path" => path_checks
            .iter()
            .any(|check| check.path.to_ascii_lowercase().contains(&rule.pattern)),
        _ => false,
    }
}

pub(crate) fn guardrail_policy_rule_details(
    rules: &[GuardrailPolicyRule],
) -> Vec<serde_json::Value> {
    rules
        .iter()
        .map(|rule| {
            serde_json::json!({
                "decision": rule.decision,
                "scope": rule.scope,
                "pattern": rule.pattern
            })
        })
        .collect()
}

pub(crate) fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

pub(crate) fn handoff_next_steps(
    readiness: &AgentReadinessReport,
    current_session: Option<&AgentSessionDetails>,
) -> Vec<String> {
    let mut steps = Vec::new();
    for blocker in &readiness.blockers {
        match blocker.code.as_str() {
            "agent_removed" => steps
                .push("Restore or respawn the agent branch before continuing the handoff.".into()),
            "dirty_workdir" => steps.push(
                "Record or force-sync the materialized workdir before reviewing or merging.".into(),
            ),
            "pending_approvals" => {
                steps.push("Resolve pending human approvals before merge.".into())
            }
            "open_conflicts" => steps.push("Resolve open conflict sets before merge.".into()),
            "latest_test_failed" => steps.push("Fix and rerun the latest test gate.".into()),
            "latest_eval_failed" => steps.push("Fix and rerun the latest eval gate.".into()),
            "missing_required_test_suite" => {
                steps.push("Run the required named test suite before merge.".into())
            }
            "missing_required_eval_suite" => {
                steps.push("Run the required named eval suite before merge.".into())
            }
            "required_test_suite_failed" => {
                steps.push("Fix and rerun the failed required test suite.".into())
            }
            "required_eval_suite_failed" => {
                steps.push("Fix and rerun the failed required eval suite.".into())
            }
            _ => steps.push(blocker.message.clone()),
        }
    }

    if steps.is_empty() {
        steps.push("Review changed paths, recent operations, and provenance before merge.".into());
    }

    for warning in &readiness.warnings {
        match warning.code.as_str() {
            "missing_latest_test" => {
                steps.push("Run a test gate if this branch should be merged.".into())
            }
            "missing_latest_eval" => {
                steps.push("Run an eval gate when model or policy quality matters.".into())
            }
            "no_changed_paths" => steps
                .push("Confirm this is an audit-only handoff or record the intended work.".into()),
            "queued_merge" => steps.push(
                "Inspect the existing queued or running merge before queuing another.".into(),
            ),
            _ => steps.push(warning.message.clone()),
        }
    }

    match current_session {
        Some(details) if details.session.status == "active" => steps.push(format!(
            "Continue or close active session `{}` after the receiving agent catches up.",
            details.session.session_id
        )),
        Some(details) => steps.push(format!(
            "Use session `{}` as historical context for this handoff.",
            details.session.session_id
        )),
        None => steps
            .push("Start a new session or turn if the receiving agent will continue work.".into()),
    }

    steps.dedup();
    steps
}

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

pub(crate) fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

pub(crate) fn backup_manifest_path(path: &Path) -> PathBuf {
    path.join("manifest.json")
}

pub(crate) fn backup_sqlite_path(path: &Path) -> PathBuf {
    path.join(DB_RELATIVE_PATH)
}

pub(crate) fn read_backup_manifest(path: &Path) -> Result<BackupManifest> {
    let bytes = fs::read(backup_manifest_path(path))?;
    serde_json::from_slice(&bytes).map_err(Error::from)
}

pub(crate) fn file_digest(path: &Path) -> Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((bytes, hex::encode(hasher.finalize())))
}
