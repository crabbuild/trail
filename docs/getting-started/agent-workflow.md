# Agent Workflow

Trail gives AI coding agents branch-like memory, transcripts, checkpoints, and
rewind without polluting your active Git branch.

## 1. Install Trail

```sh
make install
trail --help
```

## 2. Check ACP Readiness

For built-in aliases or any current ACP registry agent:

```sh
trail acp doctor --agent claude-code
trail acp doctor --agent codex
trail acp list
trail acp doctor --agent gemini
```

Print the editor command:

```sh
trail acp install --agent claude-code --print
trail acp install --agent codex --print
trail acp install --agent gemini --print
```

## 3. Run One Prompt

Configure your ACP editor to launch the printed relay command. Trail will
create or reuse a lane, start a session, record each prompt as a turn, and
checkpoint materialized workdir changes when the prompt finishes.

## 4. Read What Happened

```sh
trail acp sessions
trail transcript <lane>
trail turn show <turn-id>
```

## 5. Review, Rewind, or Merge

```sh
trail status
trail lane review <lane>
trail merge-lane <lane> --into main --dry-run
```

Use the checkpoint change id shown in the transcript or turn details when you
need to rewind a lane to a known-good state.
