# Tests, Evals, Gates, and Readiness

Test and eval gates are durable records attached to a lane branch.

## Run a Test Gate

```sh
crabdb lane test doc-bot \
  --suite unit \
  --timeout-secs 600 \
  -- cargo test -p crabdb
```

## Run an Eval Gate

```sh
crabdb lane eval doc-bot \
  --suite policy-smoke \
  --score 1.0 \
  --threshold 1.0 \
  -- cargo test -p crabdb
```

Everything after `--` is the command run in the lane workdir.

## Inspect Gates

```sh
crabdb lane gates doc-bot --kind test --limit 20
crabdb lane gates doc-bot --kind eval --limit 20
```

Gate records include status, suite, score, threshold, exit code, duration, output object IDs, and output previews.

## Require Gates Before Merge

```sh
crabdb config set lane.require_test_gate true
crabdb config set lane.required_test_suites unit
crabdb config set lane.require_eval_gate true
crabdb config set lane.required_eval_suites policy-smoke
```

Readiness blocks merge until required gates are present and passing.

## Code Facts Used

- Test/eval args: `crates/crabdb/src/cli/command/lane_args.rs`
- Gate reports: `crates/crabdb/src/model/reports/lane.rs`
- Gate runner: `crates/crabdb/src/db/lane/gates`
- Tests: `lane_test_runs_in_workdir_and_records_events_and_output_blobs`, `required_gate_config_blocks_merge_until_test_and_eval_pass`
