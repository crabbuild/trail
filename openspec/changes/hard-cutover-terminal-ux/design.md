## Context

Trail's CLI exposes a large, coherent domain through report types, but human rendering is implemented as command-local `println!` calls. Most views print internal metadata before the user-visible state, collections are not width-aware, raw Rust enum names leak into the interface, color is largely confined to patches, and failures rarely include recovery guidance. Richer agent and lane reports compound the problem because every field receives similar visual weight.

The CLI already has useful boundaries: commands return serializable reports, JSON errors have stable codes and exit statuses, `NO_COLOR` is partially honored, and diff rendering detects redirected stdout. The redesign should therefore replace presentation rather than rewrite Trail's operations. It must work on macOS, Linux, and Windows; remain useful over SSH and in CI logs; handle arbitrary user-controlled paths/messages safely; and avoid turning ordinary commands into a full-screen application.

The intended interaction is a composable terminal UI: fast command output with excellent hierarchy, responsive tables, reviewable diffs, transient progress where useful, and exact next actions. It is not an alternate interactive shell.

## Goals / Non-Goals

**Goals:**

- Give every command an immediately legible outcome and consistent information hierarchy.
- Make the daily flow—orient, inspect, review, validate, recover, and apply—discoverable without requiring Trail's internal model.
- Render attractive wide-terminal output that degrades cleanly to narrow terminals and deterministic logs.
- Centralize presentation policy so command renderers describe semantics rather than spacing, ANSI sequences, or terminal behavior.
- Make warnings, blockers, failures, and recovery actions unambiguous without relying on color.
- Keep JSON/NDJSON, exit codes, and error codes as the stable automation surfaces.
- Complete the cutover across every human renderer in one release, with no legacy-style branch.

**Non-Goals:**

- Building a persistent full-screen dashboard, alternate shell, or mouse-driven interface.
- Changing Trail's storage model, operation semantics, readiness rules, or merge safety rules.
- Preserving byte-for-byte human output compatibility.
- Making human or plain output a machine-parsing API; automation must use JSON or NDJSON.
- Adding terminal hyperlinks, image protocols, themes, or user-defined layout templates in the initial cutover.
- Replacing domain reports with view-specific database queries when existing reports already contain the required facts.

## Decisions

### 1. Render semantic documents instead of printing from command modules

Each command adapter will transform its existing report into a `Document` composed from a small set of semantic blocks:

- `Lead`: outcome text plus `Success`, `Attention`, `Blocked`, `Failure`, `Neutral`, or `Progress` state.
- `Metadata`: low-priority key/value facts.
- `Section`: titled groups with importance and optional empty-state text.
- `Table`: typed headers and rows with per-column alignment, priority, and wrapping policy.
- `ChangeList`: path, change kind, rename source, additions, deletions, and optional line-identity detail.
- `Checklist`: pass/warn/fail/blocked/pending/skip evidence rows.
- `Diagnostic`: stable code, summary, cause, consequence, and recovery actions.
- `Diff`: file summaries and optional unified patch content.
- `NextAction`: exactly one primary safe command and reason, plus optional secondary actions.
- `Notice`: truncation, stale data, skipped detail, or pager fallback.

A shared `Renderer<W: Write>` will render the document. Command-specific modules will not emit ANSI escapes, compute padding, inspect TTY state, or call stdout/stderr directly.

This separates domain facts from presentation, permits unit testing without process-global streams, and makes wide, narrow, human, and plain behavior consistent. A loose collection of formatting helpers was rejected because it would retain imperative layout decisions in every command.

### 2. Resolve one immutable render context at CLI startup

`RenderContext` will contain output format, verbosity, quiet mode, color policy, glyph policy, terminal width/height, stdout/stderr TTY state, pager policy, clock/timezone formatting, and whether transient progress is allowed. Environment and global flags are resolved once before command dispatch.

The new global contract is:

- `--format human`: adaptive terminal output; styling, Unicode glyphs, relative times, progress, and paging are allowed when supported.
- `--format plain`: deterministic line-oriented output with ASCII, no ANSI/OSC sequences, no pager, no transient updates, and absolute machine-independent timestamps.
- `--format json`: one structured JSON value.
- `--format ndjson`: one structured JSON value per record for streaming/list commands.
- `--color auto|always|never`: explicit color policy; `auto` honors TTY state, `NO_COLOR`, and `TERM=dumb`.
- `--pager auto|always|never`: explicit paging policy; `auto` is limited to long human output on an interactive terminal.
- `--verbose`: reveal full IDs, roots, internal refs, secondary evidence, and additional commands.
- `--quiet`: suppress successful human/plain output; errors still render.

The current `--no-color` switch is removed rather than maintained as an alias. This is an intentional hard cutoff.

### 3. Use one outcome-first grammar

Human documents use this order:

1. Lead outcome or current state.
2. One short context line when needed.
3. Primary evidence or changed items.
4. Warnings, blockers, or diagnostics.
5. One primary next action.
6. Secondary metadata/actions only when useful or verbose.

Clean and empty outcomes remain compact. A clean `trail status` should fit in two or three lines; a blocked readiness view should spend space on blockers and recovery, not object IDs. Mutation commands start with the completed or previewed action and its scope.

Metadata-first and label-dump layouts were rejected because users must read several lines before learning whether action is required.

### 4. Define a restrained visual language

State is always written as text and optionally reinforced with color:

| State | Label examples | Color role |
| --- | --- | --- |
| Success | `PASS`, `READY`, `CLEAN`, `RECORDED` | green |
| Attention | `WARN`, `PENDING`, `DIRTY` | yellow |
| Blocked/failure | `BLOCKED`, `FAIL`, `CONFLICT` | red |
| Informational | identifiers, refs, commands | cyan |
| Secondary | roots, timestamps, counts | dim/default |

Bold is reserved for the lead and section titles. Commands are visually distinct from explanations. Human mode may use `·`, `→`, `…`, and a light horizontal rule; plain mode uses ASCII equivalents. Boxed tables and decorative frames are prohibited because they waste width and dominate logs.

### 5. Make layouts responsive, not merely truncated

The renderer will measure display width, including Unicode width, and choose a layout based on the available terminal width:

- At wide widths, tables expose useful secondary columns and change statistics.
- At standard widths, low-priority columns disappear before content is truncated.
- At narrow widths, each record becomes a compact stacked block with the identity or path first.

Paths and user messages are the flexible columns. Status, counts, and short identifiers remain intact. Cells never contain raw newlines; multiline content becomes a paragraph beneath the row or uses escaped separators. Truncation always emits an explicit count and a command that reveals the omitted content.

A fixed 80-column renderer was rejected because it looks sparse on modern terminals and still fails on split panes.

### 6. Treat tables as a collection tool, not a universal layout

Borderless tables are used for timelines, branches, queues, tasks, sessions, approvals, gates, checks, mappings, traces, and other multi-row comparisons. Key/value groups are used for one object's metadata. Change lists are used for paths. Paragraphs and diagnostics are used for explanations.

Column headers appear when they disambiguate three or more rows or when a collection has heterogeneous columns. A one-row result should normally render as a sentence or key/value group. Tables must remain understandable with color disabled.

### 7. Make diffs progressive and review-oriented

Diff output has three layers:

1. A lead naming the comparison.
2. A compact change list with `A/M/D/R/T`, paths, additions/deletions, and proportional change bars.
3. Unified patches only when requested.

`--stat` produces summary/change statistics without patch bodies. New `--name-only` and `--name-status` options provide focused path output. `--patch` adds patch bodies grouped by file without repeating redundant global headings. Stable line identities remain opt-in and appear beneath the relevant file/hunk. Renames use a readable `old -> new` relationship with an ASCII fallback.

Patch color distinguishes additions, deletions, hunk headers, and metadata, but prefixes remain authoritative. Long patches may use the pager. Binary, opaque, missing-patch, and truncated cases receive explicit notices rather than disappearing.

### 8. Translate internal vocabulary at the presentation boundary

All domain enums receive explicit user-facing labels. Renderers will not use debug formatting for operation kinds, change kinds, worktree states, readiness states, or run states. Examples include `ManualRecord` to `record`, `GitImport` to `import from Git`, and `DirtyTracked` to `unrecorded changes`.

The default human view prefers task/lane names, checkpoint aliases, branch names, and resolvable selectors. Full change IDs, root object IDs, raw ref names, and database identities are verbose details. An identifier may be shortened only when the shortened form is accepted unambiguously by every command that presents it as a copyable selector; otherwise the renderer uses a full ID or existing alias.

### 9. Make diagnostics explain recovery

Human errors and blocking reports use the same diagnostic model. Each diagnostic contains:

- a stable code;
- a concise user-facing summary;
- the cause when known;
- the consequence or protected action;
- one safest exact recovery command when available;
- optional alternatives ordered by safety.

Force, bypass, destructive, or approval-sensitive commands are never the primary recovery action and must state their consequence. JSON errors retain their current code/message/exit-code contract. Diagnostics go to stderr; successful result documents go to stdout.

### 10. Present exactly one primary next action

Status, guide, dashboard, readiness, dry-run, conflict, and error views may end with `Next:`. It contains one command and one short reason. Additional useful commands appear under `More:` only when they represent genuinely different intents; verbose mode may show a fuller command set.

The primary action must be safe for the current state, copy/paste ready, and use the selector already visible to the user. Commands requiring confirmation must say so before the user runs them. Views with no meaningful next step omit the section.

### 11. Keep long-running UX transient and composable

Long-running scans, checks, backups, cache operations, and agent launches may display a single transient progress line on interactive stderr. The line reports the operation and, when reliable, completed/total work or elapsed time. It is cleared before the final document renders.

Progress is disabled for plain, JSON, NDJSON, quiet, redirected stderr, and `TERM=dumb`. Trail never emits an unbounded stream of spinner frames into logs. A broken pipe exits cleanly without printing a second error.

### 12. Page only review content and preserve pipeline behavior

Automatic paging is limited to long human-mode content such as patches, transcripts, detailed histories, conflict explanations, and large inspection views. Lists intended for composition do not page by default. Paging is disabled when stdout is redirected, in plain/JSON/NDJSON modes, and in quiet mode.

The pager receives already-rendered content and an explicit color policy. If it cannot be started, Trail writes directly to stdout and, only in verbose mode, emits a non-fatal notice. Trail does not treat a pager's early close as command failure.

### 13. Sanitize untrusted terminal content

Paths, operation messages, actor strings, tool output summaries, conflict text, and patch content may contain terminal control sequences. User-controlled content is sanitized before styling: escape characters and non-printing controls are rendered visibly, embedded newlines are normalized according to block type, and renderer-owned ANSI/OSC sequences are added only after sanitization. Tabs remain meaningful in patch bodies but are width-accounted elsewhere.

This protects terminals and snapshot structure without changing stored values or JSON output.

### 14. Migrate by command archetype and cut over atomically

Implementation proceeds through reusable archetypes:

1. Core infrastructure and diagnostics.
2. Orientation/mutation: init, status, record, checkout, branch.
3. Review: timeline, show, history, why, code-from, diff, transcript.
4. Decisions: guardrails, readiness, doctor, approvals, conflicts, merges, queue.
5. Lane and agent dashboards, actions, evidence, and recovery.
6. Specialist inspection, environment, cache, backup, and integration commands.

All human renderers must migrate before release. There is no runtime legacy renderer and no mixed-style supported state. JSON/NDJSON rendering bypasses the semantic document renderer and continues to serialize reports directly.

### 15. Verify semantics and presentation independently

Tests operate at three levels:

- Semantic document tests assert lead state, evidence, blocker ordering, and next action without depending on spacing.
- Golden renderer tests cover human and plain modes at representative narrow, standard, and wide widths, with and without color/glyphs.
- PTY/end-to-end tests cover TTY detection, redirection, `NO_COLOR`, paging, progress cleanup, errors, broken pipes, and representative command families.

Clocks and terminal dimensions are injected. Snapshots do not depend on the developer's locale, timezone, terminal, or current time. A source-level guard prevents new direct `println!`/`eprintln!` calls in command render modules after cutover.

## Risks / Trade-offs

- **[Risk] The all-at-once cutover creates a large review surface.** → Migrate by archetype behind an internal compile-time module boundary, require complete command inventory coverage, and merge only when the legacy renderer has no call sites.
- **[Risk] Adaptive output produces more snapshot combinations.** → Keep the primitive set small, test representative widths, and assert semantic documents separately from layout snapshots.
- **[Risk] Hiding internal identifiers may make expert diagnosis slower.** → Keep `--verbose`, `show`, and structured formats complete, and never abbreviate a selector that cannot be pasted back into Trail.
- **[Risk] Terminal capability detection differs across platforms and remote shells.** → Default uncertain cases to plain styling, provide explicit color/pager overrides, and test Windows plus PTY and redirected cases.
- **[Risk] Automatic paging can surprise pipelines or hang automation.** → Restrict it to interactive human review content and disable it whenever either output mode or TTY state is ambiguous.
- **[Risk] Sanitizing patch content can make unusual source bytes look different.** → Render escaped bytes visibly, retain raw content in JSON/object access, and test control-character fixtures.
- **[Risk] Additional presentation dependencies increase maintenance and binary size.** → Prefer focused, platform-neutral libraries; benchmark release size/startup; implement small stable primitives internally when dependency cost is disproportionate.
- **[Risk] Report models may lack precise recovery commands or summaries.** → Add optional semantic fields at the report layer only when the command handler cannot derive them safely; keep those additions shared by CLI, HTTP, and MCP where meaningful.

## Migration Plan

1. Inventory every human renderer and capture representative pre-cutover reports as fixtures, not as compatibility snapshots.
2. Add the render context, semantic document model, terminal capability resolver, buffered writer, sanitizer, and test harness.
3. Add the new global format/color/pager contract and remove the old `--no-color` surface.
4. Migrate command families in the archetype order above while JSON/NDJSON continue to bypass the new renderer.
5. Add semantic, golden, and PTY tests; validate Windows and non-TTY behavior; benchmark startup, large tables, and long patches.
6. Delete legacy formatting helpers and direct output calls, then update README examples, reference documentation, shell completions, and changelog with a clear breaking-change notice.
7. Release the new rendering style only after every command is migrated. Rollback is a source/release rollback, not a runtime compatibility flag.

## Open Questions

None. The following boundary decisions are resolved for this change:

- Unique-prefix resolution is not added. Default views use existing resolvable aliases or full IDs; abbreviated decorative IDs are not presented as copyable selectors.
- Pager eligibility is explicit per semantic document. The initial eligible set is patch output, transcripts, detailed history, conflict explanations, and detailed object/text/map inspection. Schema, snippet, path-only, status, queue, and other composition-oriented output never pages automatically.
- Plain timestamps use RFC 3339 UTC and plain durations use integer milliseconds. Human mode may use relative timestamps and compact durations.
