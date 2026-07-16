# Persistent Path-Invariant Index Implementation Plan

## Task 1: Root schema and full builders

- Add Serde-defaulted `WorktreeRoot.case_fold_map_root`.
- Add helpers to build a sorted folded-key -> canonical-path Prolly tree.
- Populate it in every full root constructor, including workspace-view tests.
- Test ASCII/Unicode collisions and legacy root deserialization.

## Task 2: Indexed mutation helper

- Add `validate_and_update_case_fold_index(previous_root, removals, additions)`.
- Query only touched folded keys, apply removals before additions, and write one
  mutation batch.
- Return stable `PATH_INDEX_REQUIRED` for a legacy root with no index.
- Test rename, delete-then-add, within-batch collision, existing-root collision,
  and SHA/content-addressed node reuse.

## Task 3: Hot validator integration

- Replace patch and record final-root full scans with indexed validation.
- Integrate the returned index root into incremental root constructors.
- Ensure validation happens before content/operation/ref writes.
- Add structural counters proving zero full-root path loads and <=k lookups.

## Task 4: Rebuild and compatibility

- Extend `trail index rebuild` to backfill the current root's case-fold index
  without changing visible files.
- Add `PATH_INDEX_REQUIRED` CLI/JSON diagnostics and recovery guidance.
- Test old-root open/read, mutation failure, rebuild, then successful mutation.

## Task 5: Scale gates and verification

- Extend the CLI scale harness with 1k/100k/1M structured patch and record
  structural metrics.
- Gate indexed mode, lookup count, and zero full-root path loads.
- Run fmt, focused tests, `cargo test -p trail`, 100k acceptance, and scheduled
  1M automation; independently review before completion.
