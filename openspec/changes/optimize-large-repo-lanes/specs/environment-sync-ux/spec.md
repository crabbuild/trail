## ADDED Requirements

### Requirement: Environment synchronization uses one command hierarchy
Trail SHALL expose all-component synchronization as `trail env sync all [<lane>]`. Trail SHALL expose targeted synchronization as `trail env sync component <component> [--lane <lane>]`.

#### Scenario: Explicit all-component synchronization
- **WHEN** a user runs `trail env sync all agent-a`
- **THEN** Trail discovers the complete desired graph, prepares stale or missing components, and atomically activates one generation for `agent-a`

#### Scenario: Single component
- **WHEN** a user runs `trail env sync component web.dependencies --lane agent-a`
- **THEN** Trail synchronizes only that component after verifying that all required upstream components are already ready

### Requirement: Lane omission is resolved safely
Trail MAY infer an omitted lane only when the current directory or managed execution context identifies exactly one active lane. Trail MUST reject absent or ambiguous lane context rather than selecting a lane by recency or name ordering.

#### Scenario: Command inside a mounted lane
- **WHEN** a user runs `trail env sync all` from a path contained by exactly one mounted lane
- **THEN** Trail synchronizes that lane and reports how the lane was resolved

#### Scenario: Ambiguous or external directory
- **WHEN** no lane or more than one lane can own the command context
- **THEN** Trail exits with invalid-input status and requires an explicit lane argument

### Requirement: Synchronization is convergent and explainable
Environment synchronization SHALL compare canonical desired component keys with the active generation, skip exact hits, build each missing key at most once concurrently, and return one typed report shared by Rust, CLI JSON, HTTP, and MCP surfaces.

#### Scenario: Complete cache hit
- **WHEN** every desired component and output has a verified compatible layer or private binding
- **THEN** Trail runs no build command and reports every component as reused with its key, layer or storage identity, and reuse source

#### Scenario: Partial cache hit
- **WHEN** some desired components are reusable and others are stale
- **THEN** Trail retains reusable components, rebuilds only stale dependency closures, and atomically activates the composed generation

#### Scenario: Human terminal rendering
- **WHEN** synchronization is rendered for a human terminal
- **THEN** Trail summarizes reused, built, private, stale, and failed components while automation receives the complete typed report through a structured format

### Requirement: The legacy sync-all command is removed by hard cutover
Trail MUST reject `trail env sync-all` as an unknown command and MUST NOT retain a hidden alias, deprecated parser path, or stored compatibility marker.

#### Scenario: Legacy command invocation
- **WHEN** a user invokes `trail env sync-all agent-a`
- **THEN** Trail exits with command-usage status and shows the new `trail env sync all agent-a` spelling
