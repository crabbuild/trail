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

pub(crate) fn contains_sensitive_json(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            (is_sensitive_json_key(key) && json_value_has_payload(value))
                || contains_sensitive_json(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_sensitive_json),
        serde_json::Value::String(value) => contains_sensitive_text(value),
        _ => false,
    }
}

pub(crate) fn redact_sensitive_text(input: &str) -> String {
    if !may_contain_sensitive_text(input) && !contains_private_key_pem(input) {
        return input.to_string();
    }
    let without_private_keys = redact_private_key_pem_blocks(input);
    let mut output = String::with_capacity(without_private_keys.len());
    for chunk in without_private_keys.split_inclusive('\n') {
        let (line, ending) = split_line_ending(chunk);
        output.push_str(&redact_sensitive_line(line));
        output.push_str(ending);
    }
    if without_private_keys.is_empty() {
        output.clear();
    }
    output
}

fn redact_private_key_pem_blocks(input: &str) -> String {
    if !contains_private_key_pem(input) {
        return input.to_string();
    }
    let mut output = String::with_capacity(input.len());
    let mut in_private_key = false;
    for chunk in input.split_inclusive('\n') {
        let (line, ending) = split_line_ending(chunk);
        let upper = line.to_ascii_uppercase();
        let starts_private_key =
            upper.contains("-----BEGIN ") && upper.contains("PRIVATE KEY-----");
        let ends_private_key = upper.contains("-----END ") && upper.contains("PRIVATE KEY-----");
        if starts_private_key {
            output.push_str("[REDACTED]");
            output.push_str(ending);
            in_private_key = !ends_private_key;
        } else if in_private_key {
            if ends_private_key {
                in_private_key = false;
            }
        } else {
            output.push_str(line);
            output.push_str(ending);
        }
    }
    output
}

pub(crate) fn contains_sensitive_text(input: &str) -> bool {
    if contains_private_key_pem(input) {
        return true;
    }
    redact_sensitive_text(input) != input
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
        "private key",
        "bearer ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn contains_private_key_pem(input: &str) -> bool {
    let upper = input.to_ascii_uppercase();
    upper.contains("-----BEGIN ") && upper.contains("PRIVATE KEY-----")
}

fn split_line_ending(chunk: &str) -> (&str, &str) {
    if let Some(line) = chunk.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = chunk.strip_suffix('\n') {
        (line, "\n")
    } else {
        (chunk, "")
    }
}

fn json_value_has_payload(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(values) => !values.is_empty(),
        serde_json::Value::Object(map) => !map.is_empty(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_text_detector_avoids_benign_keyword_mentions() {
        assert!(!contains_sensitive_text(
            "token expiration logic stays visible"
        ));
        assert!(contains_sensitive_text("OPENAI_API_KEY=sk-live-secret"));
        assert!(contains_sensitive_text("Authorization: Bearer abc123"));
        assert!(contains_sensitive_text(
            "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----"
        ));
    }

    #[test]
    fn sensitive_text_redaction_removes_private_key_pem_blocks() {
        let redacted = redact_sensitive_text(
            "before\n-----BEGIN PRIVATE KEY-----\nkey-material\n-----END PRIVATE KEY-----\nafter\n",
        );
        assert!(redacted.contains("before"));
        assert!(redacted.contains("after"));
        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("key-material"));
        assert!(!redacted.contains("PRIVATE KEY"));
    }

    #[test]
    fn sensitive_json_detector_flags_secret_keys_with_payloads() {
        assert!(contains_sensitive_json(&serde_json::json!({
            "safe": "token expiration logic",
            "client_secret": "abc"
        })));
        assert!(!contains_sensitive_json(&serde_json::json!({
            "safe": "token expiration logic",
            "client_secret": ""
        })));
    }
}
