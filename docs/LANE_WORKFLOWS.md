# Trail Lane Workflows

Trail lanes should default to structured, no-materialize workflows for large repositories. Use patches, MCP/API reads, readiness checks, merge preflight, and the merge queue before creating filesystem workdirs.

Lane workflow references:

- [Lane overview](lanes/overview.md)
- [Lane work model](lanes/work-model.md)
- [Spawn and materialize workdirs](lanes/spawn-and-materialize-workdirs.md)
- [Structured patches](lanes/structured-patches.md)
- [Sessions, turns, messages, and runs](lanes/sessions-turns-messages-and-runs.md)
- [Events, traces, and spans](lanes/events-traces-and-spans.md)
- [Tests, evals, gates, and readiness](lanes/tests-evals-gates-and-readiness.md)
- [Handoff, review, and merge](lanes/handoff-review-and-merge.md)
- [Harden agent workflows](guides/hardening-agent-workflows.md)

For daemon-backed automation patterns, see [Daemon-backed automation](use-cases/daemon-backed-automation.md) and [Parallel lane work](use-cases/parallel-lane-work.md).
