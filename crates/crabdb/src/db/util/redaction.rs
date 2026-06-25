pub(crate) fn redact_sensitive_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    if is_sensitive_json_key(&key) {
                        (key, serde_json::Value::String("[REDACTED]".to_string()))
                    } else {
                        (key, redact_sensitive_json(value))
                    }
                })
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(redact_sensitive_json).collect())
        }
        serde_json::Value::String(value) => {
            serde_json::Value::String(redact_sensitive_text(&value))
        }
        other => other,
    }
}

pub(crate) fn redact_sensitive_text(input: &str) -> String {
    if !may_contain_sensitive_text(input) {
        return input.to_string();
    }
    let mut output = String::with_capacity(input.len());
    for chunk in input.split_inclusive('\n') {
        if let Some(line) = chunk.strip_suffix('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            output.push_str(&redact_sensitive_line(line));
            if chunk.ends_with("\r\n") {
                output.push_str("\r\n");
            } else {
                output.push('\n');
            }
        } else {
            output.push_str(&redact_sensitive_line(chunk));
        }
    }
    if input.is_empty() {
        output.clear();
    }
    output
}

pub(crate) fn may_contain_sensitive_text(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    [
        "authorization",
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "api-key",
        "apikey",
        "private_key",
        "private-key",
        "bearer ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(crate) fn redact_sensitive_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if let Some((separator_idx, value_start)) = sensitive_assignment_span(line, &lower) {
        let mut redacted = String::new();
        redacted.push_str(&line[..separator_idx + 1]);
        redacted.push_str(&line[separator_idx + 1..value_start]);
        redacted.push_str("[REDACTED]");
        return redacted;
    }
    if let Some(idx) = lower.find("bearer ") {
        let value_start = idx + "bearer ".len();
        let mut redacted = String::new();
        redacted.push_str(&line[..value_start]);
        redacted.push_str("[REDACTED]");
        return redacted;
    }
    line.to_string()
}

pub(crate) fn sensitive_assignment_span(line: &str, lower: &str) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for key in SENSITIVE_TEXT_KEYS {
        let mut search_from = 0;
        while let Some(relative_idx) = lower[search_from..].find(key) {
            let key_idx = search_from + relative_idx;
            let rest_start = key_idx + key.len();
            let rest = &lower[rest_start..];
            let Some(separator_relative_idx) = rest.find(|ch| ch == ':' || ch == '=') else {
                search_from = rest_start;
                continue;
            };
            let between = &line[rest_start..rest_start + separator_relative_idx];
            if between.chars().all(is_secret_separator_padding) {
                let separator_idx = rest_start + separator_relative_idx;
                let value_start = line[separator_idx + 1..]
                    .char_indices()
                    .find(|(_, ch)| !ch.is_whitespace())
                    .map(|(idx, _)| separator_idx + 1 + idx)
                    .unwrap_or(line.len());
                let candidate = (separator_idx, value_start);
                if best
                    .map(|(best_idx, _)| separator_idx < best_idx)
                    .unwrap_or(true)
                {
                    best = Some(candidate);
                }
                break;
            }
            search_from = rest_start;
        }
    }
    best
}

pub(crate) fn is_secret_separator_padding(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '"' | '\'' | '`' | '_' | '-')
}

pub(crate) fn is_sensitive_json_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .map(|ch| match ch {
            '-' | ' ' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect::<String>();
    normalized == "authorization"
        || normalized == "password"
        || normalized == "passwd"
        || normalized == "secret"
        || normalized == "token"
        || normalized == "credential"
        || normalized.ends_with("password")
        || normalized.ends_with("secret")
        || normalized.ends_with("token")
        || normalized.ends_with("credential")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_token")
        || normalized.ends_with("_credential")
        || normalized.contains("api_key")
        || normalized.contains("apikey")
        || normalized.contains("private_key")
}

pub(crate) const SENSITIVE_TEXT_KEYS: &[&str] = &[
    "authorization",
    "openai_api_key",
    "anthropic_api_key",
    "client_secret",
    "client-secret",
    "private_key",
    "private-key",
    "refresh_token",
    "refresh-token",
    "access_token",
    "access-token",
    "auth_token",
    "auth-token",
    "id_token",
    "id-token",
    "api_key",
    "api-key",
    "apikey",
    "password",
    "passwd",
    "secret",
    "token",
];
