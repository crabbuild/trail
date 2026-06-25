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
