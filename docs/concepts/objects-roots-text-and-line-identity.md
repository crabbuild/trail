# Objects, Roots, Text, and Line Identity

CrabDB stores content-addressed objects and uses stable IDs for files and lines.

## Object Kinds

The public object kinds include:

- `WorktreeRoot`
- `TextContent`
- `Operation`
- `Blob`
- `Message`
- `ConflictSet`
- `Anchor`

Objects are stored by ID and kind. The `object`, `root`, and `text` CLI commands inspect them.

## Worktree Roots

A `WorktreeRoot` contains map roots for path and file indexes, a file count, total text bytes, and the operation that created it.

Each file entry stores:

- `file_id`
- `kind`: `Text`, `OpaqueText`, or `Binary`
- mode and executable bit
- content reference
- size and content hash
- creating and last-changing operations

## Text Representations

`TextContent` can be represented as:

- `TreeText`: line order and line indexes in prolly maps.
- `LazyText`: full blob plus operation that introduced it.
- `OpaqueText`: blob plus reason such as too large, line too long, invalid UTF-8, or binary-like content.
- `SmallTextTable` or `SmallText`: compact storage for small text.

## Line Identity

Lines have `LineId` values with an origin operation and local sequence. Line changes track added, modified, deleted, and moved lines.

Use line IDs when an agent needs precise edits:

```sh
crabdb diff --dirty --show-line-ids
crabdb lane diff doc-bot --patch --show-line-ids
```

Structured patches can replace a specific line by `line_id` and optional `expected_text`.

## Code Facts Used

- Object model: `crates/crabdb/src/model/domain/objects.rs`
- IDs: `crates/crabdb/src/ids.rs`
- Line changes: `crates/crabdb/src/model/lane/changes.rs`
- Tests: `same_position_rewrite_preserves_line_identity`, `copying_a_file_allocates_a_new_file_identity`, `small_text_policy_avoids_prolly_text_maps_for_tiny_files`

