# Structured Patches

Structured patches let tools and agents edit a lane branch without directly touching the main workspace.

## CLI

```sh
trail lane apply-patch doc-bot --patch patch.json
trail lane turn apply-patch <turn-id> --patch patch.json
```

Use `--allow-ignored` only for intentional ignored fixtures.

## Patch Document

```json
{
  "base_change": "current-lane-head-change-id",
  "message": "describe the patch",
  "session_id": "optional-session-id",
  "allow_ignored": false,
  "allow_stale": false,
  "edits": []
}
```

Direct lane patches require `base_change` to match the current lane head. Turn-linked patches may omit it because the turn's `before_change` guards freshness. `message`, `session_id`, `allow_ignored`, and `allow_stale` are optional. Use `allow_stale: true` or `--allow-stale` only when intentionally applying without a fresh base. `edits` is required by the public patch document type.

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

`expected_text` is required for `replace_line`. It must match the current line text or the patch is rejected before mutation.

## Delete and Rename

```json
{ "op": "delete", "path": "old.md" }
```

```json
{ "op": "rename", "from": "README.md", "to": "docs/README.md" }
```

## HTTP/MCP Alternate Shape

The HTTP and MCP patch parsers also accept a `files` array with `add_text`, `modify_text`, `write_bytes`, `delete`, and `rename` entries. The parser converts these entries into the same patch edits. External patch requests must use either a non-empty `edits` array or a non-empty `files` array, not both.

Structured patch messages and edit payloads are secret-scanned before storage. Assignment-style credentials, bearer tokens, and private-key PEM blocks reject the patch rather than writing those bytes into Trail objects.
Patch paths are normalized and reject parent-directory escapes, absolute paths,
backslash separators on non-Windows platforms, non-NFC Unicode spellings, slash
lookalikes, invisible Unicode format controls, Windows-reserved device names and
aliases, internal paths, and hardcoded private paths.

## Code Facts Used

- Patch schema: `crates/trail/src/model/inspect/patch.rs`
- HTTP patch request schema: `crates/trail/src/server/request_types/patches.rs`
- Patch policy: `crates/trail/src/db/lane/patch_policy.rs`
- Tests: `lane_patch_incrementally_handles_rename_delete_and_write`, `lane_patch_can_replace_stable_line_with_expected_text`, `lane_payload_secret_scan_rejects_patch_content_and_redacts_stored_payloads`
