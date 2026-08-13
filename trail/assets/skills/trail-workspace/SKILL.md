---
name: trail-workspace
description: Record and inspect ordinary local work with Trail. Use when an agent needs to initialize or inspect a Trail workspace; review dirty changes; record all or selected paths; query timelines, file history, line provenance, or stable identities; manage Trail branches or checkout previews; or prepare an explicit Trail-to-Git export without using a managed agent task.
---

# Trail Workspace

Use Trail as local operation history beside Git. Git remains shared publication history; a Trail branch is not a Git branch.

## Orient Before Mutating

Locate the intended workspace and inspect both Trail and Git state independently:

```sh
trail --format json status
trail diff --dirty --patch
git status --short
```

Use `--workspace <root>` when discovery could select the wrong `.trail`. If no workspace exists, initialize only after the baseline is explicit: `--from-git` for tracked Git state, `--working-tree` for visible files, or plain `trail init` for an empty Trail root.

## Choose the Workflow

- For status, selective recording, timeline, history, `why`, and stable line/file identity, read [record-and-provenance.md](references/record-and-provenance.md).
- For Trail branches, checkout, merge previews, and explicit Git import/export, read [branches-and-git.md](references/branches-and-git.md).

## Preserve User Work

Apply a read-preview-mutate-verify loop. Keep unrelated edits out of the operation. Inspect ignore policy rather than reaching for `--allow-ignored`; never record `.trail`, `.git`, credentials, tokens, or private keys.

Treat checkout, merges, Git commit export, destructive branch operations, and overwriting dirty files as consequential. Preview first and require explicit user intent for the mutation. Finish by reporting the recorded operation or provenance result, remaining dirty paths, and the exact safe next command.
