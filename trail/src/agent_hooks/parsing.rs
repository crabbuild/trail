use serde::{Deserialize, Serialize};

use super::*;
use crate::{
    AgentCaptureTransport, AgentEventCorrelation, AgentEventEvidence, AgentEvidenceConfidence,
    AgentLifecycleEvent, AgentLifecycleEventType, AgentNativeEventIdentity,
    AGENT_LIFECYCLE_EVENT_SCHEMA, AGENT_LIFECYCLE_EVENT_VERSION,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentHookParseContext {
    pub receipt_id: String,
    pub workspace_id: String,
    pub lane_id: Option<String>,
    pub capture_run_id: Option<String>,
    pub provider_version: Option<String>,
    pub raw_digest: String,
    pub received_at: i64,
    pub transport: AgentCaptureTransport,
}

pub fn parse_agent_hook_payload(
    registry: &AgentProviderRegistry,
    provider: &str,
    native_event: &str,
    payload: &serde_json::Value,
    context: AgentHookParseContext,
) -> Result<Vec<AgentLifecycleEvent>> {
    let manifest = registry.resolve(provider)?;
    let binding = manifest
        .events
        .iter()
        .find(|binding| binding.native_event == native_event)
        .or_else(|| {
            manifest
                .events
                .iter()
                .find(|binding| binding.native_event.eq_ignore_ascii_case(native_event))
        });
    let normalized = match binding {
        Some(binding) => select_normalized_events(&binding.normalized_events, payload),
        None => vec![format!(
            "provider.{}.{}",
            manifest.provider,
            normalize_provider_event_name(native_event)
        )],
    };
    let native = parse_native_identity(native_event, payload);
    let occurred_at = payload_i64(payload, &["timestamp", "occurred_at", "occurredAt"]);
    let correlation = AgentEventCorrelation {
        parent_event_id: payload_string(payload, &["parent_event_id", "parentEventId"]),
        trace_id: payload_string(payload, &["trace_id", "traceId"]),
        span_id: payload_string(payload, &["span_id", "spanId"]),
        parent_span_id: payload_string(payload, &["parent_span_id", "parentSpanId"]),
    };

    normalized
        .into_iter()
        .enumerate()
        .map(|(index, event_type)| {
            let event = AgentLifecycleEvent {
                schema: AGENT_LIFECYCLE_EVENT_SCHEMA.to_string(),
                version: AGENT_LIFECYCLE_EVENT_VERSION,
                event_id: format!("{}_{}", context.receipt_id, index),
                event_type: AgentLifecycleEventType::new(event_type),
                occurred_at,
                received_at: context.received_at,
                provider: manifest.provider.clone(),
                provider_version: context.provider_version.clone(),
                transport: context.transport,
                workspace_id: context.workspace_id.clone(),
                lane_id: context.lane_id.clone(),
                capture_run_id: context.capture_run_id.clone(),
                native: native.clone(),
                correlation: correlation.clone(),
                payload: payload.clone(),
                evidence: AgentEventEvidence {
                    receipt_id: context.receipt_id.clone(),
                    raw_digest: Some(context.raw_digest.clone()),
                    transcript_offset: payload_u64(
                        payload,
                        &["transcript_offset", "transcriptOffset"],
                    ),
                    confidence: AgentEvidenceConfidence::NativeStructured,
                },
            };
            event.validate()?;
            Ok(event)
        })
        .collect()
}

fn parse_native_identity(
    native_event: &str,
    payload: &serde_json::Value,
) -> AgentNativeEventIdentity {
    AgentNativeEventIdentity {
        session_id: payload_string(
            payload,
            &[
                "session_id",
                "sessionId",
                "sessionID",
                "conversation_id",
                "conversationId",
                "thread_id",
                "threadId",
            ],
        ),
        turn_id: payload_string(payload, &["turn_id", "turnId"]),
        message_id: payload_string(payload, &["message_id", "messageId", "part_id", "partId"]),
        tool_id: payload_string(
            payload,
            &[
                "tool_use_id",
                "toolUseId",
                "tool_call_id",
                "toolCallId",
                "tool_id",
                "toolId",
                "call_id",
                "callId",
            ],
        ),
        subagent_id: payload_string(
            payload,
            &["agent_id", "agentId", "subagent_id", "subagentId"],
        ),
        event_name: native_event.to_string(),
        sequence: payload_u64(
            payload,
            &["sequence", "seq", "event_sequence", "eventSequence"],
        ),
    }
}

fn select_normalized_events(candidates: &[String], payload: &serde_json::Value) -> Vec<String> {
    if candidates.len() <= 1 {
        return candidates.to_vec();
    }

    if contains_any(
        candidates,
        &["turn.completed", "turn.failed", "turn.cancelled"],
    ) {
        let outcome = payload_outcome(payload);
        let selected = match outcome {
            ParsedOutcome::Completed => "turn.completed",
            ParsedOutcome::Failed => "turn.failed",
            ParsedOutcome::Cancelled => "turn.cancelled",
        };
        return selected_with_supplemental(
            candidates,
            &["turn.completed", "turn.failed", "turn.cancelled"],
            selected,
        );
    }
    if contains_any(candidates, &["tool.completed", "tool.failed"]) {
        return vec![if payload_failed(payload) {
            "tool.failed"
        } else {
            "tool.completed"
        }
        .to_string()];
    }
    if contains_any(candidates, &["subagent.completed", "subagent.failed"]) {
        return vec![if payload_failed(payload) {
            "subagent.failed"
        } else {
            "subagent.completed"
        }
        .to_string()];
    }
    if contains_any(candidates, &["session.started", "session.resumed"]) {
        let source = payload_string(payload, &["source", "reason"])
            .unwrap_or_default()
            .to_ascii_lowercase();
        return vec![
            if matches!(source.as_str(), "resume" | "resumed" | "continue") {
                "session.resumed"
            } else {
                "session.started"
            }
            .to_string(),
        ];
    }
    candidates.to_vec()
}

fn selected_with_supplemental(
    candidates: &[String],
    alternatives: &[&str],
    selected: &str,
) -> Vec<String> {
    let mut selected_emitted = false;
    let mut normalized = Vec::new();
    for candidate in candidates {
        if alternatives.contains(&candidate.as_str()) {
            if !selected_emitted {
                normalized.push(selected.to_string());
                selected_emitted = true;
            }
        } else {
            normalized.push(candidate.clone());
        }
    }
    normalized
}

fn contains_any(candidates: &[String], expected: &[&str]) -> bool {
    expected
        .iter()
        .all(|expected| candidates.iter().any(|candidate| candidate == expected))
}

#[derive(Clone, Copy)]
enum ParsedOutcome {
    Completed,
    Failed,
    Cancelled,
}

fn payload_outcome(payload: &serde_json::Value) -> ParsedOutcome {
    let value = payload_string(
        payload,
        &["outcome", "status", "reason", "stop_reason", "stopReason"],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();
    if value.contains("cancel") || value.contains("abort") || value.contains("interrupt") {
        ParsedOutcome::Cancelled
    } else if payload_failed(payload) {
        ParsedOutcome::Failed
    } else {
        ParsedOutcome::Completed
    }
}

fn payload_failed(payload: &serde_json::Value) -> bool {
    if payload
        .get("error")
        .is_some_and(|value| !value.is_null() && value != false)
    {
        return true;
    }
    if ["is_error", "isError", "failed"]
        .iter()
        .any(|key| payload_value(payload, key).and_then(serde_json::Value::as_bool) == Some(true))
    {
        return true;
    }
    if payload_value(payload, "success").and_then(serde_json::Value::as_bool) == Some(false) {
        return true;
    }
    payload_string(
        payload,
        &["outcome", "status", "reason", "stop_reason", "stopReason"],
    )
    .is_some_and(|value| {
        let value = value.to_ascii_lowercase();
        value.contains("fail") || value.contains("error") || value.contains("timeout")
    })
}

fn payload_string(payload: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        payload_value(payload, key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn payload_i64(payload: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        payload_value(payload, key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        })
    })
}

fn payload_u64(payload: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        payload_value(payload, key).and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        })
    })
}

fn payload_value<'a>(payload: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    payload.get(key).or_else(|| {
        [
            "properties",
            "input",
            "event",
            "data",
            "session",
            "output",
            "result",
            "tool_response",
            "toolResponse",
            "tool_result",
            "toolResult",
        ]
        .iter()
        .find_map(|container| payload.get(*container).and_then(|value| value.get(key)))
    })
}

fn normalize_provider_event_name(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "unknown".to_string()
    } else {
        normalized.chars().take(80).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentLifecycleEventKind;

    fn context() -> AgentHookParseContext {
        AgentHookParseContext {
            receipt_id: "receipt_test".to_string(),
            workspace_id: "workspace_test".to_string(),
            lane_id: Some("lane_test".to_string()),
            capture_run_id: None,
            provider_version: Some("test".to_string()),
            raw_digest: "sha256:test".to_string(),
            received_at: 10,
            transport: AgentCaptureTransport::NativeHooks,
        }
    }

    #[test]
    fn codex_prompt_yields_turn_and_message_with_native_ids() {
        let registry = AgentProviderRegistry::built_in().unwrap();
        let events = parse_agent_hook_payload(
            &registry,
            "codex",
            "UserPromptSubmit",
            &serde_json::json!({
                "session_id": "session-1",
                "turn_id": "turn-1",
                "prompt": "fix it",
                "timestamp": 4
            }),
            context(),
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].event_type.kind(),
            AgentLifecycleEventKind::TurnStarted
        );
        assert_eq!(
            events[1].event_type.kind(),
            AgentLifecycleEventKind::MessageUser
        );
        assert_eq!(events[0].native.session_id.as_deref(), Some("session-1"));
        assert_eq!(events[0].native.turn_id.as_deref(), Some("turn-1"));
    }

    #[test]
    fn terminal_and_tool_bindings_select_one_observed_outcome() {
        let registry = AgentProviderRegistry::built_in().unwrap();
        let turn = parse_agent_hook_payload(
            &registry,
            "gemini",
            "AfterAgent",
            &serde_json::json!({"sessionId": "s", "status": "cancelled"}),
            context(),
        )
        .unwrap();
        assert_eq!(turn.len(), 1);
        assert_eq!(
            turn[0].event_type.kind(),
            AgentLifecycleEventKind::TurnCancelled
        );

        let tool = parse_agent_hook_payload(
            &registry,
            "codex",
            "PostToolUse",
            &serde_json::json!({"session_id": "s", "error": "failed"}),
            context(),
        )
        .unwrap();
        assert_eq!(tool.len(), 1);
        assert_eq!(
            tool[0].event_type.kind(),
            AgentLifecycleEventKind::ToolFailed
        );
    }

    #[test]
    fn kiro_stop_preserves_the_assistant_message_alongside_the_turn_outcome() {
        let registry = AgentProviderRegistry::built_in().unwrap();
        let events = parse_agent_hook_payload(
            &registry,
            "kiro",
            "Stop",
            &serde_json::json!({
                "session_id": "kiro-session",
                "assistant_response": "Implemented and tested."
            }),
            context(),
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].event_type.kind(),
            AgentLifecycleEventKind::MessageAssistantCompleted
        );
        assert_eq!(
            events[1].event_type.kind(),
            AgentLifecycleEventKind::TurnCompleted
        );
    }

    #[test]
    fn unknown_native_events_are_retained_but_inert() {
        let registry = AgentProviderRegistry::built_in().unwrap();
        let events = parse_agent_hook_payload(
            &registry,
            "grok",
            "Future Event/V2",
            &serde_json::json!({"sessionId": "s"}),
            context(),
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].event_type.kind(),
            AgentLifecycleEventKind::Unknown
        );
        assert_eq!(
            events[0].event_type.as_str(),
            "provider.grok.future_event_v2"
        );
    }

    #[test]
    fn copilot_pascal_case_contract_resolves_case_insensitively() {
        let registry = AgentProviderRegistry::built_in().unwrap();
        let events = parse_agent_hook_payload(
            &registry,
            "copilot-cli",
            "SessionStart",
            &serde_json::json!({"session_id": "s", "source": "resume"}),
            context(),
        )
        .unwrap();
        assert_eq!(
            events[0].event_type.kind(),
            AgentLifecycleEventKind::SessionResumed
        );
    }

    #[test]
    fn checked_in_provider_contract_fixtures_normalize_all_nine_adapters() {
        let registry = AgentProviderRegistry::built_in().unwrap();
        let fixtures = [
            (
                "codex",
                include_str!("../../tests/fixtures/agent-hooks/codex/contracts.json"),
            ),
            (
                "claude-code",
                include_str!("../../tests/fixtures/agent-hooks/claude-code/contracts.json"),
            ),
            (
                "pi",
                include_str!("../../tests/fixtures/agent-hooks/pi/contracts.json"),
            ),
            (
                "opencode",
                include_str!("../../tests/fixtures/agent-hooks/opencode/contracts.json"),
            ),
            (
                "cursor",
                include_str!("../../tests/fixtures/agent-hooks/cursor/contracts.json"),
            ),
            (
                "gemini",
                include_str!("../../tests/fixtures/agent-hooks/gemini/contracts.json"),
            ),
            (
                "copilot",
                include_str!("../../tests/fixtures/agent-hooks/copilot/contracts.json"),
            ),
            (
                "grok",
                include_str!("../../tests/fixtures/agent-hooks/grok/contracts.json"),
            ),
            (
                "kiro",
                include_str!("../../tests/fixtures/agent-hooks/kiro/contracts.json"),
            ),
        ];
        for (provider, bytes) in fixtures {
            let cases: serde_json::Value = serde_json::from_str(bytes).unwrap();
            let cases = cases.as_array().unwrap();
            assert_eq!(cases.len(), 3, "{provider} fixture coverage");
            for case in cases {
                let event = case["event"].as_str().unwrap();
                let normalized = parse_agent_hook_payload(
                    &registry,
                    provider,
                    event,
                    &case["payload"],
                    context(),
                )
                .unwrap()
                .into_iter()
                .map(|event| event.event_type.as_str().to_string())
                .collect::<Vec<_>>();
                let expected = case["expected"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_string())
                    .collect::<Vec<_>>();
                assert_eq!(normalized, expected, "{provider} {event}");
            }
        }
    }
}
