use super::*;

impl CrabDb {
    pub fn guardrail_check(
        &self,
        agent: Option<&str>,
        action: &str,
        summary: Option<&str>,
        payload: Option<serde_json::Value>,
        paths: &[String],
    ) -> Result<GuardrailCheckReport> {
        let action = action.trim();
        if action.is_empty() {
            return Err(Error::InvalidInput(
                "guardrail action cannot be empty".to_string(),
            ));
        }
        let summary = summary
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(redact_sensitive_text);
        let payload = payload.map(redact_sensitive_json);
        let agent_details = agent.map(|agent| self.agent_details(agent)).transpose()?;
        let agent_name = agent_details
            .as_ref()
            .map(|details| details.record.name.clone())
            .or_else(|| agent.map(str::to_string));
        let approvals = if let Some(agent) = agent {
            self.list_agent_approvals(Some(agent), None)?
        } else {
            Vec::new()
        };
        let pending_approvals = approvals
            .iter()
            .filter(|approval| approval.status == "pending")
            .cloned()
            .collect::<Vec<_>>();

        let mut reasons = Vec::new();
        let mut path_checks = Vec::new();
        for path in paths {
            let check = self.ignore_check(path)?;
            if check.ignored {
                match check.source.as_deref() {
                    Some("hardcoded") => reasons.push(guardrail_reason(
                        "blocked_path",
                        "blocked",
                        format!(
                            "`{}` is protected by CrabDB's hardcoded private path denylist",
                            check.path
                        ),
                        Some(serde_json::json!({ "path": check.path, "source": check.source })),
                    )),
                    _ => reasons.push(guardrail_reason(
                        "ignored_path",
                        "approval_required",
                        format!(
                            "`{}` is ignored by workspace policy and needs explicit approval or allow_ignored",
                            check.path
                        ),
                        Some(serde_json::json!({ "path": check.path, "source": check.source })),
                    )),
                }
            }
            path_checks.push(check);
        }

        let risk_text = guardrail_risk_text(action, summary.as_deref(), payload.as_ref());
        for reason in classify_guardrail_action(&risk_text) {
            reasons.push(reason);
        }
        apply_configured_guardrail_policy(
            &mut reasons,
            &self.config.guardrails.policy,
            action,
            &risk_text,
            &path_checks,
        )?;

        let matching_pending = pending_approvals
            .iter()
            .filter(|approval| approval.action == action)
            .map(|approval| approval.approval_id.clone())
            .collect::<Vec<_>>();
        if !matching_pending.is_empty() {
            reasons.push(guardrail_reason(
                "pending_approval",
                "approval_required",
                "matching human approval is already pending",
                Some(serde_json::json!({ "approval_ids": matching_pending })),
            ));
        }

        let latest_decided_matching_approval = approvals.iter().find(|approval| {
            approval.action == action && matches!(approval.status.as_str(), "approved" | "rejected")
        });
        let mut satisfied_approvals = Vec::new();
        if matching_pending.is_empty() {
            if let Some(approval) = latest_decided_matching_approval {
                match approval.status.as_str() {
                    "approved" => {
                        let approval_ids = vec![approval.approval_id.clone()];
                        for reason in reasons
                            .iter_mut()
                            .filter(|reason| reason.severity == "approval_required")
                        {
                            let original_details = reason.details.take();
                            reason.severity = "allowed".to_string();
                            reason.details = Some(serde_json::json!({
                                "approval_ids": approval_ids.clone(),
                                "original_severity": "approval_required",
                                "original_details": original_details
                            }));
                        }
                        reasons.push(guardrail_reason(
                            "approval_satisfied",
                            "allowed",
                            "matching approved human approval satisfies approval-required guardrails",
                            Some(serde_json::json!({ "approval_ids": approval_ids.clone() })),
                        ));
                        satisfied_approvals.push(approval.clone());
                    }
                    "rejected" => {
                        reasons.push(guardrail_reason(
                            "approval_rejected",
                            "blocked",
                            "matching human approval was rejected",
                            Some(serde_json::json!({
                                "approval_id": approval.approval_id.clone(),
                                "reviewer": approval.reviewer.clone(),
                                "note": approval.note.clone()
                            })),
                        ));
                    }
                    _ => {}
                }
            }
        }

        let decision = if reasons.iter().any(|reason| reason.severity == "blocked") {
            "blocked"
        } else if reasons
            .iter()
            .any(|reason| reason.severity == "approval_required")
        {
            "approval_required"
        } else {
            "allowed"
        }
        .to_string();

        let approval_request =
            (decision == "approval_required").then(|| GuardrailApprovalRequest {
                agent: agent_name,
                action: action.to_string(),
                summary: summary
                    .clone()
                    .unwrap_or_else(|| format!("Approve `{action}`")),
                payload: payload.clone(),
            });

        Ok(GuardrailCheckReport {
            agent: agent_details,
            action: action.to_string(),
            summary,
            decision,
            reasons,
            path_checks,
            pending_approvals,
            satisfied_approvals,
            approval_request,
        })
    }
}
