use super::*;

pub(crate) fn set_config_value(
    db: &Trail,
    config: &mut TrailConfig,
    key: &str,
    value: &str,
) -> Result<()> {
    match key {
        "workspace.id" => Err(Error::InvalidInput(
            "config key `workspace.id` is read-only".to_string(),
        )),
        "agent.default_provider" => {
            let provider = value.trim();
            if provider.is_empty()
                || !provider
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            {
                return Err(Error::InvalidInput(
                    "agent.default_provider must use lowercase ASCII letters, digits, and hyphens"
                        .to_string(),
                ));
            }
            config.agent.default_provider = Some(provider.to_string());
            Ok(())
        }
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
        "lane.default_materialize" => {
            config.lane.default_materialize = parse_config_bool(key, value)?;
            Ok(())
        }
        "lane.require_test_gate" => {
            config.lane.require_test_gate = parse_config_bool(key, value)?;
            Ok(())
        }
        "lane.require_eval_gate" => {
            config.lane.require_eval_gate = parse_config_bool(key, value)?;
            Ok(())
        }
        "lane.required_test_suites" => {
            config.lane.required_test_suites = parse_config_suite_list(key, value)?;
            Ok(())
        }
        "lane.required_eval_suites" => {
            config.lane.required_eval_suites = parse_config_suite_list(key, value)?;
            Ok(())
        }
        "lane.claim_enforcement" => match value {
            "off" | "warn" | "reject" => {
                config.lane.claim_enforcement = value.to_string();
                Ok(())
            }
            other => Err(Error::InvalidInput(format!(
                "lane.claim_enforcement must be off, warn, or reject, got `{other}`"
            ))),
        },
        "lane.enforce_sparse_paths" => {
            config.lane.enforce_sparse_paths = parse_config_bool(key, value)?;
            Ok(())
        }
        "lane.max_patch_bytes" => {
            config.lane.max_patch_bytes = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "lane.max_patch_file_bytes" => {
            config.lane.max_patch_file_bytes = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "lane.max_changed_paths" => {
            config.lane.max_changed_paths = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "lane.max_event_payload_bytes" => {
            config.lane.max_event_payload_bytes = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "lane.max_trace_payload_bytes" => {
            config.lane.max_trace_payload_bytes = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "lane.worktrees_dir" => {
            config.lane.worktrees_dir = normalize_relative_path(value)?;
            Ok(())
        }
        "lane.merge_strategy" => {
            if value != "conservative" {
                return Err(Error::InvalidInput(format!(
                    "lane.merge_strategy must be conservative, got `{value}`"
                )));
            }
            config.lane.merge_strategy = value.to_string();
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
        "workspace_views.upper_logical_bytes" => {
            config.workspace_views.upper_logical_bytes = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "workspace_views.upper_file_count" => {
            config.workspace_views.upper_file_count = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "workspace_views.single_file_bytes" => {
            config.workspace_views.single_file_bytes = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "workspace_views.journal_bytes" => {
            config.workspace_views.journal_bytes = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "workspace_views.cache_build_bytes" => {
            config.workspace_views.cache_build_bytes = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "workspace_views.concurrent_cache_builders" => {
            config.workspace_views.concurrent_cache_builders = parse_config_u64(key, value, false)?;
            Ok(())
        }
        "workspace_views.cache_retention_secs" => {
            config.workspace_views.cache_retention_secs = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "workspace_views.cache_max_bytes" => {
            config.workspace_views.cache_max_bytes = parse_config_u64(key, value, true)?;
            Ok(())
        }
        _ => Err(Error::InvalidInput(format!("unknown config key `{key}`"))),
    }
}
