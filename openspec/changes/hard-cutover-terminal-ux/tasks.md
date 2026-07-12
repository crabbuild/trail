## 1. Baseline and Coverage Inventory

- [x] 1.1 Inventory every persistent and transient stdout/stderr write reachable from Trail CLI commands, map each route to a command family, and record the intentionally raw schema/snippet/content exceptions.
- [x] 1.2 Capture representative typed report fixtures for clean, dirty, empty, success, warning, blocked, conflict, and failure states without treating existing human text as a compatibility baseline.
- [x] 1.3 Define the command-family output matrix covering human, plain, JSON, NDJSON, quiet, verbose, TTY, redirected, narrow, and wide behavior.
- [x] 1.4 Evaluate focused terminal-width, Unicode-width, wrapping, styling, paging, and PTY test dependencies against Rust/platform support, binary size, startup time, and maintenance cost; document the selected minimal set.
- [x] 1.5 Add baseline measurements for CLI startup, a 10,000-row collection render, and a large patch render so the cutover has explicit performance regression thresholds.

## 2. Semantic Rendering Core

- [x] 2.1 Create the new terminal UI module structure and a buffered `Renderer<W: Write>` entry point that returns I/O errors rather than printing directly.
- [x] 2.2 Implement the semantic `Document`, `Lead`, `Section`, `Metadata`, `Notice`, and `NextAction` models with importance and visibility metadata.
- [x] 2.3 Implement typed `Table`, `ChangeList`, `Checklist`, `Diagnostic`, and `Diff` blocks without embedding terminal styling in their values.
- [x] 2.4 Implement explicit user-facing label mappings for operation kinds, file changes, worktree states, lane/task states, readiness, approvals, gates, runs, and merge states; prohibit debug enum formatting in render adapters.
- [x] 2.5 Add human-mode style roles for lead, success, attention, blocked/failure, identifiers, commands, headings, and secondary metadata with text labels that remain authoritative without color.
- [x] 2.6 Add human Unicode glyphs and plain ASCII fallbacks for separators, arrows, ellipses, rules, and change bars.
- [x] 2.7 Implement user-content sanitization for cells, paragraphs, paths, messages, diagnostics, conflict text, and patch lines before renderer-owned styling is applied.
- [x] 2.8 Implement display-width measurement, wrapping, indentation, and truncation that account for Unicode width and renderer-owned style sequences.
- [x] 2.9 Implement responsive borderless tables with column priority, alignment, minimum/flexible width, wide/standard layouts, and narrow stacked fallback.
- [x] 2.10 Implement outcome-first document ordering, compact empty/clean states, explicit omitted-item notices, and single-primary-action enforcement.
- [x] 2.11 Implement change-list totals, `A/M/D/R/T` markers, rename presentation, addition/deletion counts, and proportional bars for human and plain modes.
- [x] 2.12 Implement checklist ordering that places blockers before warnings and renders stable textual state labels.
- [x] 2.13 Implement human and deterministic plain render backends, including RFC 3339 UTC timestamps and integer-millisecond durations in plain mode.
- [x] 2.14 Add semantic-document and renderer golden tests at representative narrow, standard, and wide widths with color/glyph policies enabled and disabled.

## 3. CLI Render Context and Stream Behavior

- [x] 3.1 Replace global output parsing with explicit `human`, `plain`, `json`, and `ndjson` formats and add parsing/error tests for each format.
- [x] 3.2 Replace `--no-color` with `--color auto|always|never`, honor `NO_COLOR` and `TERM=dumb` in `auto`, and add resolution tests.
- [x] 3.3 Add `--pager auto|always|never` and resolve pager eligibility from output format, command document policy, terminal state, and output size.
- [x] 3.4 Expand `RenderContext` to include verbosity, quiet mode, color/glyph policy, terminal width/height, stdout/stderr TTY state, pager policy, clock, and progress eligibility.
- [x] 3.5 Route persistent successful documents to stdout and diagnostics/progress to stderr through injected buffered writers.
- [x] 3.6 Implement clean broken-pipe handling for direct and paged output so an early downstream close does not produce a second diagnostic.
- [x] 3.7 Implement explicit pager integration for patches, transcripts, detailed history, conflict explanations, and detailed object/text/map views, including direct-output fallback and early-close handling.
- [x] 3.8 Implement one bounded transient progress line for eligible long-running commands and guarantee cleanup before final output.
- [x] 3.9 Implement NDJSON record emission for streaming/list commands and reject or document NDJSON use on commands that cannot produce a record stream.
- [x] 3.10 Add PTY and redirected-stream tests for color auto-detection, `TERM=dumb`, `NO_COLOR`, paging, progress suppression/cleanup, quiet mode, and broken pipes.

## 4. Actionable Diagnostics

- [x] 4.1 Add a diagnostic presentation mapping for every stable `trail::Error` code with concise summary, cause/consequence text, and a safe recovery action when determinable.
- [x] 4.2 Convert Clap parse failures to the shared human diagnostic style while preserving JSON error shape and exit status.
- [x] 4.3 Convert daemon, watchdog, sandbox, provider-receipt, and command-execution failures that reach users to the shared stream and diagnostic policy.
- [x] 4.4 Add reusable blocker/warning adapters so readiness, guardrails, doctor, conflicts, and agent diagnosis share diagnostic ordering and recovery presentation.
- [x] 4.5 Ensure force, bypass, destructive, or confirmation-sensitive alternatives are never primary and always describe their consequence.
- [x] 4.6 Add golden and semantic tests for every stable error category, unknown-cause fallback, multiple blockers, risky alternatives, JSON preservation, and stderr routing.

## 5. Core Workspace and Configuration Cutover

- [x] 5.1 Migrate init output to a completed-action lead, concise import summary, default branch, and verbose-only workspace/operation details.
- [x] 5.2 Migrate clean, tracked-dirty, and untracked-dirty status output to outcome-first change lists with one state-appropriate next action.
- [x] 5.3 Migrate record and watch output, including no-change, partial path, repeated record, and long-running watch/progress states.
- [x] 5.4 Migrate checkout output for dry-run, alternate output root, dirty-worktree recording, successful materialization, and protected failure states.
- [x] 5.5 Migrate config and ignore list/get/set/add/remove/check outputs using compact outcomes, key/value detail, and responsive collections.
- [x] 5.6 Migrate guardrail checks to decision-first checklists with approval evidence, ignored/denied path detail, blockers, and the safest next action.
- [x] 5.7 Add semantic and width-matrix golden tests for each migrated workspace/configuration state.

## 6. History, Inspection, and Diff Cutover

- [x] 6.1 Migrate timeline to a responsive borderless operation table with explicit user-facing kinds, deterministic plain timestamps, and narrow stacked records.
- [x] 6.2 Migrate show output for operation, message, ref, lane, and object variants using narrative/key-value layouts and verbose technical identities.
- [x] 6.3 Migrate history, code-from, and why output to progressive summaries, readable provenance, path/line relationships, and explicit empty states.
- [x] 6.4 Migrate object, root, text, map-range, and map-diff inspection with eligible paging, safe truncation notices, and explicit commands to reveal omitted content.
- [x] 6.5 Refactor diff rendering into semantic file summaries and per-file patch blocks while preserving patch prefixes and line-identity opt-in behavior.
- [x] 6.6 Add and test `--name-only` and `--name-status`; make `--stat` suppress patch bodies and make conflicting projection flags actionable input errors.
- [x] 6.7 Add explicit binary, opaque, missing-patch, and truncated diff notices plus sanitized control-character fixtures.
- [x] 6.8 Migrate ACP profile/install/doctor/session and transcript output, using tables for collections and paging only for long transcripts.
- [x] 6.9 Add semantic, golden width, color, plain, pager, and structured-format regression tests for the migrated review/inspection commands.

## 7. Collaboration, Merge, and Maintenance Cutover

- [x] 7.1 Migrate branch create/list/delete/rename and Git import/export/mapping output using concise outcomes and responsive collections.
- [x] 7.2 Migrate anchor and lease create/resolve/list/release/claim outputs with safe coordination status, ownership, expiry, and conflict guidance.
- [x] 7.3 Migrate merge dry-run/result output to decision-first summaries with changed paths, conflicts, and exact safe follow-up.
- [x] 7.4 Migrate merge queue add/list/run/explain/remove output to responsive queue tables, ordered blockers, pause reasons, and one primary action.
- [x] 7.5 Migrate conflict list/detail/explanation/resolve output to progressive path/line evidence, recommendations, safe recovery actions, and eligible paging.
- [x] 7.6 Migrate doctor and FSCK output to ordered checklists with compact healthy states and actionable failed checks.
- [x] 7.7 Migrate backup create/verify/restore, index rebuild, worktree index, and GC output with dry-run distinction, progress, summaries, and verbose integrity metadata.
- [x] 7.8 Add semantic and golden tests for success, empty, warning, blocked, conflict, dry-run, and destructive-confirmation states across collaboration and maintenance commands.

## 8. Lane Workflow Cutover

- [x] 8.1 Migrate lane spawn/list/detail/status outputs with user-facing lane state, base freshness, changed paths, workdir state, gates, and queued merges.
- [x] 8.2 Migrate lane contribution, review packet, gate history, readiness, refresh preview, and handoff to decision/checklist layouts with blockers before warnings.
- [x] 8.3 Migrate lane record, preview, rewind, watch, test/eval, workdir, file-read, sync, patch, remove, workspace checkpoint/space/exec/mount outputs.
- [x] 8.4 Migrate session start/current/list/detail/context/end output with responsive session tables and concise lifecycle outcomes.
- [x] 8.5 Migrate turn/message/event/start/detail/end output with progressive evidence and readable checkpoint selectors.
- [x] 8.6 Migrate run pause/resume/list/state output with explicit paused reason, approval dependency, and safe resume guidance.
- [x] 8.7 Migrate trace span start/end/list/summary/detail output with aligned timing/count data and deterministic plain durations.
- [x] 8.8 Migrate approval request/list/detail/decision output to checklist/state language with reviewer evidence and confirmation-sensitive actions.
- [x] 8.9 Migrate dependency environment, adapter environment, workspace cache, and related specialist lane reports discovered by the renderer inventory.
- [x] 8.10 Add semantic and width-matrix golden tests covering clean, dirty, stale, ready, blocked, pending approval, failed gate, paused, conflicted, and empty lane states.

## 9. Agent Task UX Cutover

- [x] 9.1 Split the monolithic agent renderer into focused home, review, evidence, validation, lifecycle, and sharing adapters while keeping shared task titles, selectors, risk, and action helpers centralized.
- [x] 9.2 Migrate setup, empty-task guidance, status, guide, inbox, board, stack, list, and next outputs to low-noise orientation layouts.
- [x] 9.3 Migrate dashboard, brief, summary, view, workdir, and review-flow outputs so each emphasizes one current state, one focus item, and one primary action.
- [x] 9.4 Migrate review-data, action palette, review plan, focus, review map, risk, confidence, and ready/can-land output to typed actions and ordered checklists.
- [x] 9.5 Migrate changes, delta, timeline, change-set, files, file, checkpoints, why, turn, diff, review, story, tools, and impact output to progressive review-oriented blocks.
- [x] 9.6 Migrate validation, test plan, diagnosis, compare, and multi-task stack output with consistent evidence, priority, overlap, and blocker presentation.
- [x] 9.7 Migrate new/start/run, mark-reviewed, archive/unarchive, apply/land, finish/ship, continue, undo/recovery, and related lifecycle mutations to completed/preview outcomes and confirmation-aware next actions.
- [x] 9.8 Migrate report, receipt, handoff, and PR output while preserving intentionally raw Markdown/snippet modes as documented exceptions.
- [x] 9.9 Add semantic and width-matrix golden tests covering no tasks, running, needs record, needs review, validation missing/failed/passed, blocked, conflicted, ready, applied, archived, overlap, and recovery states.

## 10. Hard Cutoff and Documentation

- [x] 10.1 Update every command handler to pass typed reports through the new render context/document adapters and bypass them only for JSON, NDJSON, and approved raw-content modes.
- [x] 10.2 Delete legacy human formatting helpers, direct ANSI patch styling, obsolete boolean `json/quiet/color` renderer signatures, and the removed `--no-color` argument.
- [x] 10.3 Add an automated source guard that rejects new direct `println!`, `eprintln!`, `print!`, and raw ANSI use in command render/handler modules outside approved writer/progress modules.
- [x] 10.4 Verify the renderer inventory has no unmigrated command route and no mixed legacy/new human output path.
- [x] 10.5 Update CLI help, generated completions, README walkthroughs, CLI reference pages, agent/lane examples, and screenshots/text fixtures to the new output contract.
- [x] 10.6 Add a changelog and migration note declaring the breaking human-output cutoff, removed `--no-color`, new format/color/pager flags, and requirement that automation use JSON or NDJSON.
- [x] 10.7 Run formatting, linting, unit, integration, PTY, platform-relevant, OpenSpec validation, and full workspace tests; record any intentionally deferred platform verification.
- [x] 10.8 Re-run startup/large-table/large-patch benchmarks and confirm rendering remains within the agreed regression thresholds before release.
