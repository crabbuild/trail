use super::*;

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
        if let Some((last_gap, last_lines)) = groups.last_mut() {
            if *last_gap == gap {
                last_lines.push(line.clone());
                continue;
            }
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
    let mut idx = if let Some(next) = &gap.next {
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
    for line in inserted {
        lines.insert(idx, line);
        idx += 1;
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
