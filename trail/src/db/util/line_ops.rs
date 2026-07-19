use super::*;

const SMALL_TEXT_TABLE_VERSION: u8 = 1;
const SMALL_TEXT_NEWLINE_MASK: u8 = 0b0000_0011;
const SMALL_TEXT_HAS_ORIGIN_CHANGE: u8 = 0b0000_0100;
const SMALL_TEXT_HAS_INTRODUCED_BY: u8 = 0b0000_1000;
const SMALL_TEXT_HAS_LAST_CONTENT_CHANGE: u8 = 0b0001_0000;
const SMALL_TEXT_HAS_LAST_MOVE_CHANGE: u8 = 0b0010_0000;
const SMALL_TEXT_GENERATED: u8 = 0b0100_0000;
const SMALL_TEXT_REDACTED: u8 = 0b1000_0000;

#[derive(Clone)]
pub(crate) struct SplitLine {
    pub(crate) text: Vec<u8>,
    pub(crate) newline: NewlineKind,
}

pub(crate) fn split_lines(bytes: &[u8]) -> Vec<SplitLine> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if idx > start && bytes[idx - 1] == b'\r' {
                out.push(SplitLine {
                    text: bytes[start..idx - 1].to_vec(),
                    newline: NewlineKind::Crlf,
                });
            } else {
                out.push(SplitLine {
                    text: bytes[start..idx].to_vec(),
                    newline: NewlineKind::Lf,
                });
            }
            start = idx + 1;
        }
    }
    if start < bytes.len() {
        out.push(SplitLine {
            text: bytes[start..].to_vec(),
            newline: NewlineKind::None,
        });
    }
    out
}

pub(crate) fn materialize_lines(lines: &[LineEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(&line.text);
        match line.newline {
            NewlineKind::None => {}
            NewlineKind::Lf => out.push(b'\n'),
            NewlineKind::Crlf => out.extend_from_slice(b"\r\n"),
        }
    }
    out
}

pub(crate) fn encode_small_text_table(lines: &[LineEntry]) -> Vec<u8> {
    let default_origin_change = most_common_origin_change(lines)
        .unwrap_or_else(|| ChangeId("change_empty_small_text_table".to_string()));
    let mut out = Vec::new();
    out.push(SMALL_TEXT_TABLE_VERSION);
    write_change_id(&mut out, &default_origin_change);
    write_varint(&mut out, lines.len() as u64);

    for line in lines {
        write_varint(&mut out, line.line_id.local_seq);
        let mut flags = newline_code(line.newline);
        if line.line_id.origin_change != default_origin_change {
            flags |= SMALL_TEXT_HAS_ORIGIN_CHANGE;
        }
        if line.introduced_by != line.line_id.origin_change {
            flags |= SMALL_TEXT_HAS_INTRODUCED_BY;
        }
        if line.last_content_change != line.line_id.origin_change {
            flags |= SMALL_TEXT_HAS_LAST_CONTENT_CHANGE;
        }
        if line.last_move_change.is_some() {
            flags |= SMALL_TEXT_HAS_LAST_MOVE_CHANGE;
        }
        if line.flags.generated {
            flags |= SMALL_TEXT_GENERATED;
        }
        if line.flags.redacted {
            flags |= SMALL_TEXT_REDACTED;
        }
        out.push(flags);
        if flags & SMALL_TEXT_HAS_ORIGIN_CHANGE != 0 {
            write_change_id(&mut out, &line.line_id.origin_change);
        }
        if flags & SMALL_TEXT_HAS_INTRODUCED_BY != 0 {
            write_change_id(&mut out, &line.introduced_by);
        }
        if flags & SMALL_TEXT_HAS_LAST_CONTENT_CHANGE != 0 {
            write_change_id(&mut out, &line.last_content_change);
        }
        if let Some(last_move_change) = &line.last_move_change {
            write_change_id(&mut out, last_move_change);
        }
        write_varint(&mut out, line.text.len() as u64);
        out.extend_from_slice(&line.text);
    }
    out
}

pub(crate) fn decode_small_text_table(table: &[u8]) -> Result<Vec<LineEntry>> {
    let mut cursor = 0;
    let version = read_u8(table, &mut cursor)?;
    if version != SMALL_TEXT_TABLE_VERSION {
        return Err(Error::Corrupt(format!(
            "unsupported small text table version {version}"
        )));
    }
    let default_origin_change = read_change_id(table, &mut cursor)?;
    let line_count = read_varint(table, &mut cursor)?;
    let mut lines = Vec::with_capacity(line_count as usize);

    for _ in 0..line_count {
        let local_seq = read_varint(table, &mut cursor)?;
        let flags = read_u8(table, &mut cursor)?;
        let newline = decode_newline(flags & SMALL_TEXT_NEWLINE_MASK)?;
        let origin_change = if flags & SMALL_TEXT_HAS_ORIGIN_CHANGE != 0 {
            read_change_id(table, &mut cursor)?
        } else {
            default_origin_change.clone()
        };
        let introduced_by = if flags & SMALL_TEXT_HAS_INTRODUCED_BY != 0 {
            read_change_id(table, &mut cursor)?
        } else {
            origin_change.clone()
        };
        let last_content_change = if flags & SMALL_TEXT_HAS_LAST_CONTENT_CHANGE != 0 {
            read_change_id(table, &mut cursor)?
        } else {
            origin_change.clone()
        };
        let last_move_change = if flags & SMALL_TEXT_HAS_LAST_MOVE_CHANGE != 0 {
            Some(read_change_id(table, &mut cursor)?)
        } else {
            None
        };
        let text_len = read_varint(table, &mut cursor)? as usize;
        if cursor + text_len > table.len() {
            return Err(Error::Corrupt(
                "small text table line exceeds table length".to_string(),
            ));
        }
        let text = table[cursor..cursor + text_len].to_vec();
        cursor += text_len;
        let text_hash = sha256_hex(&text);
        lines.push(LineEntry {
            line_id: LineId::new(origin_change, local_seq),
            text,
            newline,
            text_hash,
            introduced_by,
            last_content_change,
            last_move_change,
            flags: LineFlags {
                generated: flags & SMALL_TEXT_GENERATED != 0,
                redacted: flags & SMALL_TEXT_REDACTED != 0,
            },
        });
    }

    if cursor != table.len() {
        return Err(Error::Corrupt(
            "small text table has trailing bytes".to_string(),
        ));
    }
    Ok(lines)
}

pub(crate) fn line_map_by_id(lines: &[LineEntry]) -> HashMap<String, &LineEntry> {
    lines
        .iter()
        .map(|line| (line.line_id_key(), line))
        .collect()
}

pub(crate) fn line_content_equal(left: Option<&LineEntry>, right: Option<&LineEntry>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.text_hash == right.text_hash
                && left.newline == right.newline
                && left.text == right.text
        }
        (None, None) => true,
        _ => false,
    }
}

fn most_common_origin_change(lines: &[LineEntry]) -> Option<ChangeId> {
    let mut counts: BTreeMap<ChangeId, usize> = BTreeMap::new();
    for line in lines {
        *counts
            .entry(line.line_id.origin_change.clone())
            .or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|(left_id, left_count), (right_id, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_id.cmp(left_id))
        })
        .map(|(change_id, _)| change_id)
}

fn newline_code(newline: NewlineKind) -> u8 {
    match newline {
        NewlineKind::None => 0,
        NewlineKind::Lf => 1,
        NewlineKind::Crlf => 2,
    }
}

fn decode_newline(code: u8) -> Result<NewlineKind> {
    match code {
        0 => Ok(NewlineKind::None),
        1 => Ok(NewlineKind::Lf),
        2 => Ok(NewlineKind::Crlf),
        _ => Err(Error::Corrupt(format!(
            "invalid small text newline code {code}"
        ))),
    }
}

fn write_change_id(out: &mut Vec<u8>, change_id: &ChangeId) {
    if let Some(hex_id) = change_id.0.strip_prefix(crate::ids::CHANGE_ID_PREFIX)
        && hex_id.len() == 64
        && let Ok(bytes) = hex::decode(hex_id)
    {
        out.push(0);
        out.extend_from_slice(&bytes);
        return;
    }
    out.push(1);
    write_bytes(out, change_id.0.as_bytes());
}

fn read_change_id(data: &[u8], cursor: &mut usize) -> Result<ChangeId> {
    match read_u8(data, cursor)? {
        0 => {
            let bytes = read_exact(data, cursor, 32)?;
            Ok(ChangeId(format!(
                "{}{}",
                crate::ids::CHANGE_ID_PREFIX,
                hex::encode(bytes)
            )))
        }
        1 => {
            let bytes = read_len_bytes(data, cursor)?;
            let value = String::from_utf8(bytes.to_vec()).map_err(|err| {
                Error::Corrupt(format!("small text table change id is not UTF-8: {err}"))
            })?;
            Ok(ChangeId(value))
        }
        tag => Err(Error::Corrupt(format!(
            "invalid small text change id tag {tag}"
        ))),
    }
}

fn write_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    write_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn read_len_bytes<'a>(data: &'a [u8], cursor: &mut usize) -> Result<&'a [u8]> {
    let len = read_varint(data, cursor)? as usize;
    read_exact(data, cursor, len)
}

fn read_exact<'a>(data: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8]> {
    if *cursor + len > data.len() {
        return Err(Error::Corrupt(
            "small text table ended unexpectedly".to_string(),
        ));
    }
    let bytes = &data[*cursor..*cursor + len];
    *cursor += len;
    Ok(bytes)
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn read_varint(data: &[u8], cursor: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    let mut shift = 0;
    for _ in 0..10 {
        let byte = read_u8(data, cursor)?;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
    Err(Error::Corrupt(
        "small text table varint is too long".to_string(),
    ))
}

fn read_u8(data: &[u8], cursor: &mut usize) -> Result<u8> {
    if *cursor >= data.len() {
        return Err(Error::Corrupt(
            "small text table ended unexpectedly".to_string(),
        ));
    }
    let byte = data[*cursor];
    *cursor += 1;
    Ok(byte)
}

pub(crate) fn preserves_base_line_order(base_order: &[String], lines: &[LineEntry]) -> bool {
    let positions = base_order
        .iter()
        .enumerate()
        .map(|(idx, line_id)| (line_id.as_str(), idx))
        .collect::<HashMap<_, _>>();
    let mut last = None;
    for line in lines {
        let line_id = line.line_id_key();
        let Some(position) = positions.get(line_id.as_str()).copied() else {
            continue;
        };
        if last.is_some_and(|last| position < last) {
            return false;
        }
        last = Some(position);
    }
    true
}

pub(crate) fn inserted_line_gaps(
    lines: &[LineEntry],
    base_keys: &HashSet<String>,
) -> BTreeSet<LineGap> {
    inserted_line_groups(lines, base_keys)
        .into_iter()
        .map(|(gap, _)| gap)
        .collect()
}

pub(crate) fn inserted_line_groups(
    lines: &[LineEntry],
    base_keys: &HashSet<String>,
) -> Vec<(LineGap, Vec<LineEntry>)> {
    let mut groups: Vec<(LineGap, Vec<LineEntry>)> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let line_id = line.line_id_key();
        if base_keys.contains(&line_id) {
            continue;
        }
        let gap = line_gap_at(lines, idx, base_keys);
        if let Some((last_gap, last_lines)) = groups.last_mut()
            && *last_gap == gap
        {
            last_lines.push(line.clone());
            continue;
        }
        groups.push((gap, vec![line.clone()]));
    }
    groups
}

pub(crate) fn line_gap_at(lines: &[LineEntry], idx: usize, base_keys: &HashSet<String>) -> LineGap {
    let previous = lines[..idx]
        .iter()
        .rev()
        .map(LineEntryExt::line_id_key)
        .find(|line_id| base_keys.contains(line_id));
    let next = lines[idx + 1..]
        .iter()
        .map(LineEntryExt::line_id_key)
        .find(|line_id| base_keys.contains(line_id));
    LineGap { previous, next }
}

pub(crate) fn replace_or_insert_line(
    lines: &mut Vec<LineEntry>,
    line_id: &str,
    replacement: LineEntry,
) {
    if let Some(line) = lines
        .iter_mut()
        .find(|line| line.line_id_key().as_str() == line_id)
    {
        *line = replacement;
    } else {
        lines.push(replacement);
    }
}

pub(crate) fn remove_line(lines: &mut Vec<LineEntry>, line_id: &str) {
    if let Some(idx) = lines
        .iter()
        .position(|line| line.line_id_key().as_str() == line_id)
    {
        lines.remove(idx);
    }
}

pub(crate) fn insert_lines_at_gap(
    lines: &mut Vec<LineEntry>,
    gap: &LineGap,
    inserted: Vec<LineEntry>,
) {
    let idx = if let Some(next) = &gap.next {
        lines
            .iter()
            .position(|line| line.line_id_key() == *next)
            .unwrap_or(lines.len())
    } else if let Some(previous) = &gap.previous {
        lines
            .iter()
            .position(|line| line.line_id_key() == *previous)
            .map(|idx| idx + 1)
            .unwrap_or(lines.len())
    } else {
        lines.len()
    };
    for (offset, line) in inserted.into_iter().enumerate() {
        lines.insert(idx + offset, line);
    }
}

pub(crate) fn order_key(line_number: u64) -> Vec<u8> {
    (line_number * ORDER_KEY_STEP).to_be_bytes().to_vec()
}

pub(crate) fn line_similarity(left: &[u8], right: &[u8]) -> f32 {
    if left == right {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let max = left.len().max(right.len()) as f32;
    let common = left
        .iter()
        .zip(right)
        .filter(|(left, right)| left == right)
        .count() as f32;
    common / max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_text_table_round_trips_line_metadata() {
        let origin = change_id(1);
        let other_origin = change_id(2);
        let content_change = change_id(3);
        let move_change = change_id(4);
        let lines = vec![
            line(
                LineId::new(origin.clone(), 1),
                b"hello".to_vec(),
                NewlineKind::Lf,
                origin.clone(),
                origin.clone(),
                None,
                LineFlags::default(),
            ),
            line(
                LineId::new(other_origin.clone(), 200),
                b"world".to_vec(),
                NewlineKind::Crlf,
                origin.clone(),
                content_change.clone(),
                Some(move_change.clone()),
                LineFlags {
                    generated: true,
                    redacted: true,
                },
            ),
            line(
                LineId::new(origin.clone(), 3),
                b"tail".to_vec(),
                NewlineKind::None,
                origin.clone(),
                origin,
                None,
                LineFlags::default(),
            ),
        ];

        let table = encode_small_text_table(&lines);
        let legacy_bytes = serde_cbor::ser::to_vec_packed(&lines).unwrap();
        let decoded = decode_small_text_table(&table).unwrap();

        assert_eq!(decoded, lines);
        assert!(table.len() < legacy_bytes.len());
    }

    #[test]
    fn small_text_table_rejects_corrupt_data() {
        let lines = vec![line(
            LineId::new(change_id(1), 1),
            b"hello".to_vec(),
            NewlineKind::Lf,
            change_id(1),
            change_id(1),
            None,
            LineFlags::default(),
        )];
        let mut table = encode_small_text_table(&lines);
        table.push(0);
        assert!(decode_small_text_table(&table).is_err());

        let mut table = encode_small_text_table(&lines);
        table[0] = 99;
        assert!(decode_small_text_table(&table).is_err());
    }

    fn change_id(byte: u8) -> ChangeId {
        ChangeId(format!("change_{}", hex::encode([byte; 32])))
    }

    fn line(
        line_id: LineId,
        text: Vec<u8>,
        newline: NewlineKind,
        introduced_by: ChangeId,
        last_content_change: ChangeId,
        last_move_change: Option<ChangeId>,
        flags: LineFlags,
    ) -> LineEntry {
        LineEntry {
            line_id,
            text_hash: sha256_hex(&text),
            text,
            newline,
            introduced_by,
            last_content_change,
            last_move_change,
            flags,
        }
    }
}
