use super::*;

pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

pub(crate) fn classify_file_kind(bytes: &[u8], text_config: &TextConfig) -> FileKind {
    let below_tree_text_threshold = (bytes.len() as u64) < text_config.tree_text_min_bytes;
    let should_store_small_text = text_config.small_text_max_bytes > 0
        && bytes.len() as u64 <= text_config.small_text_max_bytes;
    if looks_binary(bytes) {
        FileKind::Binary
    } else if std::str::from_utf8(bytes).is_err()
        || bytes.len() as u64 > text_config.opaque_text_max_bytes
        || max_line_len(bytes) as u64 > text_config.max_line_bytes
        || (below_tree_text_threshold && !should_store_small_text)
    {
        FileKind::OpaqueText
    } else {
        FileKind::Text
    }
}

pub(crate) fn max_line_len(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .map(|line| line.len())
        .max()
        .unwrap_or(0)
}
