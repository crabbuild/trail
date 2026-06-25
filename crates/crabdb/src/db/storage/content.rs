use super::*;

impl CrabDb {
    pub(crate) fn load_root_files(
        &self,
        root_id: &ObjectId,
    ) -> Result<BTreeMap<String, FileEntry>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = tree_from_root_hex(root.path_map_root.as_deref())?;
        let iter = self.prolly.range(&tree, &[], None)?;
        let mut out = BTreeMap::new();
        for item in iter {
            let (key, value) = item?;
            let path = String::from_utf8(key)
                .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
            let entry: FileEntry = from_cbor(&value)?;
            out.insert(path, entry);
        }
        Ok(out)
    }

    pub(crate) fn load_text_lines(&self, text_id: &ObjectId) -> Result<Vec<LineEntry>> {
        let content: TextContent = self.get_object(TEXT_CONTENT_KIND, text_id)?;
        let tree = tree_from_root_hex(content.order_map_root.as_deref())?;
        let iter = self.prolly.range(&tree, &[], None)?;
        let mut out = Vec::new();
        for item in iter {
            let (_, value) = item?;
            out.push(from_cbor(&value)?);
        }
        Ok(out)
    }

    pub(crate) fn materialize_entry_bytes(&self, entry: &FileEntry) -> Result<Vec<u8>> {
        match &entry.content {
            FileContentRef::Text(text_id) => {
                let lines = self.load_text_lines(text_id)?;
                Ok(materialize_lines(&lines))
            }
            FileContentRef::Opaque(blob_id) | FileContentRef::Binary(blob_id) => {
                let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
                Ok(blob.bytes)
            }
        }
    }

    pub(crate) fn materialize_files(
        &self,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        materialize_into(
            &self.workspace_root,
            &self.workspace_root,
            previous,
            target,
            |entry| self.materialize_entry_bytes(entry),
        )
    }
}
