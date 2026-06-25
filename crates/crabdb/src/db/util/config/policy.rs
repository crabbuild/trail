use super::*;

pub(crate) fn apply_text_policy(config: &mut TextConfig, policy: Option<&str>) -> Result<()> {
    let Some(policy) = policy else {
        return Ok(());
    };
    match policy {
        "balanced" => Ok(()),
        "minimal" => {
            config.small_text_max_bytes = 4 * 1024;
            config.tree_text_min_bytes = 64 * 1024 * 1024 + 1;
            config.opaque_text_max_bytes = 64 * 1024 * 1024;
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
