# ACP Agent Integration

CrabDB can sit between an ACP-capable editor and a real ACP coding agent. The
relay is neutral: Claude Code, Codex, or another ACP agent remains the real
agent, while CrabDB records lanes, turns, transcripts, events, and checkpoints.

## Claude Code Quickstart

Install CrabDB normally:

```sh
make install
```

Check the ACP setup:

```sh
crabdb acp doctor --agent claude-code
```

Print a relay command and generic editor snippet:

```sh
crabdb acp install --agent claude-code --print
```

The default Claude Code profile uses:

```sh
crabdb acp relay --provider claude-code --materialize -- npx -y @agentclientprotocol/claude-agent-acp@latest
```

## Daily Use

After an ACP prompt runs:

```sh
crabdb acp sessions
crabdb transcript <lane>
crabdb lane review <lane>
```

The core terms are:

- **Lane**: branch-like workspace for one agent or task.
- **Turn**: one prompt/response/tool cycle.
- **Checkpoint**: recorded code state after a turn.

If an agent goes sideways, inspect the transcript and use lane rewind to return
to a known-good checkpoint.
