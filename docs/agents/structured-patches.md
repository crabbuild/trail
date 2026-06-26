# Structured Patches

Structured patches let tools and agents edit an agent branch without directly touching the main workspace.

## CLI

```sh
crabdb agent apply-patch doc-bot --patch patch.json
crabdb agent turn apply-patch <turn-id> --patch patch.json
```

Use `--allow-ignored` only for intentional ignored fixtures.

## Patch Document

```json
{
  "base_change": "optional-change-id",
  "message": "describe the patch",
  "session_id": "optional-session-id",
  "allow_ignored": false,
  "edits": []
}
```

`base_change`, `message`, `session_id`, and `allow_ignored` are optional. `edits` is required by the public patch document type.

## Write Text

```json
{
  "op": "write",
  "path": "docs/notes.md",
  "content": "notes\n",
  "executable": false
}
```

## Write Bytes

```json
{
  "op": "write_bytes",
  "path": "image.bin",
  "bytes_hex": "00ff",
  "executable": false
}
```

## Replace a Stable Line

```json
{
  "op": "replace_line",
  "path": "README.md",
  "line_id": "<change-id>:1",
  "expected_text": "old line\n",
  "new_text": "new line\n"
}
```

`expected_text` is optional but useful for safety.

## Delete and Rename

```json
{ "op": "delete", "path": "old.md" }
```

```json
{ "op": "rename", "from": "README.md", "to": "docs/README.md" }
```

## HTTP Alternate Shape

The HTTP patch parser also accepts a `files` array with `add_text`, `modify_text`, `write_bytes`, `delete`, and `rename` entries. The parser converts these entries into the same patch edits.

## Code Facts Used

- Patch schema: `crates/crabdb/src/model/inspect/patch.rs`
- HTTP patch request schema: `crates/crabdb/src/server/request_types/patches.rs`
- Patch policy: `crates/crabdb/src/db/agent/patch_policy.rs`
- Tests: `agent_patch_incrementally_handles_rename_delete_and_write`, `agent_patch_can_replace_stable_line_with_expected_text`

