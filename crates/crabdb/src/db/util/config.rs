use super::*;

pub(crate) fn apply_sqlite_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

pub(crate) fn ensure_column(
    conn: &Connection,
    table: &'static str,
    column: &'static str,
    definition: &'static str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !columns.iter().any(|existing| existing == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

pub(crate) fn config_entries_from(config: &CrabConfig) -> Vec<ConfigEntry> {
    vec![
        config_entry("workspace.id", &config.workspace.id.0, "string", true),
        config_entry(
            "workspace.default_branch",
            &config.workspace.default_branch,
            "string",
            false,
        ),
        config_entry("recording.mode", &config.recording.mode, "string", false),
        config_entry(
            "recording.debounce_ms",
            config.recording.debounce_ms,
            "u64",
            false,
        ),
        config_entry(
            "recording.ignore_gitignored",
            config.recording.ignore_gitignored,
            "bool",
            false,
        ),
        config_entry(
            "text.small_text_max_bytes",
            config.text.small_text_max_bytes,
            "u64",
            false,
        ),
        config_entry(
            "text.tree_text_min_bytes",
            config.text.tree_text_min_bytes,
            "u64",
            false,
        ),
        config_entry(
            "text.opaque_text_max_bytes",
            config.text.opaque_text_max_bytes,
            "u64",
            false,
        ),
        config_entry(
            "text.max_line_bytes",
            config.text.max_line_bytes,
            "u64",
            false,
        ),
        config_entry(
            "text.preserve_similarity",
            config.text.preserve_similarity,
            "f32",
            false,
        ),
        config_entry(
            "agent.default_materialize",
            config.agent.default_materialize,
            "bool",
            false,
        ),
        config_entry(
            "agent.require_test_gate",
            config.agent.require_test_gate,
            "bool",
            false,
        ),
        config_entry(
            "agent.require_eval_gate",
            config.agent.require_eval_gate,
            "bool",
            false,
        ),
        config_entry(
            "agent.required_test_suites",
            format_config_list(&config.agent.required_test_suites),
            "list",
            false,
        ),
        config_entry(
            "agent.required_eval_suites",
            format_config_list(&config.agent.required_eval_suites),
            "list",
            false,
        ),
        config_entry(
            "agent.worktrees_dir",
            &config.agent.worktrees_dir,
            "path",
            false,
        ),
        config_entry(
            "agent.merge_strategy",
            &config.agent.merge_strategy,
            "string",
            false,
        ),
        config_entry(
            "git.export_trailers",
            config.git.export_trailers,
            "bool",
            false,
        ),
        config_entry(
            "guardrails.policy",
            &config.guardrails.policy,
            "policy",
            false,
        ),
    ]
}

pub(crate) fn config_entry(
    key: impl Into<String>,
    value: impl ToString,
    value_type: impl Into<String>,
    read_only: bool,
) -> ConfigEntry {
    ConfigEntry {
        key: key.into(),
        value: value.to_string(),
        value_type: value_type.into(),
        read_only,
    }
}

pub(crate) fn format_config_list(values: &[String]) -> String {
    values.join(",")
}

pub(crate) fn normalize_agent_gate_options(
    kind: &str,
    mut options: AgentGateOptions,
) -> Result<AgentGateOptions> {
    if let Some(suite) = options.suite.take() {
        let suite = suite.trim();
        if suite.is_empty() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} suite cannot be empty"
            )));
        }
        options.suite = Some(suite.to_string());
    }
    if let Some(score) = options.score {
        if !score.is_finite() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} score must be a finite number"
            )));
        }
    }
    if let Some(threshold) = options.threshold {
        if !threshold.is_finite() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} threshold must be a finite number"
            )));
        }
        if options.score.is_none() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} threshold requires a score"
            )));
        }
    }
    Ok(options)
}

pub(crate) fn normalize_agent_gate_filter(kind: Option<&str>) -> Result<Option<&'static str>> {
    let Some(kind) = kind.map(str::trim).filter(|kind| !kind.is_empty()) else {
        return Ok(None);
    };
    let normalized = kind.to_ascii_lowercase();
    match normalized.as_str() {
        "all" => Ok(None),
        "test" | "tests" => Ok(Some("test")),
        "eval" | "evals" => Ok(Some("eval")),
        other => Err(Error::InvalidInput(format!(
            "agent gate kind must be test, eval, or all, got `{other}`"
        ))),
    }
}

pub(crate) fn agent_gate_event_type(kind: &str) -> Result<&'static str> {
    match kind {
        "test" => Ok("test_finished"),
        "eval" => Ok("eval_finished"),
        other => Err(Error::InvalidInput(format!(
            "agent gate kind must be test or eval, got `{other}`"
        ))),
    }
}

pub(crate) fn agent_gate_kind_from_event_type(event_type: &str) -> Result<&'static str> {
    match event_type {
        "test_finished" => Ok("test"),
        "eval_finished" => Ok("eval"),
        other => Err(Error::Corrupt(format!(
            "unknown agent gate event type `{other}`"
        ))),
    }
}

pub(crate) fn parse_agent_gate_summary(
    event_id: &str,
    turn_id: Option<String>,
    kind: &str,
    payload_json: &str,
    created_at: i64,
) -> Result<AgentTestSummary> {
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
    Ok(AgentTestSummary {
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
        created_at,
    })
}

pub(crate) fn apply_text_policy(config: &mut TextConfig, policy: Option<&str>) -> Result<()> {
    let Some(policy) = policy else {
        return Ok(());
    };
    match policy {
        "balanced" => Ok(()),
        "minimal" => {
            config.small_text_max_bytes = 4 * 1024;
            config.tree_text_min_bytes = 64 * 1024 + 1;
            config.opaque_text_max_bytes = 64 * 1024;
            config.max_line_bytes = 256 * 1024;
            config.preserve_similarity = 0.35;
            Ok(())
        }
        "full" => {
            config.small_text_max_bytes = 0;
            config.tree_text_min_bytes = 1;
            config.opaque_text_max_bytes = 64 * 1024 * 1024;
            config.max_line_bytes = 8 * 1024 * 1024;
            config.preserve_similarity = 0.65;
            Ok(())
        }
        other => Err(Error::InvalidInput(format!(
            "text policy must be minimal, balanced, or full, got `{other}`"
        ))),
    }
}

pub(crate) fn config_entry_from(config: &CrabConfig, key: &str) -> Option<ConfigEntry> {
    config_entries_from(config)
        .into_iter()
        .find(|entry| entry.key == key)
}

pub(crate) fn set_config_value(
    db: &CrabDb,
    config: &mut CrabConfig,
    key: &str,
    value: &str,
) -> Result<()> {
    match key {
        "workspace.id" => Err(Error::InvalidInput(
            "config key `workspace.id` is read-only".to_string(),
        )),
        "workspace.default_branch" => {
            validate_ref_segment(value)?;
            if db.try_get_ref(&branch_ref(value))?.is_none() {
                return Err(Error::InvalidInput(format!(
                    "default branch `{value}` does not exist"
                )));
            }
            config.workspace.default_branch = value.to_string();
            Ok(())
        }
        "recording.mode" => match value {
            "save" | "manual" | "watch" => {
                config.recording.mode = value.to_string();
                Ok(())
            }
            other => Err(Error::InvalidInput(format!(
                "recording.mode must be save, manual, or watch, got `{other}`"
            ))),
        },
        "recording.debounce_ms" => {
            config.recording.debounce_ms = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "recording.ignore_gitignored" => {
            config.recording.ignore_gitignored = parse_config_bool(key, value)?;
            Ok(())
        }
        "text.small_text_max_bytes" => {
            config.text.small_text_max_bytes = parse_config_u64(key, value, false)?;
            Ok(())
        }
        "text.tree_text_min_bytes" => {
            config.text.tree_text_min_bytes = parse_config_u64(key, value, false)?;
            Ok(())
        }
        "text.opaque_text_max_bytes" => {
            config.text.opaque_text_max_bytes = parse_config_u64(key, value, false)?;
            Ok(())
        }
        "text.max_line_bytes" => {
            config.text.max_line_bytes = parse_config_u64(key, value, false)?;
            Ok(())
        }
        "text.preserve_similarity" => {
            let parsed = value.parse::<f32>().map_err(|_| {
                Error::InvalidInput(format!("config key `{key}` expects a floating point value"))
            })?;
            if !parsed.is_finite() || !(0.0..=1.0).contains(&parsed) {
                return Err(Error::InvalidInput(format!(
                    "config key `{key}` must be between 0.0 and 1.0"
                )));
            }
            config.text.preserve_similarity = parsed;
            Ok(())
        }
        "agent.default_materialize" => {
            config.agent.default_materialize = parse_config_bool(key, value)?;
            Ok(())
        }
        "agent.require_test_gate" => {
            config.agent.require_test_gate = parse_config_bool(key, value)?;
            Ok(())
        }
        "agent.require_eval_gate" => {
            config.agent.require_eval_gate = parse_config_bool(key, value)?;
            Ok(())
        }
        "agent.required_test_suites" => {
            config.agent.required_test_suites = parse_config_suite_list(key, value)?;
            Ok(())
        }
        "agent.required_eval_suites" => {
            config.agent.required_eval_suites = parse_config_suite_list(key, value)?;
            Ok(())
        }
        "agent.worktrees_dir" => {
            config.agent.worktrees_dir = normalize_relative_path(value)?;
            Ok(())
        }
        "agent.merge_strategy" => {
            if value != "conservative" {
                return Err(Error::InvalidInput(format!(
                    "agent.merge_strategy must be conservative, got `{value}`"
                )));
            }
            config.agent.merge_strategy = value.to_string();
            Ok(())
        }
        "git.export_trailers" => {
            config.git.export_trailers = parse_config_bool(key, value)?;
            Ok(())
        }
        "guardrails.policy" => {
            let _ = parse_guardrail_policy(value)?;
            config.guardrails.policy = value.to_string();
            Ok(())
        }
        _ => Err(Error::InvalidInput(format!("unknown config key `{key}`"))),
    }
}

pub(crate) fn parse_config_bool(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(Error::InvalidInput(format!(
            "config key `{key}` expects a boolean value"
        ))),
    }
}

pub(crate) fn parse_config_suite_list(key: &str, value: &str) -> Result<Vec<String>> {
    let mut suites = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.split([',', ';', '\n']) {
        let suite = raw.trim();
        if suite.is_empty() {
            continue;
        }
        if suite
            .chars()
            .any(|ch| matches!(ch, ',' | ';' | '\n' | '\r'))
        {
            return Err(Error::InvalidInput(format!(
                "config key `{key}` suite names cannot contain separators"
            )));
        }
        if seen.insert(suite.to_string()) {
            suites.push(suite.to_string());
        }
    }
    Ok(suites)
}

pub(crate) fn parse_config_u64(key: &str, value: &str, allow_zero: bool) -> Result<u64> {
    let parsed = value.parse::<u64>().map_err(|_| {
        Error::InvalidInput(format!("config key `{key}` expects an unsigned integer"))
    })?;
    if !allow_zero && parsed == 0 {
        return Err(Error::InvalidInput(format!(
            "config key `{key}` must be greater than zero"
        )));
    }
    Ok(parsed)
}

pub(crate) fn read_config(db_dir: &Path) -> Result<CrabConfig> {
    let text = fs::read_to_string(db_dir.join(CONFIG_FILE))?;
    Ok(toml::from_str(&text)?)
}

pub(crate) fn write_config(db_dir: &Path, config: &CrabConfig) -> Result<()> {
    let path = db_dir.join(CONFIG_FILE);
    let temp = db_dir.join(format!("{CONFIG_FILE}.tmp.{}", now_nanos()));
    fs::write(&temp, toml::to_string_pretty(config)?)?;
    if let Err(err) = fs::rename(&temp, &path) {
        let _ = fs::remove_file(&temp);
        return Err(Error::Io(err));
    }
    Ok(())
}

pub(crate) fn write_default_crabignore(workspace_root: &Path) -> Result<()> {
    let path = workspace_root.join(".crabignore");
    if path.exists() {
        return Ok(());
    }
    fs::write(
        path,
        format!("{}\n", DEFAULT_CRABIGNORE_PATTERNS.join("\n")),
    )?;
    Ok(())
}

pub(crate) fn read_ignore_patterns(path: &Path) -> Result<Vec<IgnorePattern>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(Error::Io(err)),
    };
    Ok(content
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let pattern = line.trim();
            if pattern.is_empty() || pattern.starts_with('#') {
                None
            } else {
                Some(IgnorePattern {
                    line: idx + 1,
                    pattern: pattern.to_string(),
                })
            }
        })
        .collect())
}

pub(crate) fn normalize_ignore_pattern(pattern: &str) -> Result<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(Error::InvalidInput(
            "ignore pattern cannot be empty".to_string(),
        ));
    }
    if pattern.starts_with('#') {
        return Err(Error::InvalidInput(
            "ignore pattern cannot be a comment".to_string(),
        ));
    }
    if pattern.contains('\0') || pattern.contains('\n') || pattern.contains('\r') {
        return Err(Error::InvalidInput(
            "ignore pattern cannot contain control separators".to_string(),
        ));
    }
    Ok(pattern.to_string())
}

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

pub(crate) fn write_ref_file(
    db_dir: &Path,
    name: &str,
    change_id: &ChangeId,
    root_id: &ObjectId,
    operation_id: &ObjectId,
    generation: i64,
) -> Result<()> {
    let path = db_dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({
        "name": name,
        "change_id": change_id.0,
        "root_id": root_id.0,
        "operation_id": operation_id.0,
        "generation": generation,
        "updated_at": now_ts(),
    });
    fs::write(path, serde_json::to_vec_pretty(&body)?)?;
    Ok(())
}

pub(crate) fn remove_ref_file(db_dir: &Path, name: &str) -> Result<()> {
    let path = db_dir.join(name);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(Error::Io(err)),
    }
}

pub(crate) fn prolly_config() -> Config {
    Config::builder()
        .min_chunk_size(4)
        .max_chunk_size(1024)
        .chunking_factor(128)
        .hash_seed(0xC0DB)
        .encoding(Encoding::Raw)
        .build()
}
