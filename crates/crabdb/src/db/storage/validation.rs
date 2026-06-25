use super::*;

impl CrabDb {
    pub(crate) fn validate_worktree_root(&self, root: &WorktreeRoot) -> Result<()> {
        let path_tree = tree_from_root_hex(root.path_map_root.as_deref())?;
        let index_tree = tree_from_root_hex(root.file_index_map_root.as_deref())?;
        let path_entries = self
            .prolly
            .range(&path_tree, &[], None)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut count = 0;
        for (path, value) in path_entries {
            count += 1;
            let entry: FileEntry = from_cbor(&value)?;
            let indexed = self.prolly.get(&index_tree, &entry.file_id.encode_key())?;
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
        if count != content.line_count {
            return Err(Error::Corrupt(format!(
                "text line_count {} but order map has {}",
                content.line_count, count
            )));
        }
        Ok(())
    }
}
