use super::*;

pub(crate) fn known_gc_object_kinds() -> HashSet<&'static str> {
    [
        WORKTREE_ROOT_KIND,
        TEXT_CONTENT_KIND,
        OPERATION_KIND,
        BLOB_KIND,
        MESSAGE_KIND,
        ANCHOR_KIND,
    ]
    .into_iter()
    .collect()
}
