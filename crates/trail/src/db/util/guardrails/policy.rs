use super::*;

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
