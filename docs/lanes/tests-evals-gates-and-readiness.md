# Tests, Evals, Gates, and Readiness

Test and eval gates are durable records attached to a lane branch.

## Run a Test Gate

```sh
trail lane test doc-bot \
  --suite unit \
  --timeout-secs 600 \
  -- cargo test -p trail
```

## Run an Eval Gate

```sh
trail lane eval doc-bot \
  --suite policy-smoke \
  --score 1.0 \
  --threshold 1.0 \
  -- cargo test -p trail
```

Everything after `--` is the command run in the lane workdir.

## Inspect Gates

```sh
trail lane gates doc-bot --kind test --limit 20
trail lane gates doc-bot --kind eval --limit 20
```

Gate records include status, suite, score, threshold, exit code, duration, output object IDs, and output previews.

## Require Gates Before Merge

```sh
trail config set lane.require_test_gate true
trail config set lane.required_test_suites unit
trail config set lane.require_eval_gate true
trail config set lane.required_eval_suites policy-smoke
```

Readiness blocks merge until required gates are present and passing.

## Code Facts Used

- Test/eval args: `trail/src/cli/command/lane_args.rs`
- Gate reports: `trail/src/model/reports/lane.rs`
- Gate runner: `trail/src/db/lane/gates`
- Tests: `lane_test_runs_in_workdir_and_records_events_and_output_blobs`, `required_gate_config_blocks_merge_until_test_and_eval_pass`
