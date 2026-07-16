# Agent ACP and hooks hard-cutover implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Replace Trail's overlapping agent setup commands with explicit ACP and native-hook setup groups, positional providers, and a hard cutover.

**Architecture:** Keep `trail acp relay` as the low-level relay. Move editor-facing setup, diagnostics, sessions, and the hidden task runner below `trail agent acp`. Rename native hook installation to `trail agent hooks setup`. Resolve positional and named providers through one helper, while terminal startup alone may read `agent.default_provider`.

**Tech stack:** Rust, clap, serde, serde_json, TOML workspace configuration, existing Trail ACP and native-hook modules, cargo test.

## Global constraints

- Remove old commands without aliases, migration messages, or compatibility shims
- Prefer positional providers and accept `--provider` as an equivalent named form
- Reject input that supplies both provider forms
- Keep ACP and hook configuration ownership isolated
- Follow red-green-refactor for every behavior change
- Preserve unrelated untracked workspace content

---

### Task 1: New CLI command contract

**Files:**
- Modify: `trail/src/cli/command.rs`
- Modify: `trail/src/cli/command/acp_args.rs`
- Modify: `trail/src/cli/command/agent_args.rs`
- Test: `trail/src/cli/command.rs`

**Interfaces:**
- Produces: `AgentAcpCommand`, `AgentAcpSubcommand`, `AgentAcpSetupArgs`, `AgentAcpRunArgs`
- Produces: `resolve_provider_argument(positional, named, fallback)` in the agent handler
- Removes: `AgentSubcommand::Setup`, `AcpSubcommand::Install`, `AgentHooksSubcommand::Add`

- [x] **Step 1: Write failing parser tests**

```rust
#[test]
fn parses_new_agent_provider_forms() {
    Cli::try_parse_from(["trail", "agent", "start", "codex"]).unwrap();
    Cli::try_parse_from([
        "trail", "agent", "acp", "setup", "codex", "--editor", "zed",
    ])
    .unwrap();
    Cli::try_parse_from(["trail", "agent", "hooks", "setup", "codex"]).unwrap();
}

#[test]
fn rejects_removed_setup_commands() {
    assert!(Cli::try_parse_from(["trail", "agent", "setup"]).is_err());
    assert!(Cli::try_parse_from(["trail", "acp", "install"]).is_err());
    assert!(Cli::try_parse_from(["trail", "agent", "hooks", "add", "codex"]).is_err());
}
```

- [x] **Step 2: Run parser tests and verify RED**

Run: `cargo test -p trail cli::command::tests::parses_new_agent_provider_forms -- --exact`

Expected: FAIL because `agent acp setup` and `hooks setup` do not exist.

- [x] **Step 3: Implement the command enums and argument structs**

```rust
pub(super) enum AgentAcpSubcommand {
    Setup(AgentAcpSetupArgs),
    #[command(hide = true)]
    Run(AgentAcpRunArgs),
    Status(AgentAcpStatusArgs),
    Doctor(AgentAcpDoctorArgs),
    Sessions(AcpSessionsArgs),
}
```

- [x] **Step 4: Run parser tests and verify GREEN**

Run: `cargo test -p trail cli::command::tests -- --nocapture`

Expected: PASS.

### Task 2: Provider defaults and terminal startup

**Files:**
- Modify: `trail/src/model/domain/config.rs`
- Modify: `trail/src/db/util/config/entries.rs`
- Modify: `trail/src/db/util/config/set.rs`
- Modify: `trail/src/cli/command/handler/agent.rs`
- Test: `trail/src/db/util/config.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Produces: `AgentConfig { default_provider: Option<String> }`
- Produces: config key `agent.default_provider`
- Produces: terminal resolution order positional, named, configured default

- [x] **Step 1: Write failing config and terminal tests**

```rust
#[test]
fn agent_default_provider_round_trips() {
    let mut config = test_config();
    set_config_value(&mut config, "agent.default_provider", "codex").unwrap();
    assert_eq!(config.agent.default_provider.as_deref(), Some("codex"));
}
```

- [x] **Step 2: Run focused tests and verify RED**

Run: `cargo test -p trail agent_default_provider_round_trips -- --nocapture`

Expected: FAIL because `TrailConfig` has no agent section.

- [x] **Step 3: Add the configuration section and terminal resolver**

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub default_provider: Option<String>,
}
```

- [x] **Step 4: Run focused tests and verify GREEN**

Run: `cargo test -p trail agent_default_provider -- --nocapture`

Expected: PASS.

### Task 3: ACP setup, status, doctor, sessions, and hidden run

**Files:**
- Create: `trail/src/acp/setup.rs`
- Modify: `trail/src/acp.rs`
- Modify: `trail/src/cli/command/handler/acp.rs`
- Modify: `trail/src/cli/command/handler/agent.rs`
- Modify: `trail/src/cli/command/render/acp.rs`
- Modify: `trail/src/model/lane/activity.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Produces: `build_acp_setup_plan(workspace, db_dir, provider, editor)`
- Produces: `apply_acp_setup_plan(plan, apply)`
- Produces: generated editor command `trail --workspace <path> agent acp run <provider>`

- [x] **Step 1: Write failing ACP setup tests**

```rust
let plan = run_trail_json(
    temp.path(),
    &["agent", "acp", "setup", "codex", "--editor", "zed", "--print"],
);
assert_eq!(plan["transport"], "acp");
assert!(plan["command"].as_array().unwrap().iter().any(|v| v == "run"));
```

- [x] **Step 2: Run ACP setup test and verify RED**

Run: `cargo test -p trail agent_acp_setup_uses_hidden_run -- --nocapture`

Expected: FAIL because the ACP setup group is not handled.

- [x] **Step 3: Implement plan generation, exact Zed merge, and hidden run**

```rust
pub struct AcpSetupReport {
    pub transport: String,
    pub provider: String,
    pub editor: String,
    pub command: Vec<String>,
    pub snippet: String,
    pub applied: bool,
}
```

- [x] **Step 4: Run ACP tests and verify GREEN**

Run: `cargo test -p trail acp -- --nocapture`

Expected: PASS.

### Task 4: Native hooks setup hard cutover

**Files:**
- Modify: `trail/src/cli/command/agent_args.rs`
- Modify: `trail/src/cli/command/handler/agent.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Produces: `trail agent hooks setup <provider> [--provider <provider>]`
- Preserves: existing hook install planning, atomic apply, ownership, doctor, receipts, and removal

- [x] **Step 1: Write failing hooks setup tests**

```rust
let plan = run_trail_json(
    temp.path(),
    &["agent", "hooks", "setup", "codex", "--print"],
);
assert_eq!(plan["provider"], "codex");
assert_eq!(plan["dry_run"], true);
```

- [x] **Step 2: Run hooks setup test and verify RED**

Run: `cargo test -p trail agent_hooks_setup -- --nocapture`

Expected: FAIL because only `hooks add` exists.

- [x] **Step 3: Route setup through existing install planning**

```rust
let apply = args.yes;
let report = apply_agent_hook_install_plan(&plan, !apply)?;
```

- [x] **Step 4: Run native hook tests and verify GREEN**

Run: `cargo test -p trail agent_hooks -- --nocapture`

Expected: PASS.

### Task 5: Hard-cutover regression and documentation cleanup

**Files:**
- Modify: `trail/tests/e2e.rs`
- Modify: `docs/agent/*.md`
- Modify: `docs/integrations/*.md`
- Modify: `docs/reference/cli/integrations-and-maintenance.md`
- Modify: `skills/use-trail/references/*.md`

**Interfaces:**
- Removes all public references to old setup syntax
- Preserves `trail acp relay` as the low-level custom integration surface

- [x] **Step 1: Add help and removed-command assertions**

```rust
assert_command_rejected(&["agent", "setup"]);
assert_command_rejected(&["acp", "install"]);
assert_command_rejected(&["agent", "hooks", "add", "codex"]);
```

- [x] **Step 2: Run regression tests and verify RED where old expectations remain**

Run: `cargo test -p trail --test e2e agent -- --nocapture`

Expected: FAIL at old command expectations.

- [x] **Step 3: Remove old tests and update current documentation**

Replace examples with `agent acp setup`, `agent hooks setup`, positional providers, and hidden `agent acp run` where editor configuration requires it.

- [x] **Step 4: Run full verification**

Run: `cargo fmt --check && cargo test -p trail && cargo test -p trail`

Expected: PASS with zero failures.
