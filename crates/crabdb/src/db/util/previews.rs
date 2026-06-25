use super::*;

pub(crate) fn utf8_full(bytes: &[u8]) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}

pub(crate) fn utf8_preview(bytes: &[u8], max_chars: usize) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.chars().count() <= max_chars {
        return Some(text.to_string());
    }
    let mut preview = text.chars().take(max_chars).collect::<String>();
    preview.push_str("...");
    Some(preview)
}

pub(crate) fn hex_preview(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    if bytes.len() <= max_bytes {
        (hex::encode(bytes), false)
    } else {
        (hex::encode(&bytes[..max_bytes]), true)
    }
}

pub(crate) fn output_preview(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > AGENT_TEST_OUTPUT_PREVIEW_BYTES;
    let preview = if truncated {
        &bytes[..AGENT_TEST_OUTPUT_PREVIEW_BYTES]
    } else {
        bytes
    };
    (String::from_utf8_lossy(preview).into_owned(), truncated)
}
