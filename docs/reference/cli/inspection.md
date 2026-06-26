# CLI Reference: Inspection

Inspection commands expose high-level diffs and low-level storage objects.

## `diff`

```text
crabdb diff <RANGE> [--patch] [--stat] [--show-line-ids]
crabdb diff --root <LEFT_ROOT>..<RIGHT_ROOT> [--patch] [--stat] [--show-line-ids]
crabdb diff --dirty [--patch] [--stat] [--show-line-ids]
```

Use exactly one of range, `--root`, or `--dirty`.

## `object`

```text
crabdb object show <OBJECT_ID>
```

Shows generic object metadata and known object summaries.

## `root`

```text
crabdb root show <ROOT_ID>
```

Decodes a `WorktreeRoot`.

## `text`

```text
crabdb text show <TEXT_ID> [--limit <N>]
```

Default limit is 50.

## `map`

```text
crabdb map range <MAP_ID> [--map-type <TYPE>] [--start <KEY>] [--end <KEY>] [--limit <N>]
crabdb map diff <LEFT_MAP_ID> <RIGHT_MAP_ID> [--map-type <TYPE>] [--start <KEY>] [--end <KEY>] [--limit <N>]
```

Map types:

- `raw`
- `path`
- `file-index`
- `text-order`
- `line-index`

These commands are advanced/internal debugging tools for prolly maps and object storage.

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/inspect_args.rs`
- Renderers: `crates/crabdb/src/cli/command/render/inspection.rs`
- Tests: `inspection_apis_decode_objects_roots_and_texts`, `map_debug_commands_decode_known_prolly_maps`

