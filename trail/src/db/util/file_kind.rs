use super::*;

pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

pub(crate) fn classify_file_kind(bytes: &[u8], text_config: &TextConfig) -> FileKind {
    if looks_binary(bytes) {
        FileKind::Binary
    } else if std::str::from_utf8(bytes).is_err()
        || bytes.len() as u64 > text_config.opaque_text_max_bytes
        || max_line_len(bytes) as u64 > text_config.max_line_bytes
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

#[cfg(test)]
mod tests {
    use super::*;

    fn text_config_for_kind_test() -> TextConfig {
        TextConfig {
            small_text_max_bytes: 4,
            tree_text_min_bytes: 1024,
            opaque_text_max_bytes: 4096,
            max_line_bytes: 4096,
            preserve_similarity: 0.0,
        }
    }

    #[test]
    fn lazy_text_sized_utf8_is_still_text_kind() {
        let bytes = b"line 1\nline 2\nline 3\n";

        assert_eq!(
            classify_file_kind(bytes, &text_config_for_kind_test()),
            FileKind::Text
        );
    }

    #[test]
    fn oversized_utf8_is_opaque_text_kind() {
        let bytes = vec![b'a'; 4097];

        assert_eq!(
            classify_file_kind(&bytes, &text_config_for_kind_test()),
            FileKind::OpaqueText
        );
    }
}
