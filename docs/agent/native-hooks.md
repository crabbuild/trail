# Native Agent Hooks, Evidence, and Recovery

Trail can capture Codex, Claude Code, Pi, OpenCode, Cursor, Gemini CLI,
GitHub Copilot CLI, Grok Build, and Kiro without requiring ACP. Native hooks and ACP
are transports over the same versioned lifecycle and evidence model, so users
may use either one or both.

## Install an integration

Initialize Trail in the repository, preview the provider mutation, then apply
it:

```sh
trail agent hooks setup codex --print
trail agent hooks setup codex --yes
trail agent hooks status codex
trail agent hooks doctor codex
```

Replace `codex` with `claude-code`, `pi`, `opencode`, `cursor`, `gemini`,
`copilot`, `grok`, or `kiro`. Project scope is the default. User scope writes only to
the provider's declared user location and records the same ownership manifest:

```sh
trail agent hooks setup claude-code --scope user --yes
```

Kiro's automatic target is the current versioned standalone hook contract
(`.kiro/hooks/*.json`, used by the IDE and CLI v3 engine). Kiro CLI package
2.8.0 and newer include the v3 engine, even though `kiro-cli --version` still
reports a 2.x package version; launch it with `kiro-cli --v3` to activate the
standalone hooks. The default v2 engine continues to use hooks embedded in a
selected named custom-agent file and does not read Trail's standalone file.

Trail structurally merges JSON provider configuration, preserves unrelated
fields and hooks, writes generated plugin/extension files atomically, uses
restrictive permissions, and records exact before/after and ownership digests.
Removal refuses drift it cannot prove it owns:

```sh
trail agent hooks remove codex --dry-run
trail agent hooks remove codex
```

Review a conflict before using `--force`; force does not authorize Trail to
delete unrelated provider configuration.

## What capture creates

The provider callback first writes a bounded, centrally redacted receipt to
Trail's durable object journal. Semantic replay then correlates or creates one
native-session mapping and records:

- a Trail lane session and user-level turns;
- normalized session, turn, message, tool, approval, subagent, compaction,
  usage, workspace, and diagnostic events;
- nested root, tool, subagent, and compaction spans;
- at most one workdir checkpoint for a completed changed turn;
- a full canonical export when supplied, otherwise a native transcript
  snapshot, otherwise an explicitly low-fidelity reconstructed transcript;
- an immutable exact turn-evidence manifest;
- deterministic factual and derived provenance;
- a chained content-addressed session attestation on finalization.

Attachments and transcripts are content-addressed and separately redactable.
Their removal or retention state does not rewrite factual receipt, change, or
manifest identities.

## ACP and hybrid capture

ACP keeps its streamed messages, structured diffs, permission mirroring, usage,
and tool fidelity. It also emits the common lifecycle envelope and creates the
same evidence manifests and attestations.

When a native session ID exactly matches an active ACP or upstream session,
Trail creates a hybrid mapping to the existing Trail session. ACP remains the
lifecycle owner while native hooks enrich it with receipts and transcript
evidence. Exact role/body message deduplication and stable event IDs prevent a
second prompt, turn, span, or checkpoint. Ambiguous ACP matches fail closed and
are reported rather than guessed.

## Managed runs

A managed run attributes only sessions created while its renewable lease is
active. Matching uses the longest canonical workdir and exact owner or executor
provider; equal-specificity ambiguity fails closed.

```sh
trail agent capture begin \
  --owner codex --session orchestrator-42 \
  --executor claude-code --lane feature-a \
  --workdir "$PWD" --work-item issue-123

trail agent capture renew capture_run_ID \
  --owner codex --session orchestrator-42

trail agent capture end capture_run_ID \
  --owner codex --session orchestrator-42
```

Existing direct sessions are never retroactively stamped. If a lease expires,
`trail agent capture reconcile` records the run as expired and closes its open
turn/session as `interrupted`, preserving recoverable evidence without claiming
clean completion.

`trail agent start` creates and closes this managed capture lease automatically.
The terminal task's pre-created lane session remains the lifecycle owner; native
hooks attach prompt, tool, approval, transcript, and per-turn checkpoint evidence
to that same session. Provider exit performs one final workdir reconciliation,
which is a no-op when the latest hook checkpoint already captured every change.

## Inspect evidence

```sh
trail --json agent hooks events codex --last 100
trail --json agent artifacts SESSION
trail --json agent provenance SESSION
trail --json agent attest list --session SESSION
trail --json agent attest verify ATTESTATION
trail --json agent git-link list SESSION
trail --json agent export SESSION --attachments
```

Attestations state exactly which completed turns, changes, and evidence
manifests they cover. A signature proves possession of a configured Trail key;
it does not prove model correctness or human approval. Revoked keys remain
visible in historical verification.

Portable export is a verified projection, not a second database or hidden Git
branch. Git association occurs only through explicit exact links or a Trail Git
workflow that already knows the exported change boundary.

## Recovery and degraded operation

Provider callbacks are fail-open after attempting durable journaling. If Trail
cannot open the database, the callback writes a redacted, atomic 0600 envelope
under `.trail/runtime/agent-hooks-spool` and still returns the provider's
success contract.

```sh
trail agent hooks replay --pending
trail agent hooks events codex --failed
trail agent hooks retry RECEIPT
trail agent hooks discard RECEIPT
```

Replay recovers stale `processing` rows, honors `next_attempt_at`, applies
bounded exponential backoff, quarantines invalid/poison receipts, and uses
event and span idempotency keys when reconstructing partial work. Discard is an
explicit audited operator decision; it does not delete raw receipt identity.

## Security and privacy

- HTTP hook ingestion requires a configured daemon bearer or Trail token even
  when the rest of the loopback daemon is running in permissive local mode.
- Raw payloads, identifiers, request bodies, config files, transcript files,
  artifacts, spool size, row counts, and exports are bounded.
- Transcript locators must resolve inside the workspace or an approved
  provider-managed root; leaf symlinks and escapes are rejected.
- Secret-like JSON keys and text are redacted before durable receipt or spool
  storage. Diagnostics are bounded and redacted again before display.
- Trail never automatically writes inferred learnings into `CLAUDE.md`,
  `GEMINI.md`, or another provider context file. Only explicitly accepted,
  scoped learnings are eligible for bounded context injection.

Use `trail agent hooks doctor --all --probe` for executable/version discovery,
installation drift, recent delivery, receipt failures, spool pressure, managed
run ownership, stale finalizers, and honest transcript/export fidelity.

## Compatibility policy

Provider manifests are versioned independently from the lifecycle schema.
Unknown native events are retained as inert `provider.<provider>.<event>`
evidence. A provider with no verified version range reports compatibility as
`unknown`; Trail does not infer support from executable presence. Local
manifests are built in. Any future remote manifest must be signed and pass the
same constrained schema, path, command, and ownership checks before use.

For architecture and rationale, see
[Native Agent Hooks and ACP Integration](../design/native-agent-hooks-and-acp.md).
