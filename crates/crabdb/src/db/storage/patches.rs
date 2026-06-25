use super::*;

impl CrabDb {
    pub(crate) fn attach_patches(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
        summaries: &mut [FileDiffSummary],
    ) -> Result<()> {
        for summary in summaries {
            let old = summary
                .old_path
                .as_ref()
                .and_then(|path| left.get(path))
                .or_else(|| left.get(&summary.path));
            let new = right.get(&summary.path);
            let old_text = old
                .map(|entry| self.materialize_entry_bytes(entry))
                .transpose()?
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            let new_text = new
                .map(|entry| self.materialize_entry_bytes(entry))
                .transpose()?
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            summary.patch = Some(unified_patch(
                summary.old_path.as_deref().unwrap_or(&summary.path),
                &summary.path,
                &old_text,
                &new_text,
            ));
        }
        Ok(())
    }
}
