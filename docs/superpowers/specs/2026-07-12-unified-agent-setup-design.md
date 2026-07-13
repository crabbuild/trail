---
meta:
  contentType: Conceptual
  title: How do you set up Trail agents with ACP and native hooks?
---

# How do you set up Trail agents with ACP and native hooks?

Trail separates Agent Client Protocol (ACP) setup from native hook setup. You choose the transport first, then configure its provider. Both transports record the same Trail tasks, lifecycle events, evidence, and checkpoints.

## Document plan

- **Overview**: define the ACP, hooks, and terminal command boundaries
- **Goal**: explain how Trail configures each agent transport through one consistent setup contract
- **Audience**: Trail contributors who implement or review agent setup, provider resolution, adapters, and capture
- **Content plan**: cover commands, provider syntax, planning, configuration ownership, runtime capture, failures, and tests
- **Open questions**: none; the product and safety decisions in this document are approved

## Why Trail separates ACP and hooks

Trail currently spreads agent setup across `trail agent setup`, `trail acp install`, `trail agent acp`, and `trail agent hooks add`. These commands use different provider catalogs, editor lists, mutation rules, and terminology.

The new model exposes two sibling integration categories:

- `trail agent acp` owns editor and ACP integration
- `trail agent hooks` owns native provider hooks

Terminal agents need no setup command. `trail agent start` launches an isolated task directly.

## Product goals

The change must meet these goals:

1. Give ACP and native hooks distinct command boundaries
2. Use the same provider syntax across setup, diagnostics, and terminal startup
3. Use one planning and transaction contract for both setup categories
4. Route ACP, native hooks, terminal tasks, and hybrid capture into the same task workflow
5. Preserve unrelated editor and provider configuration
6. Keep every setup idempotent, transactional, and recoverable
7. Remove the old setup model without aliases or migration behavior

## Non-goals

This change will not:

- Add a combined setup orchestrator
- Add a graphical setup application
- Make ACP and native hooks expose identical evidence
- Require setup before a terminal task
- Install configuration without an exact adapter for the target schema
- Change task review, readiness, apply, or finish semantics
- Preserve removed command syntax

## Public command model

The public command tree makes the transport boundary visible:

```text
trail agent
├── acp
│   ├── setup
│   ├── status
│   ├── doctor
│   └── sessions
├── hooks
│   ├── setup
│   ├── status
│   ├── doctor
│   ├── events
│   ├── replay
│   └── remove
└── start
```

Configured editors invoke the hidden ACP runner. Provider hook files invoke the hidden singular hook receiver:

```sh
trail agent acp run codex
trail agent hook receive codex Stop
```

`trail acp relay` remains the low-level surface for custom ACP hosts and relay diagnostics. It does not configure editors or replace `trail agent acp setup`.

## Provider arguments

Every provider-aware agent command accepts a provider as its preferred positional argument:

```sh
trail agent start codex
trail agent acp setup codex --editor zed
trail agent hooks setup codex --scope project
trail agent acp doctor codex
trail agent hooks status codex
```

The explicit `--provider` form remains part of the new model for automation and callers that prefer named arguments:

```sh
trail agent start --provider codex
trail agent acp setup --provider codex --editor zed
trail agent hooks setup --provider codex --scope project
```

A command rejects input that supplies both forms. After resolution, Trail stores and reports the canonical provider name.

`trail agent start` uses `agent.default_provider` only when neither form is present. You set that general terminal default explicitly:

```sh
trail config set agent.default_provider codex
trail agent start
```

ACP and hooks setup never change the terminal default.

Interactive setup may ask you to choose a provider when you omit it. Non-interactive setup requires the positional or named form.

## Shared setup contract

Both setup categories use the same operation sequence:

```text
resolve provider
    -> inspect workspace and targets
    -> build a read-only plan
    -> preview and confirm
    -> lock and apply
    -> verify
    -> commit metadata or roll back
```

An interactive invocation previews the plan and requests confirmation. `--print` and JSON output return the plan without writing files or downloading providers. `--yes` applies a fully resolved plan without prompting.

A non-interactive invocation never waits for input. Without `--yes`, it remains read-only.

## ACP setup ownership

`trail agent acp setup` configures editor and ACP integration for one provider:

```sh
trail agent acp setup codex --editor zed
trail agent acp setup codex --editor vscode --yes
trail --json agent acp setup codex --editor generic
```

ACP setup may perform these actions:

- Resolve a built-in or registry ACP provider
- Report a package runner, download, or cached binary requirement
- Detect supported editors when `--editor` is absent
- Configure exact editor adapters
- Generate a generic copyable entry for unsupported editors
- Inject Trail Model Context Protocol (MCP) tools into the hidden runner
- Record ACP integration ownership and source digests
- Verify the provider, editor entry, workspace path, and launch command

ACP setup must not install or remove native hooks. It must not change `agent.default_provider`.

Each editor entry uses a stable Trail-owned identifier such as `trail-codex` and a label such as `Trail · Codex`. The generated entry uses absolute executable and workspace paths:

```text
/path/to/trail --workspace /path/to/repository agent acp run codex
```

The first exact adapter targets Zed external-agent settings. VS Code support requires an adapter for a known ACP client extension and its exact settings schema. Trail prints a generic entry when it cannot identify that schema.

## Native hook setup ownership

`trail agent hooks setup` installs or updates Trail-owned native hooks for one provider:

```sh
trail agent hooks setup codex
trail agent hooks setup claude-code --scope user
trail --json agent hooks setup gemini --scope project
```

Hooks setup may perform these actions:

- Resolve a built-in native hook manifest
- Probe provider compatibility when requested
- Merge Trail-owned entries into shared JSON configuration
- Create an owned plugin, extension, or hook file
- Record ownership inventory and before/after digests
- Verify the installed configuration and provider contract

Hooks setup must not configure editors, acquire ACP registry providers, or change ACP integration metadata. It must not change `agent.default_provider`.

Project scope is the default. User scope writes only to the provider’s declared user location. `trail agent hooks remove` removes exact Trail-owned entries or files after ownership verification.

## Terminal task behavior

`trail agent start` launches a provider without a setup phase:

```sh
trail agent start codex
trail agent start custom -- my-agent --flag
```

The command creates a fresh task lane, materializes or mounts its workdir, starts a managed capture run, launches the provider, and records the final checkpoint. Installed native hooks may enrich the same task with prompts, tools, approvals, transcripts, and per-turn checkpoints.

The terminal command resolves its provider from the positional argument, `--provider`, or `agent.default_provider`, in that order. It reports an error when all three are absent.

## Setup architecture

The implementation contains one shared setup framework and transport-specific adapters:

- **Provider resolver**: returns one canonical identity and transport capabilities
- **Detector**: locates the workspace, executable, provider dependencies, editor installations, and target files
- **Planner**: converts detected state and arguments into a serializable plan without mutation
- **ACP adapter**: owns editor detection, scoped merges, generated entries, and verification
- **Hook adapter**: owns provider hook configuration, event contracts, and verification
- **Applier**: locks targets, creates secure backups, writes atomically, and rolls back failures
- **Verifier**: checks every result against the plan before metadata commit

The planner is the only input to human previews, JSON output, and mutation. The applier rejects a plan when a source digest differs from the inspected value.

## Setup plans and results

Every setup plan contains common fields:

- Transport category
- Workspace root and optional initialization action
- Trail executable path
- Canonical provider and capability summary
- Acquisition actions and network requirements
- Target paths and source digests
- Trail-owned entries or files
- Redacted scoped diffs
- Verification checks
- Confirmation requirement

ACP plans also contain editor adapter identifiers and fallback entries. Hook plans also contain scope, manifest version, provider version range, and ownership inventory.

The apply result records every attempted action, its status, rollback status, and verification outcome. Human output summarizes these fields. JSON keeps stable field names for automation.

## Workspace and provider preparation

Setup begins with a read-only preflight. In an uninitialized Git repository, the plan may include `trail init --from-git`. Trail shows that action before confirmation. It does not choose a baseline for a non-Git directory.

Provider resolution reports every Trail-controlled download before mutation. Diagnostics redact environment values, authentication data, unrelated configuration values, and provider output that may contain secrets.

The ACP and hook resolvers use one provider capability manifest. A provider may support either transport, both transports, or terminal execution only. Each setup command rejects providers that do not support its transport.

## Configuration ownership and transactions

Before writing, the applier locks every target and compares its current digest with the plan. It stores user-readable-only backups in Trail’s operating-system application state directory. Editor and provider settings may contain credentials, so backups never enter the repository or `.trail` data.

Adapters update only their Trail-owned entry or file. Shared formats preserve unrelated keys and hooks. Owned files reject foreign content unless the operator has explicitly authorized replacement through that transport’s setup contract.

Each setup invocation forms one transaction. Trail writes temporary files, parses each result with its adapter, and replaces targets atomically. A write or verification failure restores every changed target.

If the plan creates a Trail workspace, rollback removes it only while its digest matches the created state. A concurrent change preserves the workspace and becomes the primary reported conflict.

Provider acquisition may leave an unreferenced immutable cache artifact after a later failure. It must not leave partial workspace, editor, or hook configuration.

Repeated setup with the same arguments produces no configuration diff. A concurrent target change aborts the operation and requires a new plan.

## Runtime capture boundaries

The hidden `trail agent acp run` command creates a fresh task and starts the ACP relay with MCP injection. The hidden `trail agent hook receive` command journals one bounded, redacted native receipt and returns the provider’s success response.

ACP, native hooks, terminal wrappers, and hybrid capture use the same versioned lifecycle vocabulary and capture coordinator. Provider adapters translate native events. They do not own task creation, checkpoint policy, deduplication, or finalization.

When an exact native session identity matches an active ACP session, Trail records hybrid capture. ACP owns lifecycle progression while native hooks add receipts and transcript evidence. Ambiguous matches fail closed.

## Hard cutover

The implementation removes the old setup model:

- Remove `trail agent setup`
- Remove `trail acp install`
- Remove `trail agent hooks add`
- Remove the old leaf form of `trail agent acp`
- Remove old help text, examples, tests, and documentation

The cutover adds no aliases, deprecation warnings, migration messages, or compatibility shims. Removed syntax receives the command parser’s standard unknown-command or missing-subcommand response.

`trail acp relay` remains because it serves custom ACP integrations. The explicit `--provider` form also remains because it is part of the new positional-provider contract.

## Failure behavior

Each setup category reports failures within its boundary:

- **Missing workspace**: include Git-based initialization in the confirmed plan
- **Unsupported transport**: report the provider capability that is absent
- **Unsupported editor**: print a generic ACP entry without changing editor settings
- **Malformed target**: report the parse location and preserve the file
- **Missing provider dependency**: show the required runner or acquisition action before mutation
- **Concurrent target change**: abort and require a new preview
- **Write or verification failure**: restore the full setup transaction
- **Rollback failure**: report the unrecovered target as the primary blocker
- **Native delivery failure**: spool the redacted receipt and preserve provider success behavior

ACP failure never changes hook configuration. Hook failure never changes editor or ACP configuration.

## Diagnostics and terminology

Public output names the selected category:

- `trail agent acp doctor` diagnoses editor, provider, relay, MCP, and capture readiness
- `trail agent hooks doctor` diagnoses compatibility, ownership, drift, delivery, receipts, spool pressure, and transcript fidelity
- `trail agent doctor` remains task-oriented and checks terminal launch plus workspace readiness

Daily output uses provider, editor agent, terminal agent, native hooks, and Trail task. Protocol terms such as relay, lane, session mapping, and receipt appear only where they explain an ACP or hooks diagnostic.

## Test strategy

Implementation follows test-driven development.

### Command contract tests

- Accept the positional provider and `--provider` forms
- Reject input that supplies both provider forms
- Resolve `agent.default_provider` only for terminal startup
- Require a provider for non-interactive setup
- Expose only the new command tree in help output
- Reject removed command syntax through the standard parser path

### Planning and adapter tests

- Produce no writes or downloads while planning
- Emit the same plan through human and JSON projections
- Redact unrelated configuration values
- Reject providers that lack the selected transport
- Verify ACP editor adapter contracts on each operating system
- Verify native hook adapter contracts for every built-in manifest
- Preserve unrelated configuration and reject malformed targets

### Transaction tests

- Store secure backups outside the workspace
- Detect source changes after planning
- Apply every target through atomic replacement
- Restore every changed target after an injected failure
- Report rollback failures without claiming recovery
- Produce no configuration diff after repeated setup

### Integration and end-to-end tests

- Exercise confirmation, rejection, `--print`, `--yes`, and JSON behavior
- Keep test homes, workspaces, caches, and editor settings isolated
- Launch a stub editor through hidden `trail agent acp run`
- Deliver stub native events after `trail agent hooks setup`
- Launch terminal tasks with positional, named, and configured providers
- Confirm that every transport creates addressable tasks with the same review and finish actions
- Confirm that hybrid capture deduplicates turns, messages, spans, and checkpoints

Optional release checks may exercise real editors and providers in disposable profiles. Unit and integration tests do not require network access.

## Completion criteria

The change is complete when:

1. ACP setup configures and verifies supported editor integrations through `trail agent acp setup`
2. Hook setup configures and verifies native provider hooks through `trail agent hooks setup`
3. Both setup categories use the same plan, confirmation, transaction, and verification contract
4. Terminal startup requires no setup and accepts a positional provider
5. The explicit `--provider` form behaves identically to the positional form
6. Generated editors launch the hidden `trail agent acp run`
7. ACP, hooks, terminal, and hybrid capture retain one task and evidence model
8. Removed commands, aliases, documentation, and tests no longer exist
9. Repeated setup is idempotent and injected failures restore every changed target
10. Human and JSON output expose stable, redacted plan and result fields
