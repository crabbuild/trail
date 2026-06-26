# Tests, Evals, Gates, and Readiness

Test and eval gates are durable records attached to an agent branch.

## Run a Test Gate

```sh
crabdb agent test doc-bot \
  --suite unit \
  --timeout-secs 600 \
  -- cargo test -p crabdb
```

## Run an Eval Gate

```sh
crabdb agent eval doc-bot \
  --suite policy-smoke \
  --score 1.0 \
  --threshold 1.0 \
  -- cargo test -p crabdb
```

Everything after `--` is the command run in the agent workdir.

## Inspect Gates

```sh
crabdb agent gates doc-bot --kind test --limit 20
crabdb agent gates doc-bot --kind eval --limit 20
```

Gate records include status, suite, score, threshold, exit code, duration, output object IDs, and output previews.

## Require Gates Before Merge

```sh
crabdb config set agent.require_test_gate true
crabdb config set agent.required_test_suites unit
crabdb config set agent.require_eval_gate true
crabdb config set agent.required_eval_suites policy-smoke
```

Readiness blocks merge until required gates are present and passing.

## Code Facts Used

- Test/eval args: `crates/crabdb/src/cli/command/agent_args.rs`
- Gate reports: `crates/crabdb/src/model/reports/agent.rs`
- Gate runner: `crates/crabdb/src/db/agent/gates`
- Tests: `agent_test_runs_in_workdir_and_records_events_and_output_blobs`, `required_gate_config_blocks_merge_until_test_and_eval_pass`

