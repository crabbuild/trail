# ACP Agent Integration

Trail can sit between an ACP-capable editor and a real ACP coding agent. The
relay is neutral: Claude Code, Codex, or another ACP agent remains the real
agent, while Trail records lanes, turns, transcripts, events, and checkpoints.

## Provider Modes

Trail now treats code-agent integration as three complementary paths:

- **ACP relay**: best capture path. Trail creates a fresh lane per prompt
  session, injects its MCP server into the ACP initialize params, and records
  turns/tool events as they stream through the relay.
- **MCP server**: context-tool path. Register `trail mcp` in the native agent
  when the agent supports MCP so it can inspect Trail state directly.
- **Terminal task**: universal CLI path. `trail agent start <NAME>`
  creates a task lane workdir, runs the provider command there, and records the
  final checkpoint when the process exits. Use `--workdir-mode fuse-cow` to
  mount a transparent COW view for large repositories instead of creating a full
  copied workdir.

## Built-In Aliases and Registry Providers

Trail ships provider profiles for:

- `claude-code`, via `@agentclientprotocol/claude-agent-acp`
- `codex`, via `@agentclientprotocol/codex-acp`
- `cursor`, via the Cursor CLI `agent acp` server
- `gemini`, terminal mode with optional Trail MCP registration
- `aider`, terminal mode
- `opencode`, terminal mode

Trail also reads the [official ACP registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json), so every current registry ID can be launched directly:

```sh
trail agent acp status
trail acp relay gemini
trail acp relay qwen-code
trail acp relay github-copilot-cli
```

Registry package distributions launch through `npx` or `uvx`; matching binary
distributions download into `.trail/acp/agents/` on first use. Trail caches a
validated registry index locally and uses it if the registry is temporarily
unavailable. Built-in aliases remain available without a registry request.

An ACP-compatible agent outside the registry can still be used by passing its
upstream command after `--` to `trail agent acp run` or `trail acp relay`.
Any terminal agent can be used by passing its command after `--` to
`trail agent start`.

## Quickstart

Install Trail normally:

```sh
make install
```

Check the agent setup:

```sh
trail agent doctor claude-code
trail agent doctor codex
trail agent doctor cursor
trail agent doctor gemini
```

Print editor configuration that creates a fresh Trail task lane for each ACP
session:

```sh
trail agent acp setup claude-code --editor vscode
```

After one prompt:

```sh
trail agent next
```

Then inspect or apply as needed:

```sh
trail agent
trail agent board
trail agent stack
trail agent summary latest
trail agent review-data latest
trail agent story latest
trail agent tools latest
trail agent impact latest
trail agent review-map latest
trail agent mark-file-reviewed latest README.md
trail agent risk latest
trail agent confidence latest
trail agent test-plan latest
trail agent ready latest
trail agent diagnose latest
trail agent compare <TASK_A> <TASK_B>
trail agent handoff latest
trail agent receipt latest
trail agent pr latest
trail agent report latest --markdown
trail agent validate latest
trail agent test latest -- cargo test
trail agent brief latest
trail agent workdir latest
trail agent delta latest
trail agent new latest
trail agent changes latest
trail agent review-flow latest
trail agent ask walk me through review
trail agent turn
trail agent turn-diff latest --patch
trail agent files latest
trail agent checkpoints latest
trail agent why README.md
trail agent turn-diff latest --file README.md --patch
trail agent review-plan latest
trail agent focus latest
trail agent view latest
trail agent ready latest
trail agent apply latest
```

The lower-level relay can start a built-in provider directly:

```sh
trail acp relay claude-code
```

For Codex:

```sh
trail acp relay codex
```

For Cursor:

```sh
trail acp relay cursor
```

For an ACP-compatible agent without a built-in profile, keep the explicit
form: `trail acp relay --provider my-agent -- my-agent acp`.

Terminal-first agents use fresh task lanes:

```sh
trail agent start gemini
trail agent start aider
trail agent start opencode
trail agent start custom -- my-agent --flag
```

For large repositories, terminal agents can use the FUSE COW workdir mode:

```sh
trail agent start codex --workdir-mode fuse-cow
trail agent start custom --workdir-mode fuse-cow -- my-agent --flag
trail agent start codex --workdir-mode nfs-cow
```

The FUSE COW mount is held only while the terminal process runs and while Trail
records the checkpoint afterward. On macOS it requires macFUSE; on Linux it
requires FUSE access such as `/dev/fuse`.

On macOS, use `nfs-cow` for terminal-agent copy-up through the built-in NFS
client when installing macFUSE is undesirable.

For a full operator and automation-agent runbook, including real Claude Code
edit verification and ACP permission responses, see
[ACP Agent Usage Runbook](./acp-agent-usage.md).

## Daily Use

For day-to-day use, prefer the task-oriented commands:

```sh
trail agent
trail agent next
trail agent stack
trail agent summary latest
trail agent review-data latest
trail agent story latest
trail agent tools latest
trail agent impact latest
trail agent review-map latest
trail agent mark-file-reviewed latest README.md
trail agent risk latest
trail agent confidence latest
trail agent test-plan latest
trail agent ready latest
trail agent diagnose latest
trail agent compare <TASK_A> <TASK_B>
trail agent receipt latest
trail agent pr latest
trail agent report latest --markdown
trail agent validate latest
trail agent test latest -- cargo test
trail agent brief latest
trail agent workdir latest
trail agent delta latest
trail agent new latest
trail agent changes latest
trail agent review-flow latest
trail agent ask walk me through review
trail agent turn
trail agent turn-diff latest --patch
trail agent files latest
trail agent checkpoints latest
trail agent why README.md
trail agent turn-diff latest --file README.md --patch
trail agent review-plan latest
trail agent focus latest
trail agent view latest
trail agent ready latest
trail agent apply latest
```

For low-level ACP inspection:

```sh
trail agent acp sessions
trail transcript <lane>
trail lane review <lane>
```

The core terms are:

- **Lane**: branch-like workspace for one agent or task.
- **Turn**: one prompt/response/tool cycle.
- **Checkpoint**: recorded code state after a turn.

If an agent goes sideways, use task-level undo instead of copying checkpoint
ids:

```sh
trail agent undo latest
trail agent undo latest --turn 2
trail agent undo latest --prompt 'Add hook support'
```
