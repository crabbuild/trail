# Concurrent Agents and Shared Environments

Use one idle seed lane to prepare reusable environment artifacts, then fork task lanes from it. Forks inherit only verified reusable outputs; Trail allocates fresh private uppers and runtime resources for every child.

## Prepare a Seed Lane

Run from the original Trail workspace:

```sh
trail lane spawn env-seed --from main
trail env discover env-seed
trail env plan env-seed
```

Discovery and planning are read-only. If the report marks components `resolvable`, resolve them explicitly, then synchronize while the seed lane is unmounted:

```sh
trail env resolve all env-seed
trail env sync all env-seed
trail env generation env-seed
```

Do not run a resolver merely because a component exists. Follow the exact recovery action in the structured report, and do not use `--refresh` unless new external resolution is intended. A repository with no detected environment component can skip synchronization.

Keep `env-seed` free of task source edits. The default layered mode is required for managed environment projections; if the owning platform backend is unavailable, stop on Trail's remediation instead of silently substituting a shared writable directory.

## Fork One Lane per Task

```sh
trail lane spawn agent-api --from env-seed --provider codex
trail lane spawn agent-docs --from env-seed --provider claude-code
trail lane claim agent-api src/api --ttl-secs 1800
trail lane claim agent-docs docs --ttl-secs 1800
```

Give each agent the workdir returned by `trail lane workdir <lane>`, or launch it through an authorized Trail agent workflow. Claims are advisory unless repository policy sets `lane.claim_enforcement` to `warn` or `reject`; design scopes not to overlap even when enforcement is advisory.

For coordinator-run commands, use managed execution so Trail attaches the lane's exact environment generation:

```sh
trail lane exec agent-api -- cargo check
trail lane exec agent-docs -- make docs
```

Identical immutable dependencies, toolchains, content caches, and compatible seeds may be reused. Mutable source changes, compiler targets, dependency uppers, virtual environments, generated output, secrets, ports, and services remain lane-private. Never replace this model with symlinks to one writable build or dependency directory.

## Monitor Without Taking Over

```sh
trail lane status agent-api
trail lane status agent-docs
trail lane review agent-api
trail lane handoff agent-docs
```

Use messages for durable coordination notes when needed:

```sh
trail lane message agent-api --role assistant --text "API schema is stable; docs lane may consume it"
```

If the target branch advanced, preview first:

```sh
trail lane refresh-preview agent-api --target main
```

Update or resolve conflicts only after preserving dirty work and reviewing the preview.

## Integrate Completed Lanes

For each lane, require a clean recorded state, review evidence, relevant gates, and readiness:

```sh
trail lane diff agent-api --patch
trail lane readiness agent-api
trail lane merge agent-api --into main --dry-run
```

For a shared target, queue ready lanes and let Trail serialize acceptance:

```sh
trail lane merge-queue add agent-api --into main
trail lane merge-queue add agent-docs --into main
trail lane merge-queue run
```

Queue execution and lane removal are consequential. Run them only with explicit integration authority. Keep failed or blocked lanes available for review and handoff.
