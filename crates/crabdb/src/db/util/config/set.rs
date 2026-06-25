use super::*;

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
