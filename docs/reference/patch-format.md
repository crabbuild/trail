# Patch Format Reference

Structured patches are used by:

- `trail lane apply-patch`
- `trail lane turn apply-patch`
- HTTP `POST /v1/lanes/{lane_or_id}/patches`
- HTTP `POST /v1/lane/turns/{turn_id}/patches`
- MCP `trail.apply_patch`

## Patch Document

```json
{
  "base_change": "current-lane-head-change-id",
  "message": "optional message",
  "session_id": "optional-session-id",
  "allow_ignored": false,
  "allow_stale": false,
  "edits": []
}
```

Fields:

- `base_change`: expected lane head change for direct lane patches. Turn-linked patches may omit it because the turn's `before_change` is used as the freshness guard.
- `message`: optional operation message.
- `session_id`: optional session link.
- `allow_ignored`: default `false`; opt-in for ignored paths.
- `allow_stale`: default `false`; set `true` only to bypass the `base_change` freshness check intentionally.
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

`expected_text` is required for `replace_line`. It protects against replacing a line whose current text no longer matches.

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

The HTTP and MCP parsers also accept:

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

`files` entries are converted into the public edit forms. External HTTP/MCP
patch requests must provide either a non-empty `edits` array or a non-empty
`files` array, not both.
The generated OpenAPI schema enumerates the supported `edits`, `files`, and
nested text-edit variants with `additionalProperties: false`; unknown fields are
rejected instead of ignored.

## Policy

Patch paths are normalized and checked. Parent-directory escapes, absolute
paths, backslash separators on non-Windows platforms, non-NFC Unicode spellings,
slash lookalikes, invisible Unicode format controls, Windows-reserved device
names and aliases, internal paths, and hardcoded private paths are rejected.
Workspace-ignored paths require `allow_ignored`.

Direct lane patches are rejected unless `base_change` matches the current lane head. If a tool intentionally wants to apply against whatever the current head is, it must set `allow_stale: true` or use `--allow-stale`.

Patch messages and edit payloads are secret-scanned before storage. Assignment-style credentials such as `API_KEY=...`, bearer tokens, and private-key PEM blocks reject the patch; benign prose such as “token expiration logic” is allowed.

## Code Facts Used

- Public patch schema: `crates/trail/src/model/inspect/patch.rs`
- HTTP patch schema: `crates/trail/src/server/request_types/patches.rs`
- HTTP parser: `crates/trail/src/server/route/utils.rs`
- Patch policy: `crates/trail/src/db/lane/patch_policy.rs`
