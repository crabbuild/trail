# Record and Provenance

## Record a Coherent Operation

Inspect the patch, then record either the complete intended change or explicit paths:

```sh
trail status
trail diff --dirty --patch --show-line-ids
trail record -m "Describe why this change exists"
trail record --paths README.md docs -m "Update documentation"
```

Do not absorb unrelated user changes. Check uncertain paths with:

```sh
trail ignore list
trail ignore check path/to/file
```

An ignored fixture may be recorded only after its contents are reviewed and the user intends it to become Trail history.

## Query History and Identity

```sh
trail timeline --limit 20
trail show <change-id>
trail history path/to/file
trail why path/to/file:42
trail code-from <session-or-lane-id>
```

Use Trail for recorded local operations and Git log/blame for committed shared history. State which layer produced the answer. Prefer `--line-id` or `--file-id` when a stable identity is already known.

## Verify

After recording, rerun `trail status` and inspect the recorded operation. Report any paths deliberately left dirty.
