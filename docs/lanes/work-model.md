# Lane Work Model

A Trail lane is a branch-backed work container. It stores code state like a
branch, but also stores the operational context around the work: sessions,
turns, messages, events, patches, workdir records, gates, approvals, readiness
reports, handoff packets, and rewind checkpoints.

Use a branch for a long-lived code line such as `main`, `release`, or
`experiment`. Use a lane for one active unit of work by a human, automation, or
external coding agent.

## The Short Model

```text
branch = code state

lane   = code state
       + task identity
       + optional materialized workdir
       + optional session
       + optional turns
       + optional messages and events
       + optional test/eval gates
       + readiness, review, handoff, rewind, and merge records
```

A branch can answer:

```text
What files changed?
What root or operation is this ref pointing at?
Can this source merge into that target?
```

A lane can also answer:

```text
What prompt started this work?
What user and assistant messages were recorded?
Which turn produced the operation?
Which patch or workdir record changed the code?
Which tests and evals passed or failed?
Is the lane ready to merge?
What should a reviewer or next host look at?
Can the lane rewind to a known-good point?
```

## Branch vs Lane

```text
refs/branches/main
        |
        v
  operation/root history
        |
        v
  code state


refs/lanes/docs-lane
        |
        v
  operation/root history
        |
        v
  code state
        |
        +-------------------------------+
                                        |
                                        v
                            lane activity records
                            sessions, turns, messages,
                            events, gates, approvals,
                            workdir state, handoff,
                            readiness, rewind records
```

Internally, a lane has two related records:

```text
lanes
  lane_id
  name
  kind/provider/model metadata
  created_at
       |
       v
lane_branches
  ref = refs/lanes/<name>
  base change/root
  head change/root
  current session
  optional workdir
  status
```

The ref still points at normal Trail operation/root history. The lane records
add task and activity context around that ref.

## Lanes, Sessions, Turns, Messages, and Events

The activity model is layered. A lane can exist without a session. A session can
contain many turns. A turn can contain many messages and events.

```text
lane: docs-lane
  |
  +-- session: session-docs-2026-06-26
        |
        +-- turn: turn_001
        |     |
        |     +-- message(role=user)
        |     +-- message(role=assistant)
        |     +-- event(type=tool_call)
        |     +-- event(type=patch_applied)
        |     +-- operation(change_id=change_...)
        |
        +-- turn: turn_002
              |
              +-- message(role=user)
              +-- message(role=assistant)
              +-- event(type=test_started)
              +-- event(type=test_finished)
```

The practical meaning:

- A lane is the task workspace.
- A session is one continuous run or handoff context inside the lane.
- A turn is one bounded prompt/response/tool cycle.
- A message is durable text from a user, assistant, system, or tool role.
- An event is structured activity such as `prompt_received`, `tool_call`,
  `patch_applied`, `workdir_recorded`, `test_started`, or `approval_requested`.

Sessions are not required for simple manual work:

```sh
trail lane spawn docs-lane --from main --materialize=true
trail lane record docs-lane -m "manual docs edit"
trail lane readiness docs-lane
```

Sessions are the right default when work should preserve transcript context:

```sh
trail session start docs-lane --title "Docs update" --id session-docs
trail lane turn start docs-lane --title "Handle user request"
```

## What Trail Records Automatically vs Explicitly

Trail records code-state changes when a command applies or records them. It
does not magically see prompts or transcripts from Codex, Claude Code, Cursor,
or another host unless that host sends the activity to Trail.

Manual or direct integration recording looks like this:

```text
real agent or host
      |
      +-- trail lane turn message ... --role user
      +-- trail lane turn message ... --role assistant
      +-- trail lane turn event ... --event-type tool_call
      +-- trail lane record ... or lane turn apply-patch ...
      +-- trail lane test/eval ...
      +-- trail lane turn end ...
```

Relay-style automation should make that invisible to the user:

```text
editor or agent host
      |
      v
ACP/MCP/CLI integration
      |
      v
Trail Relay
      |
      +-- forwards prompts to the real agent
      +-- mirrors prompts into lane messages/events
      +-- mirrors assistant output into lane messages
      +-- mirrors tool calls/results into lane events
      +-- records patches or workdir changes into the lane
      +-- records tests/evals as lane gates
      +-- exposes readiness/review/handoff back to the host
```

Without such a relay, the host or script must call Trail explicitly.

## Two Ways to Change a Lane

### Structured Patch Flow

Structured patches update the lane branch directly. This is useful for editor,
MCP, ACP relay, or script-driven integrations.

```text
patch.json
   |
   v
trail lane apply-patch docs-lane --patch patch.json
   |
   v
validate base_change, paths, ignored files, root, line guards
   |
   v
record LanePatch operation
   |
   v
refs/lanes/docs-lane points at new head
```

Example:

```sh
trail lane spawn docs-lane --from main --no-materialize
trail lane apply-patch docs-lane --patch patch.json
trail lane diff docs-lane --patch
```

For direct lane patches, `patch.json` must include a `base_change` matching the
current lane head. A tool can intentionally bypass that freshness check with
`allow_stale: true` in the patch document or `--allow-stale` on the CLI.
Turn-linked patches may omit `base_change` because the turn's `before_change`
acts as the freshness guard.

Turn-linked patch flow:

```sh
trail lane turn start docs-lane --title "Apply docs patch"
# Use the printed turn id in the next commands.
trail lane turn message <turn-id> --role user --text "Update the docs"
trail lane turn apply-patch <turn-id> --patch patch.json
trail lane turn end <turn-id> --status completed
```

### Materialized Workdir Flow

Materialized workdirs are useful when a tool needs real files, command
execution, incremental edits, language servers, formatters, or test runners.

```text
refs/lanes/docs-lane
        |
        v
materialized workdir
        |
        v
human/editor/agent edits files
        |
        v
trail lane record docs-lane -m "record workdir changes"
        |
        v
record LaneRecord operation
        |
        v
refs/lanes/docs-lane points at new head
```

Example:

```sh
trail lane spawn docs-lane --from main --materialize=true
LANE_DIR="$(trail lane workdir docs-lane)"
cd "$LANE_DIR"

# Edit files manually or run an external coding agent here.

cd /path/to/project
trail lane status docs-lane
trail lane record docs-lane -m "record docs update"
```

For large repositories, prefer sparse materialization:

```sh
trail lane spawn docs-lane \
  --from main \
  --materialize=true \
  --paths docs README.md
```

Only selected paths are written into the lane workdir. More paths can be read or
hydrated later:

```sh
trail lane read docs-lane docs/guide.md
trail lane sync-workdir docs-lane --paths docs --include-neighbors
```

## Daily Human or Agent Workflow

```text
one task
  |
  v
spawn lane
  |
  v
materialize workdir or apply structured patches
  |
  v
record session/turn/messages/events when context matters
  |
  v
record lane code changes
  |
  v
run test/eval gates
  |
  v
review readiness
  |
  +-- ready ---------> merge-lane or merge queue
  |
  +-- not ready -----> continue, approve, resolve conflict, or rewind
```

Concrete command flow:

```sh
cd /path/to/project

# One-time per project.
trail init --working-tree

# One lane per task.
trail lane spawn docs-lane --from main --materialize=true

# Optional but recommended for agent work.
trail session start docs-lane --title "Docs update" --id session-docs
trail lane turn start docs-lane --title "Handle user request"

# Work in the lane workdir.
LANE_DIR="$(trail lane workdir docs-lane)"
cd "$LANE_DIR"
# edit files or run Codex/Claude/Cursor here

# Record from the original project workspace.
cd /path/to/project
trail lane record docs-lane -m "record docs update"

# Inspect and gate.
trail lane diff docs-lane --patch --show-line-ids
trail lane review docs-lane
trail lane test docs-lane --suite unit -- cargo test
trail lane readiness docs-lane

# Merge when ready.
trail merge-lane docs-lane --into main --dry-run
trail merge-queue add docs-lane --into main
trail merge-queue run
```

## Readiness Model

Readiness is a rollup. It answers whether a lane is safe to merge right now and
why not when it is blocked.

```text
lane readiness docs-lane
        |
        +-- lane status
        +-- changed paths
        +-- dirty materialized workdir?
        +-- pending approvals?
        +-- open conflicts?
        +-- queued merge already?
        +-- latest test gate
        +-- latest eval gate
        +-- required gate config
        |
        v
ready / blocked + blockers + warnings
```

Example output shape:

```text
Lane readiness: docs-lane (ready)
Ref: refs/lanes/docs-lane
Ready: true
Changed paths: 1
Blockers: 0
Warnings: 0
Latest test: test_passed
Latest eval: eval_passed
```

Common blockers include:

- Dirty lane workdir that has not been recorded.
- Pending approval for a risky action.
- Open conflict against the merge target.
- Missing required test or eval gate.
- Failed latest required test or eval gate.
- Removed or invalid lane state.

Lane status and readiness also expose when the lane's saved base is behind the
workspace default branch. JSON status includes `base_status.operations_behind`
and `base_status.stale`; readiness emits a `stale_lane_base` warning, for
example: `lane started 14 operations behind main`.

## Optional Lane Hardening

Lane isolation is permissive by default for compatibility. Workspaces can opt
into stricter boundaries:

```sh
trail config set lane.claim_enforcement reject
trail config set lane.enforce_sparse_paths true
trail config set lane.max_changed_paths 25
trail config set lane.max_patch_bytes 1048576
trail config set lane.max_event_payload_bytes 65536
trail config set lane.max_trace_payload_bytes 65536
```

`lane.claim_enforcement=warn` records a `lane_policy_warning` event when a lane
touches paths outside its active write claims/leases. `reject` blocks the
mutation. Read leases do not grant write permission under this policy.
`lane.enforce_sparse_paths=true` turns sparse lane `--paths` selections into a
hard write boundary for lane patches and materialized workdir records.

## Review, Handoff, Rewind, and Merge

Review summarizes the lane:

```sh
trail lane review docs-lane --limit 50
```

It includes readiness, changed paths, recent operations, sessions, events, trace
spans, approvals, conflicts, and gates.

Handoff packages context for another host or reviewer:

```sh
trail lane handoff docs-lane --limit 50
```

Rewind moves a lane back to a known-good change/root while preserving audit
history:

```sh
trail lane rewind docs-lane \
  --to <change-or-root> \
  --record-current \
  --sync-workdir
```

Merge uses the lane ref as source:

```sh
trail merge-lane docs-lane --into main --dry-run
```

Use the merge queue for shared targets:

```sh
trail merge-queue add docs-lane --into main
trail merge-queue run
```

The merge updates Trail's target branch ref. Git history remains separate until
you export, checkout, or commit through the Git workflow.

If a merge pauses on conflict, Trail records the base, target, and source root
snapshots for that merge. Conflict explanations and resolutions use those stored
roots and label each path with a conservative conflict class, so later source
lane movement does not change what is being resolved. If the target ref moves,
Trail rejects the stale resolution instead of overwriting newer target work.

```text
refs/lanes/docs-lane              refs/branches/main
        |                                  |
        v                                  v
  lane head operation  ---- merge ---->  new main operation
        |                                  |
        v                                  v
  lane activity remains             accepted Trail branch state
```

## Large Repo Example: Sparse Lane on Svelte

This example was run against `sveltejs/svelte`, a real framework repository with
8,944 tracked files at the tested checkout.

Clone and initialize:

```sh
git clone --depth 1 https://github.com/sveltejs/svelte /tmp/trail-svelte-lane-demo
cd /tmp/trail-svelte-lane-demo
trail init --from-git
```

Observed output:

```text
Imported: 8944 files (8941 text, 0 opaque, 3 binary)
Worktree: clean
```

Create a sparse lane for a README-only task:

```sh
trail lane spawn svelte-readme \
  --from main \
  --materialize=true \
  --paths README.md

trail lane workdir svelte-readme
```

The materialized lane workdir contained only:

```text
README.md
.trail/sparse-workdir.json
.trail/workdir-manifest.json
```

Record the run context:

```sh
trail session start svelte-readme \
  --title "README clarity pass" \
  --id session-svelte-readme

trail lane turn start svelte-readme \
  --title "Handle user README prompt"

trail lane turn message <turn-id> \
  --role user \
  --text "In this large repo, add a tiny README note."

trail lane turn message <turn-id> \
  --role assistant \
  --text "I will edit only README.md in the sparse lane workdir."

trail lane turn event <turn-id> \
  --event-type prompt_received \
  --payload-json '{"repo":"sveltejs/svelte","mode":"sparse-lane"}'
```

Edit `README.md` in the sparse lane workdir, then record:

```sh
trail lane status svelte-readme
trail lane record svelte-readme -m "record README demo note"
```

Observed status before recording:

```text
svelte-readme active
Workdir: DirtyTracked
  workdir Modified README.md
```

Observed record output:

```text
Recorded lane workdir change_4ee8...
  Modified README.md
```

Run gates:

```sh
trail lane test svelte-readme \
  --suite readme-smoke \
  -- sh -c 'grep -q "Demo note" README.md'

trail lane eval svelte-readme \
  --suite sparse-policy \
  --score 1.0 \
  --threshold 1.0 \
  -- sh -c 'test -s README.md && test ! -e package.json'
```

Observed gate result:

```text
test_passed suite=readme-smoke
eval_passed suite=sparse-policy score=1 threshold=1
```

Review and readiness:

```sh
trail lane review svelte-readme
trail lane readiness svelte-readme
trail lane diff svelte-readme --patch
```

Observed readiness:

```text
Lane readiness: svelte-readme (ready)
Ready: true
Changed paths: 1
Blockers: 0
Warnings: 0
Latest test: test_passed
Latest eval: eval_passed
```

Observed review summary:

```text
Evidence: 1 operation(s), 2 session(s), 10 event(s), 2 gate(s)
Changed paths:
  Modified README.md
```

Merge through the queue:

```sh
trail session end session-svelte-readme --status completed
trail merge-queue add svelte-readme --into main
trail merge-queue run
```

Observed merge result:

```text
Queued refs/lanes/svelte-readme into refs/branches/main
merged as change_f348...
```

This demonstrates the intended large-repo pattern:

```text
large repository
      |
      v
initialize from Git-tracked snapshot
      |
      v
spawn sparse lane for the task paths
      |
      v
record prompts/messages/events when context matters
      |
      v
edit only the sparse workdir
      |
      v
record lane work
      |
      v
run gates, review readiness, merge through queue
```

## Choosing the Right Level of Recording

For a quick manual experiment:

```text
lane + workdir + record + diff
```

For reviewable human work:

```text
lane + workdir + record + test/eval gates + readiness
```

For agent work with durable transcript:

```text
lane + session + turns + messages + events + patches/records + gates + readiness
```

For an automated agent product experience:

```text
editor/agent host + Trail Relay + lane
```

The relay/integration layer should handle session creation, turn creation,
message/event mirroring, patch/workdir recording, gate recording, and readiness
checks automatically.

## Code Facts Used

- Lane CLI surface: `trail/src/cli/command/lane_args.rs`
- Session and approvals CLI: `trail/src/cli/command/collaboration_args`
- Lane models: `trail/src/model/lane`
- Lane storage and lifecycle: `trail/src/db/lane`
- Readiness and handoff: `trail/src/db/lane/readiness.rs`
- Merge queue: `trail/src/db/merge`
