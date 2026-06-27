# Patch Format Reference

Structured patches are used by:

- `crabdb lane apply-patch`
- `crabdb lane turn apply-patch`
- HTTP `POST /v1/lanes/{lane_or_id}/patches`
- HTTP `POST /v1/lane/turns/{turn_id}/patches`
- MCP `crabdb.apply_patch`

## Patch Document

```json
{
  "base_change": "optional-change-id",
  "message": "optional message",
  "session_id": "optional-session-id",
  "allow_ignored": false,
  "edits": []
}
```

Fields:

- `base_change`: optional expected base change.
- `message`: optional operation message.
- `session_id`: optional session link.
- `allow_ignored`: default `false`; opt-in for ignored paths.
- `edits`: edit list.

## Edit: `write`

```json
{
  "op": "write",
  "path": "docs/file.md",
  "content": "text\n",
  "executable": false
}
```

## Edit: `write_bytes`

```json
{
  "op": "write_bytes",
  "path": "artifact.bin",
  "bytes_hex": "00ff",
  "executable": false
}
```

## Edit: `replace_line`

```json
{
  "op": "replace_line",
  "path": "README.md",
  "line_id": "<change-id>:<local-seq>",
  "expected_text": "old\n",
  "new_text": "new\n"
}
```

`expected_text` defaults to absent. When present, it protects against replacing a line whose current text no longer matches.

## Edit: `delete`

```json
{
  "op": "delete",
  "path": "old.md"
}
```

## Edit: `rename`

```json
{
  "op": "rename",
  "from": "README.md",
  "to": "docs/README.md"
}
```

## HTTP `files` Compatibility Shape

The HTTP parser also accepts:

```json
{
  "message": "edit text",
  "files": [
    {
      "type": "add_text",
      "path": "docs/new.md",
      "content": "hello\n"
    },
    {
      "type": "modify_text",
      "path": "README.md",
      "edits": [
        {
          "type": "modify_line",
          "line_id": "<line-id>",
          "expected_text": "old\n",
          "new_text": "new\n"
        }
      ]
    }
  ]
}
```

`files` entries are converted into the public edit forms.

## Policy

Patch paths are normalized and checked. Internal paths and hardcoded private paths are rejected. Workspace-ignored paths require `allow_ignored`.

## Code Facts Used

- Public patch schema: `crates/crabdb/src/model/inspect/patch.rs`
- HTTP patch schema: `crates/crabdb/src/server/request_types/patches.rs`
- HTTP parser: `crates/crabdb/src/server/route/utils.rs`
- Patch policy: `crates/crabdb/src/db/lane/patch_policy.rs`

