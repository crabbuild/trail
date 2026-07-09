use super::*;

impl Trail {
    pub fn inspect_map_range(
        &self,
        map_id: &str,
        map_type: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: usize,
    ) -> Result<MapRangeReport> {
        let map_type = parse_map_inspect_type(map_type)?;
        let start_bytes = start
            .map(parse_map_key_spec)
            .transpose()?
            .unwrap_or_default();
        let end_bytes = end.map(parse_map_key_spec).transpose()?;
        let tree = tree_from_root_hex(Some(map_id))?;
        let iter = self
            .prolly
            .range(&tree, &start_bytes, end_bytes.as_deref())?;
        let mut entries = Vec::new();
        let mut truncated = false;
        for item in iter {
            let (key, value) = item?;
            if limit > 0 && entries.len() >= limit {
                truncated = true;
                break;
            }
            entries.push(MapEntryInspect {
                key: inspect_map_key(map_type, &key),
                value: inspect_map_value(map_type, &value),
            });
        }
        Ok(MapRangeReport {
            map_id: map_id.to_string(),
            map_type: map_type.as_str().to_string(),
            start: start.map(str::to_string),
            end: end.map(str::to_string),
            entries,
            truncated,
        })
    }

    pub fn inspect_map_diff(
        &self,
        left_map_id: &str,
        right_map_id: &str,
        map_type: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: usize,
    ) -> Result<MapDiffReport> {
        let map_type = parse_map_inspect_type(map_type)?;
        let start_bytes = start
            .map(parse_map_key_spec)
            .transpose()?
            .unwrap_or_default();
        let end_bytes = end.map(parse_map_key_spec).transpose()?;
        let left = tree_from_root_hex(Some(left_map_id))?;
        let right = tree_from_root_hex(Some(right_map_id))?;
        let diffs = self
            .prolly
            .range_diff(&left, &right, &start_bytes, end_bytes.as_deref())?;
        let mut changes = Vec::new();
        let mut truncated = false;
        for diff in diffs {
            if limit > 0 && changes.len() >= limit {
                truncated = true;
                break;
            }
            changes.push(inspect_map_diff_entry(map_type, diff));
        }
        Ok(MapDiffReport {
            left_map_id: left_map_id.to_string(),
            right_map_id: right_map_id.to_string(),
            map_type: map_type.as_str().to_string(),
            start: start.map(str::to_string),
            end: end.map(str::to_string),
            changes,
            truncated,
        })
    }
}
