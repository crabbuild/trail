# Trail terminal output contract

Trail's human output is intentionally a presentation surface. Automations must
use `--format json` for a single report or `--format ndjson` for record
streams.

## Output matrix

| Mode | TTY | Styling | Paging | Progress | Contract |
| --- | --- | --- | --- | --- | --- |
| `human` | yes | semantic color and Unicode when capable | eligible review documents only | one transient stderr line | interactive presentation |
| `human` | no | no auto color; ASCII-safe layout | never | never | readable redirected log |
| `plain` | any | no ANSI/OSC; ASCII; RFC 3339 timestamps; integer milliseconds | never | never | deterministic text |
| `json` | any | none | never | never | one report value |
| `ndjson` | any | none | never | never | one value per supported record |
| `--quiet` | any | none | never | never | suppresses successful human/plain results only |

`--color auto` honors `NO_COLOR`, `TERM=dumb`, and stdout TTY detection.
`--color always` and `--color never` override that choice. `--pager auto` is
limited to long interactive review documents; `always` still never pages a
redirected, plain, structured, or quiet command.

## Renderer inventory and raw exceptions

Every result emitted by `trail/src/cli/command/render/` is a semantic terminal
document, except for the following intentionally raw payloads:

| Route | Reason | Structured alternative |
| --- | --- | --- |
| `trail lane read` | exact file content must remain pipeable | `--format json` carries content and metadata |
| agent report Markdown mode | Markdown is copy/paste source rather than terminal prose | default human mode renders a document |
| agent PR title/body modes | requested text is an integration payload | default human mode renders a document |
| ACP/agent configuration snippets | snippets must remain directly pasteable | default human mode wraps them in a document |
| Git patch export and OpenAPI schema output | requested payload is an integration or review artifact | JSON reports where the command provides one |
| native agent hook acknowledgements | provider protocol requires an exact JSON acknowledgement | not applicable |
| JSON and NDJSON | stable automation contracts | not applicable |

Transient progress is confined to the shared renderer's stderr progress writer.
Persistent diagnostics use the shared error document on stderr. No command
adapter owns ANSI sequences, terminal-width detection, or direct terminal
writes.

## Breaking migration

This is a hard human-output cutoff. `--no-color` is removed; use
`--color never`. Shell scripts, editor integrations, and tests that consume
Trail output must select `--format json` for one report or `--format ndjson`
for a supported record stream. Do not parse human or plain output.

## Dependency decision

The renderer uses `unicode-width` for display-width measurement,
`terminal_size` for conservative terminal dimensions, `anstyle` for ANSI style
ownership, and `time` for deterministic RFC 3339 timestamps. This keeps the
presentation dependency set platform-neutral and avoids a full TUI runtime or
PTY-only dependency in the CLI binary.

## Release performance baseline

Run `scripts/terminal-output-bench.sh` from a release checkout before shipping.
It measures the median of five `trail --help` startups and the renderer's
10,000-row table and roughly 2 MiB patch cases. The release thresholds are:

| Probe | Threshold |
| --- | --- |
| CLI startup median | 250 ms |
| 10,000-row table render | 750 ms |
| Large patch render | 1.5 s |

## Verification scope

The release gate runs formatter, Trail unit/integration tests, source-output
guard, strict OpenSpec validation, and the release benchmark above on the
shipping host. Windows PTY and non-macOS terminal-emulator confirmation remain
release-environment checks: their capability policy defaults conservatively to
unstyled direct output when TTY detection is uncertain, and CI must run the
same smoke cases with `TERM=dumb` and `NO_COLOR` before a platform release.

### Local verification record (2026-07-12)

`cargo fmt --all -- --check`, focused renderer/error/source-guard tests, the
Trail unit/integration suite, strict OpenSpec validation, and
`scripts/terminal-output-bench.sh` passed on macOS. The release benchmark
reported an 8.4 ms median warm startup, a 9.4 ms 10,000-row render, and an
8.2 ms large-patch render.

`cargo test --workspace` is presently blocked outside this change by the
user-owned `prolly/bindings/uniffi` member: its match at `src/lib.rs:1222`
does not handle `prolly::Error::InvalidVersionedMap(_)`. This terminal UX
change does not alter that member, so the failure is recorded rather than
patched here.

The strict Trail clippy invocation is likewise blocked in the same dependency
tree: `prolly` currently reports existing `large_enum_variant` and
`result_large_err` diagnostics for `TransactionConflict`. Neither failure is
introduced by Trail's terminal renderer.
