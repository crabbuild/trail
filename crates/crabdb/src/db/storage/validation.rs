use super::*;
use crate::db::storage::content::validate_full_text_blob;

impl CrabDb {
    pub(crate) fn validate_worktree_root(&self, root: &WorktreeRoot) -> Result<()> {
        let path_tree = worktree_root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let index_tree = worktree_root_map_tree_from_root_hex(root.file_index_map_root.as_deref())?;
        let path_entries = self
            .root_prolly
            .range(&path_tree, &[], None)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut count = 0;
        for (path, value) in path_entries {
            count += 1;
            let entry: FileEntry = from_cbor(&value)?;
            let indexed = self
                .root_prolly
                .get(&index_tree, &entry.file_id.encode_key())?;
            if indexed.as_deref() != Some(path.as_slice()) {
                return Err(Error::Corrupt(format!(
                    "file index mismatch for {}",
                    String::from_utf8_lossy(&path)
                )));
            }
        }
        if count != root.file_count {
            return Err(Error::Corrupt(format!(
                "root file_count {} but path map has {}",
                root.file_count, count
            )));
        }
        Ok(())
    }

    pub(crate) fn validate_text_content(&self, text_id: &ObjectId) -> Result<()> {
        let content: TextContent = self.get_object(TEXT_CONTENT_KIND, text_id)?;
        if let Some(blob_id) = &content.full_bytes_blob_id {
            let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
            if blob.content_hash != content.content_hash {
                return Err(Error::Corrupt(format!(
                    "text full blob hash {} does not match content hash {}",
                    blob.content_hash, content.content_hash
                )));
            }
            if blob.bytes.len() as u64 != content.byte_count {
                return Err(Error::Corrupt(format!(
                    "text full blob byte_count {} does not match content byte_count {}",
                    blob.bytes.len(),
                    content.byte_count
                )));
            }
        }
        let count = match &content.representation {
            TextRepresentation::SmallText { lines } => {
                if materialize_lines(lines).len() as u64 != content.byte_count {
                    return Err(Error::Corrupt(format!(
                        "small text byte_count {} does not match materialized lines",
                        content.byte_count
                    )));
                }
                lines.len() as u64
            }
            TextRepresentation::SmallTextTable { table } => {
                let lines = decode_small_text_table(table)?;
                if materialize_lines(&lines).len() as u64 != content.byte_count {
                    return Err(Error::Corrupt(format!(
                        "small text table byte_count {} does not match materialized lines",
                        content.byte_count
                    )));
                }
                lines.len() as u64
            }
            TextRepresentation::TreeText => {
                let order_tree = tree_from_root_hex(content.order_map_root.as_deref())?;
                let index_tree = tree_from_root_hex(content.line_index_map_root.as_deref())?;
                let mut count = 0;
                for item in self.prolly.range(&order_tree, &[], None)? {
                    let (order_key, value) = item?;
                    count += 1;
                    let entry: LineEntry = from_cbor(&value)?;
                    let indexed = self.prolly.get(&index_tree, &entry.line_id.encode_key())?;
                    if indexed.as_deref() != Some(order_key.as_slice()) {
                        return Err(Error::Corrupt(format!(
                            "line index mismatch for {}",
                            entry.line_id.local_seq
                        )));
                    }
                }
                count
            }
            TextRepresentation::LazyText { blob_id, .. } => {
                if content.order_map_root.is_some() || content.line_index_map_root.is_some() {
                    return Err(Error::Corrupt(
                        "lazy text must not have line-order or line-index roots".to_string(),
                    ));
                }
                let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
                validate_full_text_blob(&content, &blob)?;
                split_lines(&blob.bytes).len() as u64
            }
            TextRepresentation::OpaqueText { blob_id, .. } => {
                let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
                split_lines(&blob.bytes).len() as u64
            }
        };
        if count != content.line_count {
            return Err(Error::Corrupt(format!(
                "text line_count {} but order map has {}",
                content.line_count, count
            )));
        }
        Ok(())
    }
}
