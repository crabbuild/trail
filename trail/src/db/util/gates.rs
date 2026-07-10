use super::*;

pub(crate) fn normalize_lane_gate_options(
    kind: &str,
    mut options: LaneGateOptions,
) -> Result<LaneGateOptions> {
    if let Some(suite) = options.suite.take() {
        let suite = suite.trim();
        if suite.is_empty() {
            return Err(Error::InvalidInput(format!(
                "lane {kind} suite cannot be empty"
            )));
        }
        options.suite = Some(suite.to_string());
    }
    if let Some(score) = options.score {
        if !score.is_finite() {
            return Err(Error::InvalidInput(format!(
                "lane {kind} score must be a finite number"
            )));
        }
    }
    if let Some(threshold) = options.threshold {
        if !threshold.is_finite() {
            return Err(Error::InvalidInput(format!(
                "lane {kind} threshold must be a finite number"
            )));
        }
        if options.score.is_none() {
            return Err(Error::InvalidInput(format!(
                "lane {kind} threshold requires a score"
            )));
        }
    }
    Ok(options)
}

pub(crate) fn normalize_lane_gate_filter(kind: Option<&str>) -> Result<Option<&'static str>> {
    let Some(kind) = kind.map(str::trim).filter(|kind| !kind.is_empty()) else {
        return Ok(None);
    };
    let normalized = kind.to_ascii_lowercase();
    match normalized.as_str() {
        "all" => Ok(None),
        "test" | "tests" => Ok(Some("test")),
        "eval" | "evals" => Ok(Some("eval")),
        other => Err(Error::InvalidInput(format!(
            "lane gate kind must be test, eval, or all, got `{other}`"
        ))),
    }
}

pub(crate) fn lane_gate_event_type(kind: &str) -> Result<&'static str> {
    match kind {
        "test" => Ok("test_finished"),
        "eval" => Ok("eval_finished"),
        other => Err(Error::InvalidInput(format!(
            "lane gate kind must be test or eval, got `{other}`"
        ))),
    }
}

pub(crate) fn lane_gate_kind_from_event_type(event_type: &str) -> Result<&'static str> {
    match event_type {
        "test_finished" => Ok("test"),
        "eval_finished" => Ok("eval"),
        other => Err(Error::Corrupt(format!(
            "unknown lane gate event type `{other}`"
        ))),
    }
}

pub(crate) fn parse_lane_gate_summary(
    event_id: &str,
    turn_id: Option<String>,
    kind: &str,
    payload_json: &str,
    created_at: i64,
) -> Result<LaneTestSummary> {
    let payload =
        serde_json::from_str::<serde_json::Value>(payload_json).unwrap_or(serde_json::Value::Null);
    let command = payload
        .get("command")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(LaneTestSummary {
        event_id: event_id.to_string(),
        turn_id,
        kind: kind.to_string(),
        suite: payload
            .get("suite")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        score: payload.get("score").and_then(|value| value.as_f64()),
        threshold: payload.get("threshold").and_then(|value| value.as_f64()),
        status: payload
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string(),
        success: payload
            .get("success")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        exit_code: payload
            .get("exit_code")
            .and_then(|value| value.as_i64())
            .map(|value| value as i32),
        timed_out: payload
            .get("timed_out")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        duration_ms: payload
            .get("duration_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(0),
        command,
        source_root: payload
            .get("source_root")
            .and_then(|value| value.as_str())
            .map(|value| ObjectId(value.to_string())),
        view_id: payload
            .get("view_id")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        view_generation: payload
            .get("view_generation")
            .and_then(|value| value.as_u64()),
        environment_keys: json_string_array(&payload, "environment_keys"),
        layer_ids: json_string_array(&payload, "layer_ids"),
        created_at,
    })
}

fn json_string_array(payload: &serde_json::Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}
