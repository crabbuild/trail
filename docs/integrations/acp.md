# ACP Agent Integration

CrabDB can sit between an ACP-capable editor and a real ACP coding agent. The
relay is neutral: Claude Code, Codex, or another ACP agent remains the real
agent, while CrabDB records lanes, turns, transcripts, events, and checkpoints.

## Built-In ACP Providers

CrabDB ships provider profiles for:

- `claude-code`, via `@agentclientprotocol/claude-agent-acp`
- `codex`, via `@agentclientprotocol/codex-acp`

Any other ACP-compatible agent can still be used by passing its upstream ACP
command after `--` to `crabdb agent acp` or `crabdb acp relay`.

## Quickstart

Install CrabDB normally:

```sh
make install
```

Check the agent setup:

```sh
crabdb agent doctor --provider claude-code
crabdb agent doctor --provider codex
```

Print editor configuration that creates a fresh CrabDB task lane for each ACP
session:

```sh
crabdb agent setup
```

After one prompt:

```sh
crabdb agent next
```

Then inspect or apply as needed:

```sh
crabdb agent
crabdb agent board
crabdb agent stack
crabdb agent summary latest
crabdb agent review-data latest
crabdb agent story latest
crabdb agent tools latest
crabdb agent impact latest
crabdb agent review-map latest
crabdb agent mark-file-reviewed latest README.md
crabdb agent risk latest
crabdb agent confidence latest
crabdb agent test-plan latest
crabdb agent ready latest
crabdb agent diagnose latest
crabdb agent compare <TASK_A> <TASK_B>
crabdb agent handoff latest
crabdb agent receipt latest
crabdb agent pr latest
crabdb agent report latest --markdown
crabdb agent validate latest
crabdb agent test latest -- cargo test
crabdb agent brief latest
crabdb agent workdir latest
crabdb agent delta latest
crabdb agent new latest
crabdb agent changes latest
crabdb agent review-flow latest
crabdb agent ask walk me through review
crabdb agent turn
crabdb agent turn-diff latest --patch
crabdb agent files latest
crabdb agent checkpoints latest
crabdb agent why README.md
crabdb agent turn-diff latest --file README.md --patch
crabdb agent review-plan latest
crabdb agent focus latest
crabdb agent view latest
crabdb agent ready latest
crabdb agent apply latest
```

The lower-level Claude Code ACP profile still uses:

```sh
crabdb acp relay --provider claude-code --materialize -- npx -y @agentclientprotocol/claude-agent-acp@latest
```

The lower-level Codex ACP profile uses:

```sh
crabdb acp relay --provider codex --materialize -- npx -y @agentclientprotocol/codex-acp@latest
```

For a full operator and automation-agent runbook, including real Claude Code
edit verification and ACP permission responses, see
[ACP Agent Usage Runbook](./acp-agent-usage.md).

## Daily Use

For day-to-day use, prefer the task-oriented commands:

```sh
crabdb agent
crabdb agent next
crabdb agent stack
crabdb agent summary latest
crabdb agent review-data latest
crabdb agent story latest
crabdb agent tools latest
crabdb agent impact latest
crabdb agent review-map latest
crabdb agent mark-file-reviewed latest README.md
crabdb agent risk latest
crabdb agent confidence latest
crabdb agent test-plan latest
crabdb agent ready latest
crabdb agent diagnose latest
crabdb agent compare <TASK_A> <TASK_B>
crabdb agent receipt latest
crabdb agent pr latest
crabdb agent report latest --markdown
crabdb agent validate latest
crabdb agent test latest -- cargo test
crabdb agent brief latest
crabdb agent workdir latest
crabdb agent delta latest
crabdb agent new latest
crabdb agent changes latest
crabdb agent review-flow latest
crabdb agent ask walk me through review
crabdb agent turn
crabdb agent turn-diff latest --patch
crabdb agent files latest
crabdb agent checkpoints latest
crabdb agent why README.md
crabdb agent turn-diff latest --file README.md --patch
crabdb agent review-plan latest
crabdb agent focus latest
crabdb agent view latest
crabdb agent ready latest
crabdb agent apply latest
```

For low-level ACP inspection:

```sh
crabdb acp sessions
crabdb transcript <lane>
crabdb lane review <lane>
```

The core terms are:

- **Lane**: branch-like workspace for one agent or task.
- **Turn**: one prompt/response/tool cycle.
- **Checkpoint**: recorded code state after a turn.

If an agent goes sideways, use task-level undo instead of copying checkpoint
ids:

```sh
crabdb agent undo latest
crabdb agent undo latest --turn 2
crabdb agent undo latest --prompt 'Add hook support'
```
