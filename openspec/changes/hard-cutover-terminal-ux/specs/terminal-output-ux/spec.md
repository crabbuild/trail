## ADDED Requirements

### Requirement: Hard-cutover human rendering contract
Trail SHALL use the new terminal rendering contract for every human-readable command result and diagnostic, and SHALL NOT provide a legacy rendering mode or mix legacy and new layouts in a supported release.

#### Scenario: Human command after cutover
- **WHEN** a user runs any Trail command with human output
- **THEN** the command renders exclusively through the new terminal UX primitives

#### Scenario: Removed compatibility surface
- **WHEN** a user requests the former human style or obsolete `--no-color` option
- **THEN** Trail rejects the obsolete option with an actionable input diagnostic instead of invoking a compatibility renderer

### Requirement: Explicit output modes
Trail SHALL provide distinct `human`, `plain`, `json`, and `ndjson` formats with behavior appropriate to interactive users, logs, single structured reports, and structured streams respectively.

#### Scenario: Interactive human output
- **WHEN** `--format human` writes to a capable terminal
- **THEN** Trail may use responsive layout, semantic color, Unicode glyphs, relative time, transient progress, and eligible paging

#### Scenario: Deterministic plain output
- **WHEN** `--format plain` is selected
- **THEN** Trail emits deterministic ASCII text without ANSI/OSC sequences, relative time, paging, or transient progress

#### Scenario: Structured output
- **WHEN** JSON or NDJSON output is selected
- **THEN** Trail serializes report data directly without terminal decoration or human document headings

#### Scenario: Quiet output
- **WHEN** `--quiet` is selected and the command succeeds
- **THEN** Trail suppresses normal human/plain output while still emitting failures on stderr

### Requirement: Outcome-first information hierarchy
Every non-trivial human result SHALL lead with the user-visible outcome or current state and SHALL order subsequent content as context, primary evidence, attention items, and next action.

#### Scenario: Dirty workspace
- **WHEN** `trail status` detects unrecorded changes
- **THEN** the first line states that the worktree has unrecorded changes before displaying branch, head, or root metadata

#### Scenario: Clean workspace
- **WHEN** `trail status` detects no unrecorded changes
- **THEN** Trail emits a compact clean result without empty change or suggestion sections

#### Scenario: Completed mutation
- **WHEN** a mutating command completes successfully
- **THEN** its lead states the completed action and affected scope before secondary identifiers

### Requirement: Unified semantic status language
Trail SHALL express state with a shared vocabulary covering success, attention, blocked/failure, informational, and skipped states, and SHALL NOT rely on color as the sole distinction.

#### Scenario: Readiness checklist without color
- **WHEN** a readiness view is rendered with color disabled
- **THEN** every check remains distinguishable through textual labels such as `PASS`, `WARN`, `FAIL`, `BLOCKED`, `PENDING`, or `SKIP`

#### Scenario: Semantic color enabled
- **WHEN** human color output is enabled
- **THEN** success, attention, failure, commands, and secondary metadata use the globally defined color roles consistently

### Requirement: Responsive terminal layouts
Trail SHALL adapt collections and metadata to available display width by removing low-priority columns before truncating flexible content and by switching to stacked records when aligned rows no longer fit.

#### Scenario: Wide terminal
- **WHEN** a collection is rendered with sufficient width
- **THEN** Trail displays aligned borderless columns including useful secondary data

#### Scenario: Narrow terminal
- **WHEN** the same collection cannot fit its required columns
- **THEN** Trail renders each item as a compact stacked record without splitting status labels, counts, or identifiers

#### Scenario: Omitted collection items
- **WHEN** output limits omit one or more records
- **THEN** Trail states the omitted count and provides an exact command or flag that reveals all records

### Requirement: Selective borderless tables
Trail SHALL use borderless tables for comparable multi-row collections and SHALL use sentences, key/value groups, change lists, or paragraphs when a table would not improve comparison.

#### Scenario: Timeline collection
- **WHEN** a timeline contains multiple operations
- **THEN** Trail displays consistent operation, kind, branch, time, and message columns subject to responsive priority

#### Scenario: Single object detail
- **WHEN** a command displays one object's metadata
- **THEN** Trail uses a key/value or narrative layout instead of a one-row boxed table

### Requirement: Progressive diff presentation
Trail SHALL present diffs as a comparison lead, compact file-level summary, totals, and optional per-file patches, with path-only and name/status projections available.

#### Scenario: Default diff summary
- **WHEN** a user requests a diff without `--patch`
- **THEN** Trail displays file change markers, paths, additions/deletions, change bars where space permits, and aggregate totals without patch bodies

#### Scenario: Patch requested
- **WHEN** `--patch` is selected
- **THEN** Trail groups unified patch content beneath each affected file and preserves textual diff prefixes regardless of color

#### Scenario: Name-only projection
- **WHEN** `--name-only` is selected
- **THEN** Trail emits only affected paths in the selected human/plain format

#### Scenario: Name-status projection
- **WHEN** `--name-status` is selected
- **THEN** Trail emits stable `A`, `M`, `D`, `R`, or `T` markers with affected paths

#### Scenario: Non-text change
- **WHEN** an affected file has no textual patch
- **THEN** Trail emits an explicit binary, opaque, unavailable, or truncated notice instead of silently omitting detail

### Requirement: User-facing terminology and identifiers
Trail SHALL map domain states and operation kinds to explicit user-facing labels and SHALL hide routine internal identifiers from default views unless they are the result or required selector.

#### Scenario: Domain enum rendering
- **WHEN** a report contains an internal enum such as `ManualRecord` or `DirtyTracked`
- **THEN** Trail renders a documented user-facing phrase and never a Rust debug representation

#### Scenario: Verbose identifiers
- **WHEN** `--verbose` is selected
- **THEN** Trail includes full change IDs, roots, refs, and other secondary technical metadata relevant to the command

#### Scenario: Copyable shortened identifier
- **WHEN** Trail renders an abbreviated identifier as a selector
- **THEN** the displayed abbreviation resolves unambiguously in every command that recommends copying it

### Requirement: Actionable diagnostics
Every human diagnostic for a failed or blocked operation SHALL include a stable code, concise summary, cause or protected consequence when known, and the safest exact recovery action when one can be determined.

#### Scenario: Dirty-worktree failure
- **WHEN** an operation is rejected to protect unrecorded worktree changes
- **THEN** Trail explains what would be at risk and presents inspection or recording as the primary recovery path

#### Scenario: Risky bypass exists
- **WHEN** a force, bypass, or destructive alternative exists
- **THEN** Trail places it after the safe action and explicitly states its consequence

#### Scenario: JSON diagnostic
- **WHEN** the same error is requested as JSON
- **THEN** Trail preserves its stable error code, message, and exit code without terminal-only recovery decoration

### Requirement: One primary next action
Human orientation, readiness, guide, dry-run, conflict, and recovery views SHALL present at most one primary `Next:` command that is safe, copy/paste ready, and valid for the displayed state.

#### Scenario: Blocked lane
- **WHEN** lane readiness is blocked by an unrecorded workdir
- **THEN** the primary next command addresses the unrecorded workdir rather than suggesting merge, force, or an unrelated inspection

#### Scenario: Multiple useful commands
- **WHEN** secondary commands support different user intents
- **THEN** Trail places them after the primary action under a lower-priority `More:` section or reveals them in verbose mode

#### Scenario: No useful action
- **WHEN** a result requires no follow-up and has no meaningful drill-down
- **THEN** Trail omits the `Next:` section

### Requirement: Readiness and decision checklists
Readiness, guardrail, doctor, approval, merge-preflight, and validation views SHALL summarize the decision first and render evidence as ordered checks with blockers before warnings.

#### Scenario: Ready decision
- **WHEN** all required checks pass
- **THEN** the lead states that the item is ready and the checklist identifies the passing evidence

#### Scenario: Blocked decision
- **WHEN** one or more blocking checks fail
- **THEN** the lead states that the item is blocked, blockers appear before warnings, and the primary next action addresses the highest-priority blocker

### Requirement: Terminal capability and color policy
Trail SHALL resolve terminal capabilities once per invocation and SHALL support `--color auto|always|never`, with `auto` honoring stream TTY state, `NO_COLOR`, and `TERM=dumb`.

#### Scenario: Redirected human output
- **WHEN** human stdout is redirected and color policy is `auto`
- **THEN** Trail disables terminal escape styling and automatic paging

#### Scenario: Explicit color override
- **WHEN** color policy is `always` or `never`
- **THEN** Trail applies the explicit policy consistently to all renderer-owned styling

#### Scenario: No-color environment
- **WHEN** `NO_COLOR` is present and color policy is `auto`
- **THEN** Trail emits no ANSI color sequences

### Requirement: Safe paging
Trail SHALL page only eligible long-form human review content and SHALL never automatically page plain, JSON, NDJSON, quiet, or redirected output.

#### Scenario: Long interactive patch
- **WHEN** a patch exceeds the interactive terminal height and pager policy is `auto`
- **THEN** Trail may send the rendered patch through the configured pager

#### Scenario: Pipeline output
- **WHEN** stdout is connected to a pipeline
- **THEN** Trail writes directly to stdout without starting a pager

#### Scenario: Pager closes early
- **WHEN** the pager exits after receiving only part of the output
- **THEN** Trail treats the close as a successful user action and does not print a broken-pipe diagnostic

### Requirement: Composable progress rendering
Trail SHALL restrict transient progress to long-running human commands with an interactive stderr and SHALL clear progress before rendering the final result.

#### Scenario: Interactive long operation
- **WHEN** an eligible operation is running with interactive human stderr
- **THEN** Trail may render one bounded transient progress line containing reliable progress or elapsed time

#### Scenario: Logged operation
- **WHEN** stderr is redirected or output is plain, JSON, NDJSON, or quiet
- **THEN** Trail emits no spinner frames or cursor-control updates

### Requirement: Terminal-content sanitization
Trail SHALL sanitize user-controlled text before applying renderer-owned styling so paths, messages, tool summaries, conflict text, and patches cannot inject terminal control sequences or corrupt layout.

#### Scenario: Message contains escape sequence
- **WHEN** an operation message contains ANSI escape bytes
- **THEN** human/plain output displays the bytes visibly or removes their control effect without executing them

#### Scenario: Table cell contains newline
- **WHEN** a user-controlled table value contains embedded newlines
- **THEN** Trail normalizes or escapes them so the value cannot create forged rows or headings

#### Scenario: Structured value contains controls
- **WHEN** the same value is emitted through JSON or NDJSON
- **THEN** Trail preserves the stored value using the format's normal escaping rules

### Requirement: Correct output streams and broken-pipe behavior
Trail SHALL write successful result documents to stdout, diagnostics and transient progress to stderr, and SHALL terminate cleanly when a downstream consumer closes stdout.

#### Scenario: Successful command
- **WHEN** a command succeeds without warnings requiring stderr
- **THEN** all persistent result content is written to stdout

#### Scenario: Failed command
- **WHEN** a command returns a Trail error
- **THEN** the persistent diagnostic is written to stderr and Trail exits with the error's stable exit code

#### Scenario: Consumer closes pipe
- **WHEN** a downstream consumer closes stdout early
- **THEN** Trail exits without a second user-facing I/O failure

### Requirement: Complete command-family coverage
The hard cutover SHALL cover core workspace, inspection, diff/history, collaboration, maintenance, integration, lane, and agent command families before release.

#### Scenario: Renderer inventory validation
- **WHEN** the cutover build is validated
- **THEN** every human command route maps to a semantic document renderer or an explicitly documented raw-content renderer such as schema or snippet output

#### Scenario: New command renderer
- **WHEN** a new command is added after the cutover
- **THEN** automated checks reject direct command-level terminal printing outside approved low-level writer modules

### Requirement: Rendering verification matrix
Trail SHALL test semantic content separately from layout and SHALL maintain golden coverage for representative terminal capabilities and end-to-end coverage for stream behavior.

#### Scenario: Width matrix
- **WHEN** renderer golden tests run
- **THEN** representative narrow, standard, and wide widths are verified for both human and plain output

#### Scenario: Deterministic fixtures
- **WHEN** snapshots include time, duration, terminal size, or identifiers
- **THEN** those inputs are injected or fixed so output does not depend on the developer environment

#### Scenario: Structured regression check
- **WHEN** the new renderer is introduced
- **THEN** JSON/NDJSON report semantics and stable error/exit codes are regression tested independently of human snapshots

### Requirement: Rendering performance
Human and plain rendering SHALL be linear in the amount of rendered report data, use buffered writes, and avoid repeating database or worktree queries solely for presentation.

#### Scenario: Large collection
- **WHEN** Trail renders a report containing thousands of rows
- **THEN** the renderer performs bounded per-row formatting without one write syscall per field

#### Scenario: Large patch
- **WHEN** Trail renders or pages a large patch
- **THEN** it streams or buffers within an explicit bound and does not duplicate the entire patch multiple times for styling and paging
