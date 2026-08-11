use super::*;

pub(crate) fn count_line_delta(changes: &[LineChange]) -> (u64, u64) {
    let mut additions = 0;
    let mut deletions = 0;
    for change in changes {
        match change.kind {
            LineChangeKind::Added => additions += 1,
            LineChangeKind::Deleted => deletions += 1,
            LineChangeKind::Modified => {
                additions += 1;
                deletions += 1;
            }
            LineChangeKind::Moved => {}
        }
    }
    (additions, deletions)
}

impl Trail {
    pub(crate) fn summarize_file_changes_from_content(
        &self,
        changes: &[FileChange],
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
    ) -> Result<Vec<FileDiffSummary>> {
        changes
            .iter()
            .map(|change| {
                let (additions, deletions) = self.content_line_delta(change, left, right)?;
                Ok(FileDiffSummary {
                    path: change.path.clone(),
                    old_path: change.old_path.clone(),
                    kind: change.kind.clone(),
                    before_hash: change.before_hash.clone(),
                    after_hash: change.after_hash.clone(),
                    additions,
                    deletions,
                    line_changes: Vec::new(),
                    patch: None,
                })
            })
            .collect()
    }

    fn content_line_delta(
        &self,
        change: &FileChange,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
    ) -> Result<(u64, u64)> {
        if !matches!(
            change.kind,
            FileChangeKind::Modified | FileChangeKind::TypeChanged
        ) {
            return Ok(count_line_delta(&change.line_changes));
        }
        let old_path = change.old_path.as_deref().unwrap_or(&change.path);
        let (Some(old_entry), Some(new_entry)) = (left.get(old_path), right.get(&change.path))
        else {
            return Ok(count_line_delta(&change.line_changes));
        };
        let (FileContentRef::Text(old_text), FileContentRef::Text(new_text)) =
            (&old_entry.content, &new_entry.content)
        else {
            return Ok(count_line_delta(&change.line_changes));
        };
        let old_bytes = materialize_lines(&self.load_text_lines(old_text)?);
        let new_bytes = materialize_lines(&self.load_text_lines(new_text)?);
        let (Ok(old_text), Ok(new_text)) =
            (String::from_utf8(old_bytes), String::from_utf8(new_bytes))
        else {
            return Ok(count_line_delta(&change.line_changes));
        };
        let diff = TextDiff::from_lines(&old_text, &new_text);
        let mut additions = 0_u64;
        let mut deletions = 0_u64;
        for delta in diff.iter_all_changes() {
            match delta.tag() {
                ChangeTag::Insert => additions = additions.saturating_add(1),
                ChangeTag::Delete => deletions = deletions.saturating_add(1),
                ChangeTag::Equal => {}
            }
        }
        Ok((additions, deletions))
    }
}

pub(crate) fn summarize_file_changes(changes: &[FileChange]) -> Vec<FileDiffSummary> {
    changes
        .iter()
        .map(|change| {
            let (additions, deletions) = count_line_delta(&change.line_changes);
            FileDiffSummary {
                path: change.path.clone(),
                old_path: change.old_path.clone(),
                kind: change.kind.clone(),
                before_hash: change.before_hash.clone(),
                after_hash: change.after_hash.clone(),
                additions,
                deletions,
                line_changes: Vec::new(),
                patch: None,
            }
        })
        .collect()
}

pub(crate) fn attach_line_changes(changes: &[FileChange], summaries: &mut [FileDiffSummary]) {
    for summary in summaries {
        summary.line_changes = changes
            .iter()
            .find(|change| {
                change.path == summary.path
                    && change.old_path == summary.old_path
                    && change.kind == summary.kind
            })
            .map(|change| change.line_changes.clone())
            .unwrap_or_default();
    }
}

pub(crate) fn worktree_state_from_changes(changed_paths: &[FileDiffSummary]) -> WorktreeState {
    if changed_paths.is_empty() {
        WorktreeState::Clean
    } else if changed_paths
        .iter()
        .any(|summary| summary.kind == FileChangeKind::Added)
    {
        WorktreeState::DirtyUntracked
    } else {
        WorktreeState::DirtyTracked
    }
}
