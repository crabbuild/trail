# Trail Roadmap

Trail is a local-first operation database for code and text worktrees. Its
long-term direction is to become the local coordination and provenance layer
that sits between day-to-day work and Git publication.

Git should remain the shared source-control system. Trail should own the local,
high-frequency, pre-commit layer: recorded operations, agent attempts,
line-level provenance, readiness checks, review handoffs, merge safety, and
machine-readable state for editors, agents, daemons, and automation.

This roadmap is directional. It describes the product strategy, sequencing, and
quality gates for Trail as it evolves from a powerful local tool into a durable
platform for human and agent collaboration.

## Product North Star

Trail should make local and agent-driven code work understandable, recoverable,
reviewable, and safe to merge.

The core promise is:

- Humans can ask what happened, why it happened, who or what did it, and whether
  it is safe to accept.
- Agents can work in isolated branches, produce structured changes, report
  evidence, request approval, and hand off work without losing context.
- Tools can query the same truth through CLI, HTTP, MCP, and Rust APIs.
- Git receives reviewed, intentional history rather than raw intermediate noise.

## Current Foundation

The current codebase already establishes the main architectural direction:

- Local `.trail/` workspace state.
- Operation history with content-addressed objects, refs, roots, and rebuildable
  indexes.
- Stable `ChangeId`, `FileId`, and `LineId` identity for provenance and
  line-aware patching.
- CLI workflows for init, status, record, watch, timeline, show, diff, checkout,
  branch, merge, why, history, and code-from.
- Agent branches under `refs/agents/<name>`.
- Agent sessions, turns, messages, trace events, spans, approvals, leases,
  tests, evals, gates, readiness reports, handoff reports, and merge queues.
- Structured patch application and agent workdir recording.
- Git import/export mappings.
- Guardrails for ignored paths, risky actions, approvals, and workspace policy.
- Maintenance commands for doctor, backup, restore, fsck, index rebuild, and GC.
- Shared report types across CLI JSON, HTTP API, MCP, and Rust library surfaces.
- A daemon hot path that is already the practical large-repo loop for repeated
  status, record, agent readiness, trace, handoff, and merge operations.

This foundation should be treated as the product center. Future work should
stabilize, simplify, and extend these capabilities instead of replacing them
with a separate model.

## Strategic Principles

1. Git adjacency over Git replacement.
   Trail should complement Git, not compete with it. Git remains the
   publication, synchronization, and ecosystem layer.

2. Local-first by default.
   Core workflows must work without a hosted service, network connection, or
   always-on daemon. The daemon should improve performance and integration, not
   become a hard requirement for basic use.

3. Operations before snapshots.
   Trail should preserve the meaningful action that produced a state, not only
   the final tree. Recorded operations, messages, tests, gates, and approvals
   are part of the product.

4. Agent-native without being agent-only.
   Human workflows should stay first-class. Agent features should build on the
   same branch, provenance, guardrail, and merge model that humans can inspect.

5. Typed interfaces over string protocols.
   CLI JSON, HTTP, MCP, and Rust should share report shapes and semantics. New
   features should enter the core model before being exposed through surfaces.

6. Safety is a product feature.
   Dirty workdir checks, conflict sets, approval records, guardrails, leases,
   gates, and readiness reports are not optional polish. They are how Trail
   earns trust.

7. Scale paths must be explicit.
   Large repositories should use daemon-backed dirty state, sparse or
   no-materialize agent branches, structured patches, and merge queues. Direct
   full-tree scans should remain correctness fallbacks.

## Roadmap Horizons

### Horizon 1: Stabilize the Local Developer Core

Goal: make Trail dependable as a daily local operation database for a single
developer working in normal Git repositories.

Primary outcomes:

- Installation and initialization feel predictable on new and existing
  repositories.
- Recording, inspecting, and checking out local operations are reliable enough
  for routine work.
- Users can understand line provenance and operation history without reading
  internal data structures.
- Maintenance and recovery commands are trustworthy.
- Documentation covers the common path before advanced internals.

Key work:

- Harden `init`, `status`, `record`, `watch`, `timeline`, `show`, `why`,
  `history`, `checkout`, `branch`, and `merge` workflows.
- Make human output concise, stable, and script-friendly.
- Keep JSON output backward-compatible within pre-1.0 constraints or document
  intentional breaking changes.
- Improve error messages for workspace discovery, ignored files, dirty
  worktrees, locked databases, schema mismatches, and corrupt objects.
- Expand fsck and doctor diagnostics to point to specific repair actions.
- Verify backup, restore, index rebuild, and GC on realistic repositories.
- Build a small set of golden workflows for documentation and regression tests.

Exit criteria:

- A user can initialize a Git repository, record changes, inspect provenance,
  restore from backup, and export reviewed work to Git using only documented
  commands.
- Common failures explain the cause and next action.
- Smoke tests cover the basic local lifecycle end to end.

### Horizon 2: Make Agent Workflows Production-Ready

Goal: make Trail the reliable local coordination layer for coding agents.

Primary outcomes:

- Each agent can work safely on an isolated ref.
- Humans can review what an agent did, why it did it, what evidence it produced,
  and what still blocks merge.
- Merge queues serialize accepted agent work.
- Guardrails and approvals make risky actions explicit.

Key work:

- Stabilize agent branch lifecycle: spawn, status, read, patch, record,
  sync-workdir, checkout, contribution, readiness, handoff, merge, and removal.
- Refine structured patch semantics around stable file and line identity.
- Improve conflict reporting so failed agent merges produce actionable conflict
  sets with stable anchors.
- Make readiness reports the canonical pre-merge signal for agents.
- Expand gate modeling for tests, evals, lint, security checks, human review,
  and external policy checks.
- Make approval records auditable and easy to connect to guardrail decisions.
- Strengthen lease behavior for multi-agent path coordination.
- Preserve enough session, turn, message, event, and span context for review
  without storing excessive private or generated data.
- Add examples for no-materialize, sparse materialization, and structured-patch
  agent flows.

Exit criteria:

- A maintainer can run multiple agents, inspect each contribution, require
  gates, approve risky actions, and merge accepted work through a queue.
- Agent handoff reports contain enough context for a human or another agent to
  continue the work.
- Failed merges leave durable, inspectable conflict information.

### Horizon 3: Promote the Daemon and Integration Surfaces

Goal: make Trail useful to editors, agent hosts, local services, and automation
without forcing every integration to shell out.

Primary outcomes:

- CLI, HTTP, MCP, and Rust APIs expose the same conceptual model.
- The daemon provides the fast large-repo loop through warmed state and
  watcher-backed dirty snapshots.
- MCP tools make Trail directly useful inside agent hosts.
- OpenAPI and Rust types are good enough for external integrations.

Key work:

- Expand daemon coverage for hot commands while preserving local fallbacks.
- Define daemon lifecycle behavior: discovery, authentication, health,
  shutdown, stale PID cleanup, and snapshot invalidation.
- Keep OpenAPI output aligned with actual route behavior and report types.
- Improve MCP tools, resources, prompts, and completions around common agent
  tasks: inspect, patch, gate, handoff, merge, and conflict resolution.
- Add integration examples for editor extensions, local automation, and agent
  hosts.
- Establish compatibility expectations for report schemas and API routes.
- Add traceability from API/MCP actions back to operation, session, and message
  records.

Exit criteria:

- A non-Rust tool can use Trail through documented HTTP or MCP workflows.
- Repeated large-repo status, diff, record, readiness, trace, and merge
  operations use daemon-backed fast paths.
- CLI JSON, HTTP, MCP, and Rust reports remain semantically aligned.

### Horizon 4: Improve Scale, Storage, and Performance

Goal: keep Trail viable for large monorepos and high-volume agent workflows.

Primary outcomes:

- Large repositories rely on the daemon, sparse workdirs, structured patches,
  and indexed reads.
- Cold fallback paths remain correct and understandable.
- Storage amplification is measured and reduced over time.
- Benchmarks become part of release readiness.

Key work:

- Continue calibrating 10k, 100k, and 1M-file benchmark suites.
- Treat daemon-backed dirty state as the production large-repo path.
- Reduce prolly-node and text-content amplification through compact encodings,
  path-component interning, compression for cold nodes, and improved small-text
  representations where measurements justify them.
- Improve watcher overflow handling and persisted snapshot invalidation.
- Avoid full filesystem walks when a trusted changed-path source exists.
- Keep sparse and no-materialize agent workdirs as the default recommendation.
- Track memory ceilings, SQLite size, object counts, and hot command latency.
- Add benchmark summaries to release notes.

Exit criteria:

- The documented benchmark suite runs in CI or scheduled automation at useful
  scales.
- Large-repo guidance is explicit and backed by measurements.
- Storage and latency regressions are caught before release.

### Horizon 5: Deepen Provenance and Review Semantics

Goal: make Trail the best local source of truth for why code exists.

Primary outcomes:

- Provenance queries are useful to humans, not just technically correct.
- Reviewers can move from a current line to its operation, session, agent,
  message, tests, approvals, and merge path.
- Anchors and line identity survive common edits, moves, renames, and merges.

Key work:

- Improve `why`, `history`, `code-from`, timeline, contribution, and handoff
  outputs around reviewer questions.
- Add richer change summaries across operations, sessions, agents, and merge
  queues.
- Strengthen anchor behavior for comments, review notes, conflict resolution,
  and external references.
- Explore semantic grouping of related operations into tasks, attempts,
  reviews, and accepted contributions.
- Improve rename, move, and line-rewrite tracking where current heuristics are
  weak.
- Add provenance views that connect Git commits back to Trail operations after
  export or import.

Exit criteria:

- A reviewer can answer "why is this line here?" and "what evidence supports
  this change?" from documented Trail commands or APIs.
- Handoff and contribution reports become practical review artifacts.

### Horizon 6: Team and Distributed Collaboration

Goal: explore how Trail can support teams without sacrificing the local-first
model.

Primary outcomes:

- Local Trail state can be selectively shared, exported, synchronized, or
  attached to review systems.
- Teams can preserve agent context and provenance across machines when useful.
- Shared workflows remain compatible with Git and existing code review systems.

Potential directions:

- Portable operation bundles for sharing agent attempts, handoffs, conflict
  sets, or review evidence.
- Signed or verifiable operation records for audit-sensitive workflows.
- Optional team service for indexing, policy, review artifacts, and agent
  coordination across repositories.
- Pull-request attachments that summarize Trail provenance, gates, approvals,
  and agent contributions.
- Cross-machine agent handoff where a receiving workspace can inspect and
  materialize a contribution safely.
- Organization policy packs for guardrails, approval rules, ignore defaults,
  and required gates.

Exit criteria:

- Team features are optional and layered on top of local truth.
- Sharing never requires exposing private ignored files, generated artifacts, or
  sensitive messages by default.

## Release Tracks

### Pre-1.0 Alpha

Purpose: prove the model and workflows.

Focus:

- Local recording and provenance.
- Agent branch lifecycle.
- Structured patches.
- Readiness and merge queue.
- Daemon hot paths.
- Documentation that matches the current code.

Quality bar:

- Breaking changes are acceptable when they simplify the model.
- Data migrations can be limited, but schema/version errors must be clear.
- Benchmarks and e2e smoke tests should protect core workflows.

### Pre-1.0 Beta

Purpose: stabilize external behavior.

Focus:

- Clear command UX.
- Stable report shapes for common workflows.
- More complete HTTP and MCP coverage.
- Recovery tooling.
- Editor and agent-host examples.

Quality bar:

- Backward compatibility expectations are documented.
- Data migrations are tested.
- Reports and errors are stable enough for integrations.
- Large-repo guidance is measured and repeatable.

### 1.0

Purpose: declare Trail dependable for daily local and agent-assisted work.

Focus:

- Stable workspace format or documented migration policy.
- Stable core CLI workflows.
- Stable Rust model/report types for supported APIs.
- Documented HTTP and MCP compatibility expectations.
- Strong backup, restore, fsck, doctor, and index rebuild stories.
- Documented security and privacy posture.

Quality bar:

- Users can trust Trail with important local work.
- Integrators can build against supported surfaces.
- Known limitations are explicit.

### Post-1.0

Purpose: expand from local tool to ecosystem substrate.

Focus:

- Team sharing and portable operation bundles.
- Policy packs and organization workflows.
- Deeper editor integrations.
- Hosted or peer synchronization options.
- Rich review artifacts tied to Git and code review systems.
- Advanced provenance, semantic change grouping, and audit features.

## Functional Tracks

### Workspace and Storage

Direction:

- Keep durable truth in content-addressed objects and refs.
- Treat indexes as rebuildable accelerators.
- Make schema changes explicit and recoverable.
- Continue improving prolly-map efficiency and text storage.

Important questions:

- Which object and report formats need compatibility guarantees?
- What is the smallest migration story that users can trust?
- Which storage optimizations materially improve real repositories?

### Provenance and Identity

Direction:

- Make stable file and line identity the core differentiator.
- Preserve useful continuity across edits, moves, renames, and merges.
- Connect local operations to Git commits when work is published.

Important questions:

- How much line-history detail is useful before output becomes noisy?
- How should Trail present uncertain provenance heuristics?
- What review workflows need durable anchors?

### Agent Coordination

Direction:

- Make agent branches, sessions, turns, traces, gates, approvals, and handoffs
  first-class product concepts.
- Prefer structured patches and sparse materialization.
- Make readiness the standard pre-merge interface.

Important questions:

- Which gate types should be built in?
- How should external test, eval, and policy systems attach evidence?
- What should be recorded from agent activity by default?

### Safety and Policy

Direction:

- Keep ignore rules, guardrails, approvals, leases, dirty checks, and conflict
  sets close to every mutating workflow.
- Make blocked states actionable.
- Avoid surprising capture of private, ignored, generated, or external data.

Important questions:

- Which guardrail defaults should be strict?
- How should policy be configured per workspace, team, or organization?
- How should Trail redact or omit sensitive agent messages and tool payloads?

### Interfaces and Ecosystem

Direction:

- Preserve one conceptual model across CLI, HTTP, MCP, and Rust.
- Use CLI for humans and scripts.
- Use daemon and HTTP for editor and service loops.
- Use MCP for agent-host integration.
- Use Rust for embedded integrations.

Important questions:

- Which commands deserve daemon parity first?
- Which MCP resources and prompts create the most leverage for agents?
- What compatibility policy should apply to JSON reports before and after 1.0?

### Developer Experience and Documentation

Direction:

- Keep docs generated or verified against real command behavior where possible.
- Explain workflows before internals.
- Maintain advanced design docs for contributors and integrators.

Important questions:

- What is the shortest successful first-run path?
- Which examples should be executable in CI?
- How should docs mark experimental versus stable features?

## Evolution Opportunities

Trail can evolve in several coherent directions. These are not separate
products; they are layers that can grow from the current architecture.

### Local Operational Memory for Development

Trail can become the durable memory of local software work: every meaningful
operation, branch attempt, explanation, test result, approval, and handoff can
be traced without polluting Git history.

This is the most important direction because it benefits both human and agent
workflows.

### Agent Control Plane

Trail can become a local control plane for coding agents: spawn, isolate,
claim, patch, test, approve, review, handoff, and merge. The product should make
agent work observable and governable rather than magical.

### Provenance Layer for Code Review

Trail can produce review artifacts that explain not only what changed, but
which operation or agent introduced it, what evidence exists, what risks were
identified, and what approvals were granted.

### Editor and IDE Substrate

Trail can power editor features such as line provenance, local operation
timeline, agent branch inspection, conflict anchors, safe patch application,
and readiness status directly inside the developer's working environment.

### Local Automation Database

Trail can become a structured local database for automation around code: test
runs, eval results, generated review notes, policy decisions, merge queues, and
maintenance reports can share one data model.

### Team Review and Audit Fabric

Longer term, Trail can support optional sharing of operation bundles, signed
agent contributions, review evidence, and policy results. This should remain
opt-in and privacy-preserving.

## Prioritization Rubric

Prefer work that:

- Increases user trust in recorded local history.
- Makes agent output easier to review, gate, and merge.
- Reuses the existing core model across multiple surfaces.
- Improves correctness or recovery for data already stored in `.trail/`.
- Has measurable impact on large-repo or high-volume agent workflows.
- Reduces conceptual complexity for users or integrators.

Defer work that:

- Replaces Git concepts that already work well.
- Adds hosted assumptions to core local workflows.
- Creates a second model for HTTP, MCP, or editor integrations.
- Optimizes unmeasured bottlenecks.
- Records more sensitive data without a clear policy and redaction story.
- Makes agent workflows harder for humans to inspect.

## Non-Goals

Trail should not become:

- A Git replacement.
- A hosted-only collaboration service.
- A general-purpose database unrelated to code and text worktrees.
- A black-box agent runner that hides evidence from humans.
- A tool that records ignored, private, generated, or external data by surprise.
- A merge system that bypasses review, readiness, or policy checks.

## Success Metrics

Product metrics:

- Time from install to first recorded operation.
- Percentage of documented workflows covered by automated smoke tests.
- Number of integrations using CLI JSON, HTTP, MCP, or Rust reports.
- Successful agent contributions reviewed and merged through Trail workflows.
- Recovery success rate for backup, restore, fsck, and index rebuild scenarios.

Technical metrics:

- Clean and dirty status latency at 10k, 100k, and 1M-file scales.
- Dirty record latency with and without daemon support.
- Agent readiness, handoff, merge dry-run, and merge queue latency.
- SQLite size, object count, prolly-node bytes, and text-content bytes.
- Daemon memory, watcher overflow rate, and snapshot invalidation rate.
- Test, eval, and gate execution reliability.

Trust metrics:

- Number of workflows with clear readiness blockers.
- Number of risky actions requiring explicit approval.
- Percentage of failed merges that leave actionable conflict sets.
- User ability to answer provenance questions from documented commands.

## Suggested Near-Term Sequence

1. Stabilize the first-run local workflow: install, init, record, status,
   timeline, why, history, backup, and restore.
2. Tighten agent readiness and merge flows so every blocked state is explicit.
3. Expand daemon-backed hot paths for commands used repeatedly by agents and
   editors.
4. Improve documentation examples and keep them aligned with tests.
5. Reduce storage amplification based on benchmark evidence.
6. Improve MCP and HTTP coverage for agent-host and editor integration.
7. Define compatibility and migration policy for the path toward 1.0.

## Long-Term Vision

Trail should become the standard local substrate for trustworthy human and
agent collaboration on code.

In that future:

- Git stores accepted shared history.
- Trail stores the local operational story behind that history.
- Agents work through explicit branches, patches, gates, approvals, and
  handoffs.
- Humans can inspect and govern every important step.
- Editors and agent hosts share one local truth instead of inventing separate
  state stores.
- Teams can optionally exchange provenance and review evidence without giving
  up local-first control.

The best version of Trail does not make development feel heavier. It makes the
messy space before a commit visible, structured, and safe enough to trust.
