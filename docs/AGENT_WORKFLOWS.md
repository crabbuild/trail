# CrabDB Agent Workflows

CrabDB agents should default to structured, no-materialize workflows for large repositories. Use patches, MCP/API reads, readiness checks, merge preflight, and the merge queue before creating filesystem workdirs.

Agent workflow references:

- [Agent overview](agents/overview.md)
- [Spawn and materialize workdirs](agents/spawn-and-materialize-workdirs.md)
- [Structured patches](agents/structured-patches.md)
- [Sessions, turns, messages, and runs](agents/sessions-turns-messages-and-runs.md)
- [Events, traces, and spans](agents/events-traces-and-spans.md)
- [Tests, evals, gates, and readiness](agents/tests-evals-gates-and-readiness.md)
- [Handoff, review, and merge](agents/handoff-review-and-merge.md)

For daemon-backed automation patterns, see [Daemon-backed automation](use-cases/daemon-backed-automation.md) and [Parallel agent work](use-cases/parallel-agent-work.md).
