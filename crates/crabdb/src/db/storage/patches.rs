use super::*;

impl CrabDb {
    pub(crate) fn attach_patches(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
        summaries: &mut [FileDiffSummary],
    ) -> Result<()> {
        let mut old_entries = BTreeMap::new();
        let mut new_entries = BTreeMap::new();
        for summary in summaries.iter() {
            if let Some(old_entry) = summary
                .old_path
                .as_ref()
                .and_then(|path| left.get(path))
                .or_else(|| left.get(&summary.path))
            {
                old_entries.insert(
                    summary
                        .old_path
                        .clone()
                        .unwrap_or_else(|| summary.path.clone()),
                    old_entry.clone(),
                );
            }
            if let Some(new_entry) = right.get(&summary.path) {
                new_entries.insert(summary.path.clone(), new_entry.clone());
            }
        }

        let old_bytes = self.materialize_entries_bytes(&old_entries)?;
        let new_bytes = self.materialize_entries_bytes(&new_entries)?;
        for summary in summaries {
            let old_key = summary.old_path.as_deref().unwrap_or(&summary.path);
            let old_text = old_bytes
                .get(old_key)
                .cloned()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            let new_text = new_bytes
                .get(&summary.path)
                .cloned()
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
