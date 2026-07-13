---
meta:
  contentType: Conceptual
  title: How users set up Trail agents for editors and terminals
---

# How users set up Trail agents for editors and terminals

Trail will configure supported Agent Client Protocol (ACP) editors and prepare terminal agents through one guided `trail agent setup` flow. After setup, you choose an agent and a surface while Trail selects the integration mode and records both paths as ordinary Trail agent tasks.

## Document plan

- **Overview**: define one setup and daily-use contract for editor and terminal agents
- **Goal**: let you configure Trail once, confirm one preview, and start an agent without managing ACP commands, lane names, or session identifiers
- **Audience**: Trail contributors who implement or review agent setup, provider resolution, editor adapters, and task capture
- **Content plan**: cover the product contract, command behavior, architecture, configuration safety, failures, and tests
- **Open questions**: none; the product and safety decisions in this document are approved

## Current behavior and problem

Trail already includes the ACP relay, built-in and registry providers, Model Context Protocol (MCP) injection, fresh task lanes, transcript capture, provider diagnostics, and a shared review and apply workflow. The current `trail agent setup` command prints one editor snippet but does not install it.

This leaves protocol-specific work with you. You must choose an editor target, copy settings, understand the relay command, and remember a provider flag for terminal tasks. Trail should retain those low-level controls without requiring them during standard setup or daily work.

## Product goals

The change must meet these goals:

1. Configure supported editors and the terminal default through one command
2. Preview every configuration write, provider download, and workspace initialization before mutation
3. Require confirmation before interactive writes
4. Preserve a copyable configuration path for unsupported editors
5. Route editor and terminal sessions into the same high-level Trail task workflow
6. Keep setup idempotent, transactional, and recoverable
7. Hide ACP terminology from standard daily-use output

## Non-goals

This change will not:

- Remove `trail acp` or its diagnostics
- Add a graphical setup application
- Claim that terminal capture has the same streaming detail as ACP capture
- Modify unrelated editor settings
- Install configuration for an editor without an exact adapter for its settings schema
- Change Trail’s review, readiness, apply, or finish semantics

## User contract

Run one command from a repository:

```sh
trail agent setup
```

In an interactive terminal, Trail detects the workspace, available providers, supported editor installations, and existing Trail-owned entries. Trail asks you to choose only when detection cannot produce one unambiguous choice.

Trail then prints one plan:

```text
Trail agent setup

Provider: Codex
Editors:
  ✓ Zed
  ✓ VS Code

Will add:
  “Trail · Codex” → trail agent acp --provider codex

No unrelated settings will change.
Backups will be stored in Trail’s user state directory.

Apply these changes? [y/N]
```

After confirmation, Trail applies the plan, verifies the result, and prints the two daily entry points:

```text
Editor: select “Trail · Codex” and start a chat
Terminal: trail agent start
```

Both paths create a fresh Trail agent task. Standard completion uses the existing commands:

```sh
trail agent next
trail agent ready latest
trail agent finish latest
```

You do not need to know the relay command, lane name, or session identifier. The editor label names Trail and the selected agent, not ACP.

## Command behavior

The existing `trail agent setup` command becomes the guided setup surface. Low-level ACP commands remain available for diagnostics and custom integrations.

| Invocation | Behavior |
| --- | --- |
| `trail agent setup` | Detect choices, preview the plan, request confirmation, apply, and verify in an interactive terminal |
| `trail agent setup --provider codex` | Preselect Codex and detect supported editors |
| `trail agent setup --editor zed` | Preselect Zed and resolve the provider by precedence |
| `trail agent setup --print` | Print the plan and snippets without writing files or downloading providers |
| `trail agent setup --yes` | Apply a fully resolved plan without prompting |
| `trail --json agent setup` | Emit the resolved read-only plan as JSON |
| `trail --json agent setup --yes` | Apply the resolved plan and emit a structured result |
| `trail acp relay codex` | Retain direct access to the low-level relay |

An interactive invocation may prompt for unresolved choices. A non-interactive invocation must never wait for input. Without `--yes`, a non-interactive invocation remains read-only and returns the same plan and snippets that existing automation expects.

Provider selection uses this precedence:

1. Explicit `--provider`
2. The workspace’s saved agent provider
3. The only detected ready provider
4. An interactive choice when several providers are ready
5. The current `claude-code` compatibility default in non-interactive output

Editor selection uses this precedence:

1. Explicit `--editor`
2. Every detected editor with an exact Trail adapter
3. Generic copyable output when no exact adapter exists

`--editor` may be repeated to configure more than one supported editor. `generic` always selects read-only snippet output.

## Workspace and provider preparation

Setup starts with a read-only preflight. If the current Git repository has no Trail workspace, the plan may include `trail init --from-git`. Trail must show that initialization before confirmation and must not choose a baseline for a non-Git directory.

The provider resolver reports whether the selected provider needs an executable, package runner, registry download, or cached binary. Setup includes any Trail-controlled download in the preview and performs it only after confirmation. Diagnostics must not expose environment values, authentication data, or provider output that may contain secrets.

Setup saves the chosen provider as `agent.default_provider` in `.trail/config.toml`. The new configuration section uses a Serde default so existing workspaces remain readable. `trail agent start` uses this value when `--provider` is absent, while an explicit flag always wins.

## Setup architecture

Setup separates detection and planning from mutation:

```text
trail agent setup
        |
        v
Workspace, provider, and editor detection
        |
        v
Read-only setup plan
        |
        v
Preview and confirmation
        |
        v
Transactional apply
        |
        v
Provider and editor verification
```

The implementation contains five bounded components:

- **Detector**: locates the workspace, Trail executable, provider launchers, editor installations, and relevant settings files
- **Planner**: converts detected state and command arguments into a serializable setup plan without writing or downloading
- **Editor adapter**: owns detection, scoped merge, snippet generation, and verification for one exact editor or extension schema
- **Applier**: acquires configuration locks, creates secure backups, uses atomic per-file replacements, and rolls back the transaction on failure
- **Verifier**: checks the saved default, provider readiness, configured editor entries, and generated launch commands

The planner is the only source for human previews, JSON output, and mutation input. The applier must reject a plan when any source-file digest differs from the digest captured during planning.

## Setup plan and result

The structured setup plan contains:

- Workspace root and optional initialization action
- Trail executable path
- Selected provider and capability mode
- Provider acquisition actions and network requirements
- Previous and proposed workspace default provider
- Editor adapter identifiers
- Target configuration paths and source digests
- Trail-owned keys to add, update, or remove
- Redacted scoped diffs
- Generic fallback snippets
- Verification checks
- Whether confirmation is required

The apply result records each attempted action, its status, rollback status, and verification outcome. Human output summarizes those fields, while JSON preserves stable field names for automation.

## Editor adapters and configuration ownership

The first exact adapter targets Zed’s native external-agent settings. VS Code support requires an adapter for a known ACP client extension and its exact settings schema. When Trail cannot identify that extension, it prints the generic command and snippet instead of guessing a settings key.

Each configured entry uses a stable Trail-owned identifier such as `trail-codex` and a display label such as `Trail · Codex`. The generated command uses absolute paths for both the Trail executable and workspace:

```text
/your_trail_executable_path --workspace /your_repository_path agent acp --provider codex
```

Adapters may update only their Trail-owned entry. They must preserve unrelated keys, comments, ordering, and formatting when the target format supports them. Setup records installation ownership and digests in Trail’s local integration metadata so a later run can distinguish a Trail update from your entry.

## Transaction and backup safety

Before writing, the applier locks every target and verifies its planned digest. It stores user-readable-only backups in Trail’s operating-system application state directory. Backups must never enter the repository or `.trail` workspace data because editor settings may contain credentials.

Trail retains the latest verified backup for each target and replaces it during the next successful setup. Failed setup attempts retain the snapshot needed for manual recovery until a later successful setup replaces it.

The applier writes temporary files, parses each result with the target adapter, and then replaces the originals atomically. All supported-editor changes and the workspace default form one transaction. If any write or verification fails, Trail restores every changed target and reports the rollback result.

If the plan initializes Trail, the new workspace joins the same transaction. Rollback removes it only when its digest still matches the state that setup created. Otherwise, Trail retains the workspace and reports the concurrent change.

Provider acquisition may populate Trail’s immutable cache before configuration commit. A later failure may leave an unreferenced cache artifact, but it must not leave partial editor or workspace configuration. Existing cache maintenance removes unreferenced artifacts.

Running the same setup twice must produce an empty configuration diff after readiness checks. A concurrent editor change causes a digest conflict and a new preview requirement rather than an overwrite.

## Shared editor and terminal task flow

Editor configuration invokes the existing hidden `trail agent acp` entry point with an explicit provider. Terminal startup invokes `trail agent start` with the saved default provider. Both commands must call one shared task-creation boundary for lane naming, baseline selection, task metadata, workdir reporting, review status, and completion actions.

ACP sessions retain their richer streaming transcript, tool-event, permission, MCP injection, and checkpoint capture. Terminal sessions retain their universal isolated workdir and final checkpoint behavior. Both expose the same task-level selectors and review lifecycle even when their capture detail differs.

## Failure and fallback behavior

Setup handles expected failures without discarding a usable path:

- **No Trail workspace**: include Git-based initialization in the confirmed plan
- **No supported editor**: save the terminal default and print a generic ACP snippet
- **Unsupported or unknown ACP extension**: leave editor settings untouched and print the launch command and snippet
- **Malformed editor settings**: report the parse location, preserve the file, and print the fallback snippet
- **Missing provider dependency**: report the required runner or download before mutation
- **Stale Trail or repository path**: let `trail agent doctor` identify the stale entry and recommend setup again
- **Concurrent settings change**: abort before writing and request a new preview
- **Write or verification failure**: restore the complete transaction and report any rollback failure as the primary blocker

Fallback output is a successful setup result when terminal use remains ready. Unsupported editor installation is not an error unless the caller explicitly requested that exact editor with `--editor`.

## Diagnostics and terminology

Standard setup and daily-use output uses “editor agent,” “terminal agent,” “provider,” and “Trail task.” It does not teach ACP, MCP, relay, lane, or session concepts.

`trail agent doctor` checks the high-level experience and may name the failed integration layer in diagnostic detail. `trail acp doctor` and `trail acp relay` remain the explicit protocol-level surfaces for contributors, support, and custom hosts.

## Test strategy

Implementation follows test-driven development.

### Detection and planning tests

- Resolve provider and editor precedence deterministically
- Produce no writes or downloads while planning
- Redact unrelated editor values from previews and JSON
- Include workspace initialization only for an uninitialized Git repository
- Reject unresolved non-interactive `--yes` plans

### Editor adapter contract tests

- Detect supported installation paths on each operating system
- Add, update, and remove only the Trail-owned entry
- Preserve unrelated settings, comments, ordering, and formatting
- Generate valid generic snippets for unsupported clients
- Reject malformed and concurrently modified configurations

Every adapter must pass the same fixture-based contract suite before it becomes an automatic-install target.

### Transaction tests

- Create user-readable-only backups outside the workspace
- Apply each target with atomic replacement and transaction rollback
- Restore every target after an injected write or verification failure
- Report rollback failures without claiming recovery
- Produce an empty diff when setup runs twice

### Integration and end-to-end tests

- Use temporary home, application-state, workspace, and editor directories
- Exercise interactive confirmation, rejection, `--print`, `--yes`, and JSON modes
- Verify that tests never read or modify real editor configuration
- Launch a stub ACP editor session and a stub terminal provider session
- Confirm that both sessions produce addressable Trail tasks with the same review and finish actions
- Confirm that ACP capture remains richer without changing the common task contract

Optional release checks may exercise real editor and provider installations. They must run in disposable profiles and must not gate unit tests on network access.

## Completion criteria

The change is complete when:

1. One interactive `trail agent setup` invocation previews, confirms, applies, and verifies all detected supported targets
2. Unsupported editors receive a copyable, workspace-specific fallback without configuration writes
3. `trail agent start` uses the saved provider when no explicit provider is given
4. Editor and terminal sessions enter the same Trail task review and completion workflow
5. Setup is idempotent and rolls back every changed target after an injected failure
6. Non-interactive setup remains read-only unless `--yes` is present
7. JSON plans and results expose stable automation fields without secrets
8. Current CLI, ACP relay, agent workflow, and full Trail regression tests pass
