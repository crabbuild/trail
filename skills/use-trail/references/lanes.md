# Direct Lane Workflow

Use a lane for one bounded unit of active work that needs isolation, provenance, gates, handoff, recovery, or coordinated merge. A lane is a Trail ref under `refs/lanes/<name>` plus task activity; it is not a Git branch and does not launch an AI agent by itself.

## Create the Right Workdir

From the original Trail workspace:

```sh
trail lane spawn <lane> --from main --workdir-mode full-cow
trail lane status <lane>
trail lane workdir <lane>
```

Choose intentionally:

- `virtual`: no filesystem workdir; use structured patches.
- `sparse`: selected paths only; supply `--paths`.
- `full-cow`: portable full materialization.
- `overlay-cow`: runtime-mounted FUSE COW where supported.
- `nfs-cow`: macOS loopback NFS COW.

For a narrow large-repository task:

```sh
trail lane spawn <lane> --from main --workdir-mode sparse --paths docs README.md
trail lane claim <lane> docs --ttl-secs 1800
trail lane claim <lane> README.md --ttl-secs 1800
```

Edit only in the returned lane workdir. From that workdir, pass `--workspace <original-root>` to Trail commands unless workspace discovery is known to resolve correctly.

For a layered lane with a single supported environment at the selected root, build or
reuse its immutable environment before starting work:

```sh
trail env adapters
trail env sync <lane>
trail env status <lane>
```

`trail env adapters` lists canonical identities, accepted selectors, stability, and
manifest names used by side-effect-free discovery. It does not probe package managers,
compilers, or repository files.

For semantic planning beyond a command profile, install an experimental local adapter
package explicitly:

```sh
trail env plugin install path/to/package
trail env adapters
trail env plan <lane> --adapter namespace/name@1
trail env plugin remove namespace/name@1
```

Trail content-addresses and revalidates the package, gives its planner only bounded bytes
from the pinned root, and runs it without repository, network, child-process, database,
mount, or publication authority. Local packages are experimental; signed organization
catalogs and WASI distribution are not yet available.

Auto-detection supports Node, the experimental Cargo target-seed adapter,
single-module Go vendoring, and lane-private CMake build trees. For a polyglot root,
select explicitly with
`--adapter trail/node@1`, `--adapter trail/cargo-target-seed@1`, or
`--adapter trail/go-vendor@1`; use `--adapter trail/cmake-build@1` for CMake and
`--path <root>` for a nested component.
Environment synchronization requires an unmounted lane because it atomically advances
the environment binding generation. `trail deps sync` remains the Node compatibility
command.

For CMake, synchronization provisions the lane-private build directory without running
configure in a disposable staging path. Configure and build inside the lane so
`CMakeCache.txt` records the correct mounted path:

```sh
trail env sync <lane> --adapter trail/cmake-build@1
trail lane exec <lane> -- cmake -S . -B build -G Ninja
trail lane exec <lane> -- cmake --build build
```

Inspect a monorepo before executing installers, then activate every non-conflicting
proposal together:

```sh
trail env discover <lane>
trail env plan <lane>
trail env sync-all <lane>
trail env generation <lane>
```

`trail env plan` is read-only and shows the normalized component key, input hashes,
resolved executable identity, argv, mount, portability, and capability grants before
synchronization. Repository-defined `trail/command@1` components may be declared in
`trail.environment.toml`; execution uses macOS sandbox-exec, Linux Landlock plus
seccomp, or a capability-free Windows AppContainer constrained by a one-process Job
Object, and fails closed when the required native enforcement is unavailable.
If discovery reports multiple components at one root, pass `--component <id>` to
`env plan` or `env sync`, or use `env sync-all` to activate the whole environment.

`sync-all` builds components before changing mounts; activation advances one durable
generation or leaves the predecessor authoritative.

## Materialized Workdir Changes

Preview before recording:

```sh
trail --workspace <root> lane record <lane> --preview --json
trail --workspace <root> lane record <lane> -m "Describe the bounded change"
trail --workspace <root> lane diff <lane> --patch --show-line-ids
```

The preview exposes changed, ignored, risky, and oversized paths plus policy decisions. Resolve policy failures; do not reach for `--force` or ignored-path overrides.

For sparse workdirs, read or hydrate before editing:

```sh
trail lane read <lane> path/to/file
trail lane hydrate <lane> path/to/file
trail lane sync-workdir <lane> --paths path/to/file --include-neighbors
```

Never sync over a dirty workdir without first inspecting and preserving its changes.

## Structured Patches

Use a virtual lane when a host can issue typed edits without a filesystem:

```sh
trail lane spawn <lane> --from main --workdir-mode virtual
trail lane apply-patch <lane> --patch patch.json
```

A direct patch must carry the current lane head as `base_change`:

```json
{
  "base_change": "<current-lane-head-change-id>",
  "message": "Describe the edit",
  "allow_ignored": false,
  "allow_stale": false,
  "edits": [
    {
      "op": "replace_line",
      "path": "README.md",
      "line_id": "<line-id>",
      "expected_text": "old text\n",
      "new_text": "new text\n"
    }
  ]
}
```

Use `replace_line` with both stable `line_id` and `expected_text` for sensitive edits. Supported native operations are `write`, `write_bytes`, `replace_line`, `delete`, and `rename`. Do not set `allow_stale` or `allow_ignored` unless the user explicitly accepts the specific race or ignored artifact. Trail rejects unsafe paths and secret-like payloads.

## Capture Sessions and Turns

Use explicit activity capture when transcript and causal history matter:

```sh
trail session start <lane> --title "Task title" --id <session-id>
trail lane turn start <lane> --title "Prompt-sized unit"
trail lane turn message <turn-id> --role user --text "Request"
trail lane turn apply-patch <turn-id> --patch patch.json
trail lane turn end <turn-id> --status completed
```

A lane can exist without a session. Do not fabricate transcript data that the host did not actually capture.

## Validate, Review, and Merge

Run commands inside the lane workdir and record gates:

```sh
trail lane test <lane> --suite unit -- cargo test
trail lane eval <lane> --suite quality -- ./scripts/run-eval.sh
trail lane gates <lane> --limit 20
```

Review all evidence:

```sh
trail lane review <lane>
trail lane contribution <lane>
trail lane readiness <lane>
trail lane diff <lane> --patch --show-line-ids
trail approvals list --lane <lane>
```

If readiness reports `dependency_environment_stale`, inspect the exact cause before
rebuilding:

```sh
trail env status <lane>
trail env explain <lane> --component <component-id>
trail env plan <lane> --component <component-id>
```

Explanation reports name changed inputs, tools, platforms, and policies without
rendering their values. Use `--offset` and `--limit` for large monorepos.

Stop on readiness blockers. Preview refresh and merge:

```sh
trail lane refresh-preview <lane> --target main
trail merge-lane <lane> --into main --dry-run
```

For shared targets, queue rather than directly merging:

```sh
trail merge-queue add <lane> --into main
trail merge-queue explain <lane>
trail merge-queue run
```

Queue execution is consequential and re-runs readiness. Do it only when authorized. Remove a lane only after verifying it is merged or intentionally abandoned; `lane rm --force` is not routine cleanup.
