# Agent Workflow

CrabDB gives AI coding agents branch-like memory, transcripts, checkpoints, and
rewind without polluting your active Git branch.

## 1. Install CrabDB

```sh
make install
crabdb --help
```

## 2. Check ACP Readiness

For Claude Code:

```sh
crabdb acp doctor --agent claude-code
```

Print the editor command:

```sh
crabdb acp install --agent claude-code --print
```

## 3. Run One Prompt

Configure your ACP editor to launch the printed relay command. CrabDB will
create or reuse a lane, start a session, record each prompt as a turn, and
checkpoint materialized workdir changes when the prompt finishes.

## 4. Read What Happened

```sh
crabdb acp sessions
crabdb transcript <lane>
crabdb turn show <turn-id>
```

## 5. Review, Rewind, or Merge

```sh
crabdb status
crabdb lane review <lane>
crabdb merge-lane <lane> --into main --dry-run
```

Use the checkpoint change id shown in the transcript or turn details when you
need to rewind a lane to a known-good state.
