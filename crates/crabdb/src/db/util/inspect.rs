use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum MapInspectType {
    Raw,
    Path,
    FileIndex,
    TextOrder,
    LineIndex,
}

impl MapInspectType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Path => "path",
            Self::FileIndex => "file-index",
            Self::TextOrder => "text-order",
            Self::LineIndex => "line-index",
        }
    }
}

pub(crate) fn parse_map_inspect_type(value: &str) -> Result<MapInspectType> {
    match value {
        "raw" => Ok(MapInspectType::Raw),
        "path" | "path-map" => Ok(MapInspectType::Path),
        "file-index" | "file_index" | "file-index-map" => Ok(MapInspectType::FileIndex),
        "text-order" | "text_order" | "order" | "order-map" => Ok(MapInspectType::TextOrder),
        "line-index" | "line_index" | "line-index-map" => Ok(MapInspectType::LineIndex),
        other => Err(Error::InvalidInput(format!(
            "map type must be raw, path, file-index, text-order, or line-index, got `{other}`"
        ))),
    }
}

pub(crate) fn parse_map_key_spec(spec: &str) -> Result<Vec<u8>> {
    if let Some(hex_value) = spec.strip_prefix("hex:") {
        return hex::decode(hex_value)
            .map_err(|err| Error::InvalidInput(format!("invalid hex map key: {err}")));
    }
    if let Some(text) = spec.strip_prefix("text:") {
        return Ok(text.as_bytes().to_vec());
    }
    if let Some(value) = spec.strip_prefix("u64:") {
        let value = value.parse::<u64>().map_err(|_| {
            Error::InvalidInput(format!("invalid unsigned integer map key `{value}`"))
        })?;
        return Ok(value.to_be_bytes().to_vec());
    }
    if let Some(line_number) = spec.strip_prefix("order:") {
        let line_number = line_number.parse::<u64>().map_err(|_| {
            Error::InvalidInput(format!("invalid order line number `{line_number}`"))
        })?;
        return Ok(order_key(line_number));
    }
    if let Some(id) = spec
        .strip_prefix("id:")
        .or_else(|| spec.strip_prefix("compound:"))
    {
        return parse_compound_map_key(id);
    }
    Ok(spec.as_bytes().to_vec())
}

pub(crate) fn parse_compound_map_key(spec: &str) -> Result<Vec<u8>> {
    let (change_id, local_seq) = spec.rsplit_once(':').ok_or_else(|| {
        Error::InvalidInput("compound map key must look like id:ch_...:<local_seq>".to_string())
    })?;
    if !change_id.starts_with("ch_") {
        return Err(Error::InvalidInput(
            "compound map key change id must start with ch_".to_string(),
        ));
    }
    let local_seq = local_seq.parse::<u64>().map_err(|_| {
        Error::InvalidInput(format!(
            "invalid compound map key local sequence `{local_seq}`"
        ))
    })?;
    Ok(FileId::new(ChangeId(change_id.to_string()), local_seq).encode_key())
}

pub(crate) fn inspect_map_diff_entry(map_type: MapInspectType, diff: Diff) -> MapDiffInspect {
    match diff {
        Diff::Added { key, val } => MapDiffInspect {
            kind: "added".to_string(),
            key: inspect_map_key(map_type, &key),
            old_value: None,
            new_value: Some(inspect_map_value(map_type, &val)),
        },
        Diff::Removed { key, val } => MapDiffInspect {
            kind: "removed".to_string(),
            key: inspect_map_key(map_type, &key),
            old_value: Some(inspect_map_value(map_type, &val)),
            new_value: None,
        },
        Diff::Changed { key, old, new } => MapDiffInspect {
            kind: "changed".to_string(),
            key: inspect_map_key(map_type, &key),
            old_value: Some(inspect_map_value(map_type, &old)),
            new_value: Some(inspect_map_value(map_type, &new)),
        },
    }
}

pub(crate) fn inspect_map_key(map_type: MapInspectType, key: &[u8]) -> MapKeyInspect {
    let text = utf8_full(key);
    let summary = match map_type {
        MapInspectType::Path => serde_json::json!({ "path": text.clone() }),
        MapInspectType::FileIndex => compound_key_summary(key, "file_id"),
        MapInspectType::TextOrder => order_key_summary(key),
        MapInspectType::LineIndex => compound_key_summary(key, "line_id"),
        MapInspectType::Raw => serde_json::json!({ "bytes": key.len() }),
    };
    MapKeyInspect {
        hex: hex::encode(key),
        text,
        summary,
    }
}

pub(crate) fn inspect_map_value(map_type: MapInspectType, value: &[u8]) -> MapValueInspect {
    let summary = match map_type {
        MapInspectType::Path => path_map_value_summary(value),
        MapInspectType::FileIndex => serde_json::json!({
            "path": utf8_full(value),
        }),
        MapInspectType::TextOrder => text_order_value_summary(value),
        MapInspectType::LineIndex => order_key_summary(value),
        MapInspectType::Raw => serde_json::json!({ "bytes": value.len() }),
    };
    let (hex_preview, truncated) = hex_preview(value, 256);
    MapValueInspect {
        bytes: value.len(),
        hex_preview,
        truncated,
        text: utf8_preview(value, 240),
        summary,
    }
}

pub(crate) fn path_map_value_summary(value: &[u8]) -> serde_json::Value {
    match decode_cbor_value::<FileEntry>(value) {
        Ok(entry) => serde_json::json!({
            "file_id": file_id_key(&entry.file_id),
            "kind": entry.kind,
            "mode": entry.mode,
            "executable": entry.executable,
            "size_bytes": entry.size_bytes,
            "content_hash": entry.content_hash,
            "content_object": content_object_id(&entry.content),
            "created_by": entry.created_by,
            "last_content_change": entry.last_content_change,
            "last_path_change": entry.last_path_change,
        }),
        Err(error) => decode_error_summary(error),
    }
}

pub(crate) fn text_order_value_summary(value: &[u8]) -> serde_json::Value {
    match decode_cbor_value::<LineEntry>(value) {
        Ok(entry) => serde_json::json!({
            "line_id": line_id_key_value(&entry.line_id),
            "text_hash": entry.text_hash,
            "text": utf8_preview(&entry.text, 240),
            "newline": entry.newline,
            "introduced_by": entry.introduced_by,
            "last_content_change": entry.last_content_change,
            "last_move_change": entry.last_move_change,
            "flags": entry.flags,
        }),
        Err(error) => decode_error_summary(error),
    }
}

pub(crate) fn decode_cbor_value<T>(value: &[u8]) -> std::result::Result<T, String>
where
    T: DeserializeOwned,
{
    from_cbor(value).map_err(|err| err.to_string())
}

pub(crate) fn decode_error_summary(error: String) -> serde_json::Value {
    serde_json::json!({ "decode_error": error })
}

pub(crate) fn compound_key_summary(key: &[u8], name: &str) -> serde_json::Value {
    if key.len() != 40 {
        return serde_json::json!({
            "bytes": key.len(),
            "expected": format!("{name} compound key"),
        });
    }
    let local_seq = u64::from_be_bytes(key[32..40].try_into().unwrap_or([0; 8]));
    serde_json::json!({
        "kind": name,
        "origin_change_digest": hex::encode(&key[..32]),
        "local_seq": local_seq,
    })
}

pub(crate) fn order_key_summary(key: &[u8]) -> serde_json::Value {
    if key.len() != 8 {
        return serde_json::json!({
            "bytes": key.len(),
            "expected": "8-byte big-endian order key",
        });
    }
    let order = u64::from_be_bytes(key.try_into().unwrap_or([0; 8]));
    let line_number_hint = if order % ORDER_KEY_STEP == 0 {
        Some(order / ORDER_KEY_STEP)
    } else {
        None
    };
    serde_json::json!({
        "order": order,
        "line_number_hint": line_number_hint,
    })
}

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

pub(crate) fn tree_root_hex(tree: &Tree) -> Option<String> {
    tree.root.as_ref().map(|cid| hex::encode(cid.as_bytes()))
}

pub(crate) fn tree_from_root_hex(root: Option<&str>) -> Result<Tree> {
    let cid = match root {
        Some(hex_root) => {
            let bytes = hex::decode(hex_root)
                .map_err(|err| Error::Corrupt(format!("invalid tree root hex: {err}")))?;
            let bytes: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| Error::Corrupt("tree root CID must be 32 bytes".to_string()))?;
            Some(Cid(bytes))
        }
        None => None,
    };
    Ok(Tree {
        root: cid,
        config: prolly_config(),
    })
}

pub(crate) fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub(crate) fn run_command_with_timeout(
    command: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Result<CommandRunResult> {
    let started = Instant::now();
    let mut child = match Command::new(&command[0])
        .args(&command[1..])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return Ok(CommandRunResult {
                success: false,
                exit_code: None,
                timed_out: false,
                duration_ms: elapsed_ms(started.elapsed()),
                stdout: Vec::new(),
                stderr: err.to_string().into_bytes(),
            });
        }
    };

    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            return Ok(CommandRunResult {
                success: output.status.success(),
                exit_code: output.status.code(),
                timed_out: false,
                duration_ms: elapsed_ms(started.elapsed()),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let output = child.wait_with_output()?;
            return Ok(CommandRunResult {
                success: false,
                exit_code: output.status.code(),
                timed_out: true,
                duration_ms: elapsed_ms(started.elapsed()),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        std::thread::sleep(Duration::from_millis(50));
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

pub(crate) fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

pub(crate) fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

pub(crate) fn known_gc_object_kinds() -> HashSet<&'static str> {
    [
        WORKTREE_ROOT_KIND,
        TEXT_CONTENT_KIND,
        OPERATION_KIND,
        BLOB_KIND,
        MESSAGE_KIND,
        ANCHOR_KIND,
    ]
    .into_iter()
    .collect()
}
