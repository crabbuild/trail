# Performance and Scale Benchmarks

Use the CLI scale benchmark to verify large-repo and agent orchestration behavior before treating a change as production ready.

## CI Smoke

```sh
make bench-cli-scale-smoke
```

The smoke target defaults to 1,000 synthetic files and is intended for pull-request CI. It exercises init, clean and dirty status, dirty diff, record, no-materialize lane patching, sparse and materialized workdirs, merge queue, daemon hot-path calls, Git import/update, index rebuild, GC dry-run, and backup create/verify. CI also checks selected wall-time and storage ceilings with:

```sh
python3 scripts/check-cli-scale-thresholds.py <results.tsv> name=max_seconds ... --metrics <metrics.tsv> key=max_value ...
```

Structural string invariants use `--metric-equals key=value`. The Git handoff
gate combines wall-time ceilings, numeric ceilings, and exact export-mode
checks in one invocation.

The `Changed-path Ledger Native Gates` workflow is a required merge and
release gate. Branch protection and release automation must require both its
Linux/ext4 and macOS/APFS matrix jobs for the exact commit being merged or
released; scheduled artifacts are additional qualification evidence, not a
substitute for those commit-specific checks.

The workflow is reusable. `release-automation.yml` calls it before creating a
tag, and cargo-dist declares it as a generated `plan-jobs` dependency so a tag
push cannot build, host, or publish until both native jobs pass for that tag's
exact SHA. The compiled activation manifest names those dependencies, but its
self-hash is only a declaration of the build contract; it is not evidence that
a workflow ran.

Changed-path structural reports are closed schemas: missing and unknown work
counters fail the gate. Every repository-size-sensitive counter is either
required to be zero or capped by an explicit affine O(k) bound. Selected
worktree-index SQLite work additionally has a typed disposition:

- workspace status, diff, and record report `not_applicable`, one independent
  N/A proof, and zero selected-index work;
- materialized lane record, structured patch, and COW checkpoint report
  `complete`, at least one accounting envelope (including k=0), and no N/A
  claim;
- absent, mixed, or ambiguous accounting fails even when every numeric SQLite
  counter is zero.

COW checkpoint reports must also declare
`generated_path_accounting=journal_interval`. This proves generated dirty paths
come from the bounded journal interval rather than a recursive upper-directory
inventory.

## Local and Large Runs

```sh
make bench-cli-scale
make bench-cli-scale-large
make bench-cli-scale-nightly
make bench-cli-scale-1m-headless
```

Defaults:

- `bench-cli-scale`: 10,000 files under `/Volumes/Workspace`.
- `bench-cli-scale-large`: 100,000 files under `/Volumes/Workspace`.
- `bench-cli-scale-nightly`: 10,000, 100,000, and 1,000,000 files.
- `bench-cli-scale-1m-headless`: 1,000,000 files without backup or materialized-workdir cases.

Override scale and location with:

```sh
TRAIL_SCALE_FILES=10000,100000,1000000 \
TRAIL_SCALE_BASE=/Volumes/Workspace \
TRAIL_SCALE_LABEL=manual-scale \
scripts/cli-scale-bench.sh
```

Optional toggles:

- `TRAIL_SCALE_MATERIALIZED=0|1`
- `TRAIL_SCALE_BACKUP=0|1`
- `TRAIL_SCALE_DAEMON=0|1`
- `TRAIL_SCALE_GIT_IMPORT=0|1`

Run the path-index structural matrix without the unrelated large artifacts:

```sh
TRAIL_BIN=target/release/trail \
TRAIL_SCALE_FILES=1000,100000,1000000 \
TRAIL_SCALE_MATERIALIZED=0 \
TRAIL_SCALE_BACKUP=0 \
TRAIL_SCALE_DAEMON=0 \
TRAIL_SCALE_GIT_IMPORT=0 \
scripts/cli-scale-bench.sh
```

This still runs an explicitly bounded sparse record case at every scale. It
also covers a content-only patch, a case-only rename combined with delete/add,
and a patch against an empty root. The scheduled workflow applies the same
structural gates to 10k, 100k, and 1M files.

## Output

Each scale writes:

- `results.tsv`: command name, wall time, maximum RSS, and exit code.
- `metrics.tsv`: source file count, source bytes, SQLite bytes, object count, object-kind bytes, SQLite `dbstat` bytes, daemon RSS, and clean/sparse workdir manifest bytes.
- `out/*.stdout` and `out/*.stderr`: captured command output.

Important storage signals:

- `sqlite_bytes`
- `object_kind_repo_TextContent_bytes`
- `dbstat_repo_prolly_nodes`
- `manifest_repo_clean_workdir_bytes`
- `manifest_repo_sparse_workdir_bytes`

Patch and record JSON reports include an operation-scoped `path_index` object:

- `mode`: `indexed` once the operation uses bounded folded-path lookups.
- `lookup_count`: number of folded-key index lookups. This must be no greater
  than the unique folded old/new paths touched by the completed operation;
  content-only writes to existing exact paths can require zero lookups.
- `full_root_path_load_count`: unbounded traversals that enumerate every path
  in a persisted root. A fallback that loads both the previous and target roots
  counts twice.
- `full_filesystem_path_scan_count`: unbounded repository-shaped filesystem
  traversals used for path validation. A full materialized workdir validation
  counts as one, including a clean no-op. Walking an explicitly selected sparse
  materialization does not count because its size is bounded by that selection.

The counters reset at the public patch or record boundary, including failures,
retries, and no-ops. This prevents a prior operation from making a later report
look unbounded. The benchmark extractor independently folds changed paths with
the same NFKC, per-codepoint lowercase, then NFC sequence as the Rust index and
rejects malformed or internally inconsistent reports.

A full materialized lane patch normally updates a valid clean manifest from its
touched subset. If that manifest is missing or stale, the correctness fallback
loads both complete roots and reports
`full_root_path_load_count=2`. Likewise, a full materialized record without a
usable manifest compares one complete root to the disk manifest and reports
`full_root_path_load_count=1`. These are deliberately visible cold paths; the
no-materialize and explicitly sparse benchmark operations must remain zero.

Important hot-path rows:

- `status_clean`
- `status_dirty`
- `diff_dirty`
- `git_dirty_status`
- `git_dirty_diff`
- `git_dirty_record`
- `git_status_after_dirty_record`
- `agent_git_apply_dry_run`
- `agent_git_apply`
- `agent_git_apply_missing_mapping`
- `daemon_wait_for_health`
- `daemon_wait_for_hot_cache`
- `daemon_persisted_snapshot_status`
- `daemon_persisted_snapshot_record_clean`
- `daemon_persisted_snapshot_diff_dirty`
- `lane_apply_patch`
- `lane_readiness`
- `lane_merge_queue_run`
- `daemon_cli_status`
- `daemon_cli_session_start`
- `daemon_cli_approval_request`
- `daemon_cli_lease_acquire`
- `daemon_cli_lane_readiness`
- `daemon_cli_lane_trace_summary`
- `daemon_cli_timeline`
- `daemon_cli_why`
- `daemon_cli_history`
- `daemon_cli_code_from`

`git_dirty_*` rows measure the non-daemon Git dirty-path fallback for large repositories with a committed Git baseline. This fallback is useful for correctness and smaller repositories, but 1M measurements show it is not a production hot path by itself. `daemon_wait_for_health` and `daemon_wait_for_hot_cache` measure daemon startup and cache warmup, not steady-state command latency. `daemon_persisted_snapshot_*` rows hide daemon endpoint discovery while keeping the live daemon process and persisted watcher snapshot, so they verify separate Trail handles can avoid the full direct fallback without using HTTP RPC. Use `daemon_cli_*` rows for repeated agent-command hot paths.

The `agent_git_apply_*` rows exercise the high-level committed Git handoff,
not only Trail's internal lane merge. The mapped fixture creates a
no-materialize task that changes
`k = max(1, min(100, files / 1000))` paths, records the review gate, and runs
both dry-run and actual apply. Its structural metrics require
`export_mode=mapped_delta`, exactly `k` changed paths, and at most `k` blob
writes. `agent_git_plumbing_commands` is specifically the mapped-export
plumbing subprocess count: `read-tree`, the optional batch `hash-object`, one
batch `update-index`, `write-tree`, and `commit-tree`. It excludes the final
porcelain fast-forward and is gated as a constant ceiling of 5, independent of
`k`; deletion-only exports may be lower because they do not hash new blobs.
Full-snapshot exports do not use this mapped-export counter. The missing-mapping
fixture is initialized from the working tree and must return
`GIT_MAPPING_REQUIRED` without changing Git HEAD, the Git index, or Trail's
mapping table.

## Reproducible Prolly Qualification

Trail is qualified against the full prolly revision
`b68fcf4c57ce477292440da00bfc8aeed816c63a`. The configured submodule remote is
`https://github.com/crabbuild/prolly.git`; release qualification must prove the
full object remains fetchable rather than relying on an abbreviated hash or a
developer's existing submodule object database:

```sh
prolly_revision=b68fcf4c57ce477292440da00bfc8aeed816c63a
test "$(git -C prolly rev-parse HEAD)" = "$prolly_revision"
git -C prolly fetch --dry-run origin "$prolly_revision"
git diff --submodule=log -- prolly Cargo.lock
```

Run the focused root-map/storage, diff/merge, GC/fsck/backup, and recovery
compatibility gates against that checkout:

```sh
cargo test -p trail db::storage --lib -- --nocapture
cargo test -p prolly-store-sqlite -- --nocapture
cargo test -p prolly-store-slatedb -- --nocapture
cargo test -p trail db::merge --lib -- --nocapture
cargo test -p trail --test e2e init_record_why_and_fsck_work -- --exact --nocapture
cargo test -p trail --test e2e gc_prunes_unreachable_known_objects_and_preserves_reachable_roots -- --exact --nocapture
cargo test -p trail --test e2e backup_create_verify_and_restore_roundtrip -- --exact --nocapture
cargo test -p trail db::change_ledger::recovery --lib -- --nocapture
```

The candidate package is named `prolly-map`, so regenerate and verify the lock
without updating unrelated packages:

```sh
cargo update -p prolly-map
cargo metadata --locked --format-version 1 > /tmp/trail-cargo-metadata.json
```

Finally, clone the committed candidate with `git clone --no-local`, initialize
submodules recursively, and run `cargo build -p trail --locked` from that clone.
This clean-clone gate is mandatory because an uncommitted submodule checkout can
hide an unavailable gitlink or local dependency edits.

## Current Evidence

The path-index structural matrix was freshly measured on July 13, 2026 with a
release binary and headless optional toggles at
`/tmp/trail-cli-scale-task5-struct-20260713`. All 60 structural and wall/storage
gates passed (20 per scale):

| Scale | Source files | Init | Content patch | Case rename patch | Empty-root patch | Sparse record |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1k | 1,005 | 0.14s | 0.03s | 0.01s | 0.00s | 0.01s |
| 100k | 100,029 | 9.62s | 0.29s | 0.02s | 0.01s | 0.01s |
| 1M | 1,000,029 | 188.10s | 0.34s | 0.02s | 0.01s | 0.01s |

At all three scales, all four operation reports used `mode=indexed`, loaded no
full root path set, and performed no unbounded filesystem path scan. The
content patch performed 0 lookups for 5 touched folded keys at 1k and 0 for 50
at 100k/1M. The case fixture performed 1 lookup for 2 reported folded endpoints,
while empty-root patch and sparse record each performed 1 lookup for 1 touched
folded key. The 1M artifact was 1,584,734,208 SQLite bytes with 1,000,352
objects; index rebuild took 3.28s. These results are structural evidence that
patch and bounded record work scales with touched paths rather than repository
path count.

Keep legacy missing-index recovery as a separate correctness proof rather than
mixing it into the performance rows:

```sh
cargo test -p trail --test e2e \
  cli_path_index_required_human_json_rebuild_and_retry_lifecycle -- --exact
```

The latest local `/Volumes/Workspace` runs were measured on June 25, 2026 with the release binary:

| Scale | Init | Direct clean status | Direct dirty diff | Direct dirty record | Daemon status | Daemon CLI record | Lane patch | Lane readiness | Merge apply |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10k files | 1.29s | 0.06s | 0.71s | 0.60s | 0.05s | 0.21s | 0.33s | 0.02s | 0.14s |
| 100k files | 29.59s | 0.43s | 1.86s | 1.65s | 0.07s | 0.67s | 0.63s | 0.03s | 0.30s |
| 1M files | 418.39s | 24.94s | 31.16s | 30.55s | 0.11s | 0.80s | 0.78s | 0.05s | 0.44s |

The fresh 100k run at `/Volumes/Workspace/trail-cli-scale-codex-100k-git-daemon-20260625/100000` used no backup or materialized-workdir cases and passed 31 threshold checks. Its Git fallback rows measured `git_dirty_status=1.52s`, `git_dirty_diff=2.52s`, `git_dirty_record=1.86s`, and `git_status_after_dirty_record=0.90s`. Its persisted daemon snapshot rows measured `daemon_persisted_snapshot_status=0.01s`, `daemon_persisted_snapshot_record_clean=0.01s`, and `daemon_persisted_snapshot_diff_dirty=0.65s`.

The fresh 1M Git+daemon run at `/Volumes/Workspace/trail-cli-scale-codex-1m-git-daemon-20260625/1000000` used no backup or materialized-workdir cases. It produced 1,000,029 source files, 75.9 MiB source bytes, 1.46 GiB SQLite, 1,000,460 objects, 452.5 MiB `TextContent` object bytes, and 667.5 MiB `repo_prolly_nodes` bytes. It passed 31 calibrated hot-path and storage threshold checks:

```sh
python3 scripts/check-cli-scale-thresholds.py \
  /Volumes/Workspace/trail-cli-scale-codex-1m-git-daemon-20260625/1000000/results.tsv \
  daemon_wait_for_health=60 daemon_wait_for_hot_cache=120 \
  daemon_status=5 daemon_persisted_snapshot_status=5 \
  daemon_persisted_snapshot_record_clean=5 daemon_persisted_snapshot_diff_dirty=10 \
  daemon_cli_status=5 daemon_cli_record_dirty=10 \
  daemon_cli_lane_readiness=5 daemon_cli_lane_trace_summary=5 \
  daemon_cli_merge_dry_run=10 daemon_cli_session_start=10 \
  daemon_cli_approval_request=10 daemon_cli_lease_acquire=10 \
  daemon_cli_timeline=10 daemon_cli_why=10 daemon_cli_history=10 \
  daemon_cli_code_from=10 lane_apply_patch=10 lane_readiness=10 \
  merge_lane_dry_run=10 merge_lane_apply=10 lane_merge_queue_run=10 \
  git_dirty_status=120 git_dirty_diff=120 git_dirty_record=120 \
  git_status_after_dirty_record=90 \
  --metrics /Volumes/Workspace/trail-cli-scale-codex-1m-git-daemon-20260625/1000000/metrics.tsv \
  sqlite_bytes=4000000000 dbstat_repo_prolly_nodes=2500000000 \
  object_kind_repo_TextContent_bytes=1200000000 object_count=2000000
```

The 1M artifact contains one obsolete failed `daemon_cli_doctor` row from before that auxiliary probe was removed from the harness. The rows after it were completed with a daemon continuation run and metrics were generated afterward. The measured rows are the important signal: direct cold `status`/`diff --dirty`/`record` were 24.94s, 31.16s, and 30.55s; Git fallback `status`/`diff --dirty`/`record` were 89.03s, 87.42s, and 88.84s; daemon `status`, persisted-snapshot `status`, persisted-snapshot clean `record`, persisted-snapshot dirty `diff --dirty`, and daemon CLI dirty `record` were 0.11s, 0.01s, 0.00s, 1.67s, and 0.80s. This validates the daemon snapshot path as the production large-repo loop and the Git path as a correctness fallback rather than a 1M hot path.

The compact prolly-node slice was measured on June 25, 2026 after switching newly written nodes to the `CRAB` binary encoding and widening the root-map fanout to min/max/chunking-factor `64/512/256`. The standalone SQLite prolly bench was run with `PROLLY_SQLITE_SCALE_STAGES=1000,100000 cargo bench -p prolly-map --features sqlite --bench sqlite_scale_bench` because SQLite support remains an optional `sqlite` feature on the `prolly-map` package:

| Target records | Total records | DB bytes | Bytes/record | Stage time | Reopen verified |
| ---: | ---: | ---: | ---: | ---: | :---: |
| 1,000 | 1,000 | 98,696 | 98.70 | 0.699ms | yes |
| 100,000 | 100,000 | 3,897,336 | 38.97 | 52.729ms | yes |

The end-to-end CLI smoke was also run at 1k and 100k files with no backup or materialized-workdir cases:

| Scale | Source bytes | SQLite bytes | `repo_prolly_nodes` | `TextContent` bytes | Init | Direct clean status | Direct dirty diff | Direct dirty record | Daemon status | Daemon CLI record | Lane patch | Lane readiness | Merge apply |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1k files | 73,759 | 10,895,360 | 8,916,992 | 460,369 | 0.31s | 0.02s | 0.05s | 0.05s | 0.05s | 0.01s | 0.02s | 0.00s | 0.01s |
| 100k files | 7,496,402 | 279,113,728 | 191,139,840 | 45,043,050 | 11.23s | 0.49s | 1.42s | 1.36s | 0.05s | 0.20s | 0.41s | 0.03s | 0.24s |

## Production Recommendation

Large agent orchestration should use the daemon and no-materialize/sparse workdirs by default. The production hot path is:

- Spawn agents without full materialization unless a task explicitly needs filesystem access.
- Prefer structured patches, MCP/API file reads, readiness, merge preflight, and merge queue operations over full worktree scans.
- If filesystem access is needed, materialize selected paths with `--paths` and hydrate more paths lazily through `lane read`.
- Keep the daemon running for repeated CLI calls so status, record, trace, gate, handoff, and merge queue commands use a hot SQLite connection and watcher-backed dirty path cache.
- Separate Trail handles can reuse the daemon's watcher-backed dirty snapshot only while the persisted snapshot is initialized, belongs to the same workspace, and the daemon PID is still alive. If the snapshot is missing, stale, overflowed, or too large, commands fall back to Git dirty paths when a committed Git baseline is available, then to the full persisted-index scan.
- Before high-level `trail agent apply`, reconcile the current Git HEAD with
  Trail using `trail git import-update` when `GIT_MAPPING_REQUIRED` is
  reported. A mapped apply builds from the trusted HEAD tree and touches only
  changed paths. Missing trust is intentionally a fast error; Trail does not
  silently scan and hash the full repository to manufacture a mapping.

## Known Limits

Direct non-daemon `status`, `diff --dirty`, and `record` still fall back to a full filesystem walk plus root/index comparison when there is no valid live daemon snapshot. At 1M files those direct commands measured roughly 25-33s. The Git dirty-path fallback is correct and useful for smaller committed repositories, but the fresh 1M fixture measured roughly 56-89s for Git fallback status/diff/record rows because Git itself had to inspect a 1M-file index. Both paths should be treated as fallbacks for large repos, not the production agent loop.

A read-only component probe on the saved 1M artifact also showed why direct cold status cannot become a millisecond path without a trusted change source: walking and stating 1,000,029 visible files took 54.20s in a Python probe, while loading all `worktree_file_index` rows took 2.84s and 100k indexed point lookups took 0.38s. The dominant cost is discovering whether the filesystem changed, not fetching indexed metadata. The daemon's watcher-backed persisted snapshot is the production mechanism that avoids that discovery scan.

The next storage target is reducing prolly-node and `TextContent` amplification further. `SmallTextTable` is active for small text, and compact `CRAB` prolly nodes plus wider root-map fanout are active for newly written nodes, but the measured 100k run still shows 191.1 MiB of `repo_prolly_nodes` for 7.5 MiB of source bytes. Zstd for cold nodes and global path-component interning remain deferred until the compact encoding slice is compared against larger saved-repo baselines.

## Measured Non-Solutions

Do not try to fix 1M direct status by replacing each `worktree_file_index` point lookup with bounded `WHERE path IN (...)` batches. A 512-path bounded-batch experiment still had to stat every file and measured 49.93s with about 633 MiB RSS on the saved 1M repo, worse than the prior direct status baseline. The direct-path fix needs a watcher-backed persisted changed-path generation, Git-index integration, or another design that avoids walking/stating every file when the tree is clean.

Do not replace the point lookup loop with an eager sorted filesystem snapshot plus sorted index merge unless it also avoids holding both sides in memory. A release-build experiment on the saved 1M repo collected all scanned paths, parallel-statted them, loaded the stored index, sorted/merged both sides, and then ran direct `status`. It measured 38.31s with about 645 MiB max RSS and reported 75 dirty paths, also worse than the prior direct fallback baseline.

## Code Facts Used

- Benchmark harness: `scripts/cli-scale-bench.sh`
- Threshold checker: `scripts/check-cli-scale-thresholds.py`
- Make targets: `Makefile`
- CI workflow: `.github/workflows/ci.yml`
- Nightly/manual workflow: `.github/workflows/scale.yml`
