# Trail Agent Quick Start

Trail Agent runs coding tasks in isolated Trail lanes, records prompts and
changes, and applies reviewed work to Git only after a safety preflight.

## 1. Check Readiness

```sh
make install
trail agent doctor --provider codex
```

For an ACP editor, print and install its configuration snippet:

```sh
trail agent setup --provider codex --editor vscode
```

## 2. Start a Task

For terminal-first work:

```sh
trail agent start --provider codex --name first-agent-task
```

Trail creates a fresh lane and launches the provider inside an isolated
workdir. Editor-based ACP tasks join the same review workflow after the first
prompt completes.

## 3. Review What Happened

```sh
trail agent
trail agent changes latest
trail agent focus latest --patch
```

If you are unsure what to do next:

```sh
trail agent guide
trail agent ask what should I do next
```

## 4. Validate the Task

```sh
trail agent test-plan latest
trail agent test latest -- cargo test
trail agent ready latest
```

`ready` is read-only. It combines review, validation, risk, and Git preflight
information.

## 5. Preview and Apply

```sh
trail agent apply latest --dry-run
trail agent finish latest
```

`finish` applies the task and archives it after success. Use
`trail agent apply latest` if you want the applied task to remain in the normal
inbox.

## 6. Recover a Bad Turn

```sh
trail agent diagnose latest
trail agent checkpoints latest
trail agent undo latest
```

Undo changes only the isolated task lane, not the active Git branch.

## Next Steps

- [Trail Agent overview](../agent/overview.md)
- [Complete user guide](../agent/user-guide.md)
- [CLI reference](../agent/cli-reference.md)
- [Troubleshooting](../agent/troubleshooting.md)
- [ACP integration](../integrations/acp.md)
