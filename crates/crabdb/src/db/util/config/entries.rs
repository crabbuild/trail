use super::*;

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

pub(crate) fn config_entry_from(config: &CrabConfig, key: &str) -> Option<ConfigEntry> {
    config_entries_from(config)
        .into_iter()
        .find(|entry| entry.key == key)
}
